#!/usr/bin/env python3
"""Preserve notices and exact Homebrew sources for the bundled Mach-O closure."""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import zipfile

from windows_sources import digest, download, json_bytes, rust_sources

ROOT = Path(__file__).resolve().parents[2]
SOURCE_FILES = {
    "SOURCE-INDEX.md", "SOURCE.json", "cuepool-source.tar", "Cargo.lock",
    "RUST-SOURCES.json", "ffmpeg-source.tar.xz", "FFMPEG-BUILD.txt",
    "LICENSE-MIT", "LICENSE-APACHE", "HOMEBREW-LICENSE.txt",
}


def command(*args):
    return subprocess.check_output(args, text=True, env={
        **os.environ, "HOMEBREW_NO_AUTO_UPDATE": "1", "HOMEBREW_NO_ANALYTICS": "1",
    }).strip()


def macho(path):
    deps = [line.strip().split(" (compatibility version", 1)[0]
            for line in command("otool", "-L", str(path)).splitlines()[1:]]
    loads = command("otool", "-l", str(path))
    rpaths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", loads)
    uuids = re.findall(r"cmd LC_UUID\s+cmdsize \d+\s+uuid ([A-Fa-f0-9-]+)", loads)
    if not uuids:
        raise ValueError(f"Mach-O has no build UUID: {path}")
    return deps, rpaths, sorted(uuids)


def resolve_dependency(name, owner, executable, rpaths):
    if name.startswith(("/usr/lib/", "/System/Library/")):
        return None
    def expand(value):
        return value.replace("@loader_path", str(owner.parent)).replace(
            "@executable_path", str(executable.parent))
    candidates = ([Path(expand(rp)) / name.removeprefix("@rpath/") for rp in rpaths]
                  if name.startswith("@rpath/") else [Path(expand(name))])
    for path in candidates:
        if path.is_absolute() and path.is_file():
            return path.resolve()
    raise ValueError(f"Unresolved non-system library {name} from {owner}")


def library_closure(binary):
    binary = binary.resolve()
    visited, libraries = set(), {}
    def visit(path, inherited=()):
        if path in visited:
            return
        visited.add(path)
        deps, rpaths, uuids = macho(path)
        expanded = [rp.replace("@loader_path", str(path.parent)).replace(
            "@executable_path", str(binary.parent)) for rp in rpaths] + list(inherited)
        if path != binary:
            if path.name in libraries and libraries[path.name][0] != path:
                raise ValueError(f"Conflicting bundled library basename: {path.name}")
            libraries[path.name] = (path, uuids)
        for name in deps:
            dependency = resolve_dependency(name, path, binary, expanded)
            if dependency is not None:
                visit(dependency, expanded)
    visit(binary)
    if not libraries:
        raise ValueError("CuePool has no non-system library dependencies")
    return libraries


def verify_bundle(libraries, directory):
    shipped = {p.name: p for p in directory.iterdir() if p.is_file()}
    matched, originals = {}, set()
    for name, bundled in shipped.items():
        # dylibbundler versions can retain either a load-name alias or its
        # real basename. Resolve aliases only inside the original keg directory.
        candidates = [(path, uuids) for path, uuids in libraries.values()
                      if name == path.name or (path.parent / name).resolve() == path]
        if len(candidates) != 1 or candidates[0][0] in originals:
            raise ValueError(f"Bundled library closure differs: unexpected or duplicate {name}")
        original, uuids = candidates[0]
        if macho(bundled)[2] != uuids:
            raise ValueError(f"Bundled library build UUID differs from installed source keg: {name}")
        matched[name] = (original, uuids)
        originals.add(original)
    if originals != {path for path, _ in libraries.values()}:
        raise ValueError("Bundled library closure differs: missing library")
    return matched


def source_spec(formula):
    spec = formula["urls"]["stable"]
    result = {"url": spec["url"]}
    if re.fullmatch(r"[0-9a-f]{64}", spec.get("checksum") or ""):
        result["sha256"] = spec["checksum"]
    elif re.fullmatch(r"[0-9a-f]{40}", spec.get("revision") or ""):
        result["git_commit"] = spec["revision"]
    else:
        raise ValueError(f"Source has no exact checksum or Git commit: {result['url']}")
    return result


def keg_files(keg):
    name, version = keg.parent.name, keg.name
    receipt = keg / "INSTALL_RECEIPT.json"
    recipe = keg / ".brew" / f"{name}.rb"
    installed = json.loads(receipt.read_text())
    if installed["source"]["spec"] != "stable":
        raise ValueError(f"Only stable Homebrew source receipts are supported: {keg}")
    # A full installed recipe path loads that recipe, not today's API formula.
    formula = json.loads(command("brew", "info", "--json=v2", "--formula", str(recipe)))["formulae"][0]
    if formula["versions"]["stable"] != installed["source"]["versions"]["stable"]:
        raise ValueError(f"Installed formula/receipt version mismatch: {keg}")
    pkg_version = formula["versions"]["stable"]
    if formula.get("revision", 0):
        pkg_version += f"_{formula['revision']}"
    if version != pkg_version:
        raise ValueError(f"Installed formula/keg revision mismatch: {keg}")
    prefix = f"homebrew/{name}/{version}"
    files = {f"{prefix}/recipe.rb": recipe.read_bytes(),
             f"{prefix}/INSTALL_RECEIPT.json": receipt.read_bytes(),
             f"{prefix}/formula.json": json_bytes(formula)}
    notices = []
    for path in sorted(keg.iterdir()):
        if path.is_file() and re.match(r"(COPYING|LICENSE|LICENCE|NOTICE|COPYRIGHT|AUTHORS)([._-]|$)", path.name, re.I):
            key = f"{prefix}/{path.name}"
            files[key] = path.read_bytes()
            notices.append(key)
    if not any(not Path(n).name.upper().startswith("AUTHORS") for n in notices):
        raise ValueError(f"No installed dependency notices: {keg}")
    return {"name": name, "version": version, "license": formula["license"],
            "source": source_spec(formula), "recipe": f"{prefix}/recipe.rb",
            "receipt": f"{prefix}/INSTALL_RECEIPT.json", "metadata": f"{prefix}/formula.json",
            "notices": notices}, files


def write_archive(path, files):
    if SOURCE_FILES - files.keys():
        raise ValueError("Missing macOS source files")
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            archive.writestr(name, data)
        archive.writestr("SHA256SUMS", "".join(f"{digest(data)}  {name}\n" for name, data in sorted(files.items())))


def validate_archive(path, sha=None):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or not (SOURCE_FILES | {"SHA256SUMS"}) <= set(names):
            raise ValueError("macOS source ZIP has missing or duplicate entries")
        if any(PurePosixPath(n).is_absolute() or ".." in PurePosixPath(n).parts for n in names):
            raise ValueError("Unsafe macOS source ZIP path")
        files = {n: archive.read(n) for n in names if n != "SHA256SUMS"}
        expected = "".join(f"{digest(data)}  {name}\n" for name, data in sorted(files.items()))
        if archive.read("SHA256SUMS").decode() != expected or not all(files.values()):
            raise ValueError("macOS source ZIP digest mismatch or empty source")
        manifest = json.loads(files["SOURCE.json"])
        if manifest["platform"] != "macos" or (sha and manifest["cuepool_commit"] != sha):
            raise ValueError("macOS source ZIP identifies a different CuePool commit/platform")
        dependencies = {entry["name"]: entry for entry in manifest["homebrew"]}
        if not manifest["libraries"] or "ffmpeg" not in dependencies:
            raise ValueError("macOS source ZIP lacks its FFmpeg library closure")
        for library in manifest["libraries"]:
            if library["formula"] not in dependencies or not library["uuids"]:
                raise ValueError("macOS library lacks corresponding source identity")
        for entry in dependencies.values():
            for name in [entry["recipe"], entry["receipt"], entry["metadata"], *entry["notices"]]:
                if name not in files:
                    raise ValueError(f"macOS dependency source/notice missing: {name}")
            if not entry["notices"]:
                raise ValueError("macOS dependency has no notices")
            if source_spec(json.loads(files[entry["metadata"]])) != entry["source"]:
                raise ValueError("macOS dependency source differs from its formula")
        if digest(files["ffmpeg-source.tar.xz"]) != dependencies["ffmpeg"]["source"]["sha256"]:
            raise ValueError("macOS FFmpeg source archive digest mismatch")


def build(binary, resources, output):
    libraries = verify_bundle(library_closure(binary), resources.parent / "libs")
    cellar = Path(command("brew", "--cellar")).resolve()
    commit = command("git", "-C", str(ROOT), "rev-parse", "HEAD")
    files = {"SOURCE-INDEX.md": (ROOT / "packaging/macos-sources.md").read_bytes(),
             "cuepool-source.tar": subprocess.check_output(["git", "archive", "HEAD"], cwd=ROOT),
             "Cargo.lock": (ROOT / "Cargo.lock").read_bytes()}
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        files[name] = (ROOT / name).read_bytes()
    files["HOMEBREW-LICENSE.txt"] = (Path(command("brew", "--repository")) / "LICENSE.txt").read_bytes()
    files["RUST-SOURCES.json"] = json_bytes(rust_sources(files["Cargo.lock"]))
    kegs, inventory = {}, []
    for name, (path, uuids) in sorted(libraries.items()):
        relative = path.relative_to(cellar)  # Fail closed for non-Homebrew libraries.
        keg = cellar / relative.parts[0] / relative.parts[1]
        if keg not in kegs:
            entry, additions = keg_files(keg)
            kegs[keg] = entry
            files.update(additions)
        inventory.append({"name": name, "formula": kegs[keg]["name"],
                          "version": keg.name, "uuids": uuids, "original_sha256": digest(path.read_bytes())})
    ffmpeg_keg = next(k for k, entry in kegs.items() if entry["name"] == "ffmpeg")
    ffmpeg = kegs[ffmpeg_keg]
    files["ffmpeg-source.tar.xz"] = download(ffmpeg["source"])
    codec = next(path for name, (path, _) in libraries.items() if name.startswith("libavcodec."))
    strings = re.findall(rb"[ -~]{12,}", codec.read_bytes())
    config = next(s for s in strings if b"--enable-gpl" in s and b"--enable-version3" in s)
    version = next(s for s in strings if s.startswith(b"FFmpeg version "))
    files["FFMPEG-BUILD.txt"] = version + b"\n" + config + b"\n"
    files["SOURCE.json"] = json_bytes({"platform": "macos", "cuepool_commit": commit,
                                       "libraries": inventory, "homebrew": list(kegs.values())})
    write_archive(output, files)
    validate_archive(output, commit)
    # Large sources live beside the DMG; notices, recipes and exact source
    # directions travel inside the app, before its final code signature.
    for name, data in files.items():
        if name.startswith("homebrew/") or name in {
            "SOURCE-INDEX.md", "SOURCE.json", "FFMPEG-BUILD.txt", "LICENSE-MIT", "LICENSE-APACHE", "HOMEBREW-LICENSE.txt",
        }:
            path = resources / "ThirdParty" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    print(f"macOS sources: {output} ({len(libraries)} dylibs, {len(kegs)} kegs; CuePool {commit})")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("out/cuepool-macos-sources.zip"))
    args = parser.parse_args()
    build(args.binary, args.resources, args.output)
