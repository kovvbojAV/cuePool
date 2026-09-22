#!/usr/bin/env python3
"""Publish exact Windows source snapshots and indexes without rebuilding FFmpeg."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE_FILES = (
    "SOURCE-INDEX.md", "SOURCE.json", "windows-dependencies.json", "cuepool-source.tar",
    "Cargo.lock", "RUST-SOURCES.json", "ffmpeg-source.tar.gz", "ffmpeg-build-scripts.tar.gz",
    "FFMPEG-DEPENDENCIES.json", "ffmpeg-rav1e-Cargo.lock", "FFMPEG-RAV1E-SOURCES.json",
    "asio-source.zip", "pthreads-vcpkg-port.zip", "FFMPEG-BUILD.txt",
)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def download(spec):
    with urllib.request.urlopen(spec["url"], timeout=60) as response:
        data = response.read()
    if digest(data) != spec["sha256"]:
        raise ValueError(f"Source archive digest mismatch: {spec['url']}")
    return data


def rust_sources(lock):
    sources = []
    for package in tomllib.loads(lock.decode())["package"]:
        source = package.get("source", "")
        if source.startswith("registry+"):
            name, version = package["name"], package["version"]
            sources.append({"name": name, "version": version,
                            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                            "sha256": package["checksum"]})
        elif source.startswith("git+"):
            repo, commit = source.removeprefix("git+").split("#")
            sources.append({"name": package["name"], "repository": repo.split("?")[0],
                            "commit": commit})
        elif source:
            raise ValueError(f"Unsupported Rust source: {source}")
    return sources


def ffmpeg_sources(archive):
    """Index fixed source refs; preserve the recipes for patches/submodule instructions."""
    entries = []
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as source:
        for member in source:
            if not member.isfile() or "/scripts.d/" not in member.name:
                continue
            text = source.extractfile(member).read().decode()
            values = dict(re.findall(r'^SCRIPT_(REPO\d*|COMMIT\d*|REV)="([^"]+)"', text, re.M))
            for key, repo in values.items():
                if key.startswith("REPO"):
                    revision = values.get("COMMIT" + key[4:], values.get("REV"))
                    if not revision:
                        raise ValueError(f"No fixed source revision in {member.name}")
                    entries.append({"recipe": member.name.split("/", 1)[1],
                                    "repository": repo, "revision": revision})
    if not entries:
        raise ValueError("FFmpeg build snapshot has no dependency recipes")
    return entries


def json_bytes(value):
    return (json.dumps(value, indent=2) + "\n").encode()


def write_archive(path, files):
    missing = set(SOURCE_FILES) - files.keys()
    if missing:
        raise ValueError(f"Missing Windows sources: {sorted(missing)}")
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            archive.writestr(name, data)
        archive.writestr("SHA256SUMS", "".join(f"{digest(data)}  {name}\n" for name, data in sorted(files.items())))


def validate_archive(path, sha=None):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or set(names) != set(SOURCE_FILES) | {"SHA256SUMS"}:
            raise ValueError("Windows source ZIP has missing or unexpected entries")
        expected = "".join(f"{digest(archive.read(name))}  {name}\n" for name in sorted(SOURCE_FILES))
        if archive.read("SHA256SUMS").decode() != expected or any(not archive.read(n) for n in SOURCE_FILES):
            raise ValueError("Windows source ZIP digest mismatch or empty source")
        if sha and json.loads(archive.read("SOURCE.json"))["cuepool_commit"] != sha:
            raise ValueError("Windows source ZIP identifies a different CuePool commit")


def build(path):
    specs = json.loads((ROOT / "packaging/windows-dependencies.json").read_text())
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    files = {
        "SOURCE-INDEX.md": (ROOT / "packaging/windows-sources.md").read_bytes(),
        "windows-dependencies.json": json_bytes(specs),
        "cuepool-source.tar": subprocess.check_output(["git", "archive", "HEAD"], cwd=ROOT),
        "Cargo.lock": (ROOT / "Cargo.lock").read_bytes(),
        "ffmpeg-source.tar.gz": download(specs["ffmpeg"]["source"]),
        "ffmpeg-build-scripts.tar.gz": download(specs["ffmpeg"]["build_scripts"]),
        "asio-source.zip": download(specs["asio"]),
        "FFMPEG-BUILD.txt": subprocess.check_output(
            [str(Path(os.environ["FFMPEG_DIR"]) / "bin/ffmpeg.exe"), "-version"],
            stderr=subprocess.STDOUT),
    }
    files["RUST-SOURCES.json"] = json_bytes(rust_sources(files["Cargo.lock"]))
    files["FFMPEG-DEPENDENCIES.json"] = json_bytes(ffmpeg_sources(files["ffmpeg-build-scripts.tar.gz"]))
    with urllib.request.urlopen(specs["ffmpeg"]["rav1e_cargo_lock"], timeout=60) as response:
        files["ffmpeg-rav1e-Cargo.lock"] = response.read()
    files["FFMPEG-RAV1E-SOURCES.json"] = json_bytes(rust_sources(files["ffmpeg-rav1e-Cargo.lock"]))

    # vcpkg supplies a static native dependency outside Cargo.lock. Preserve the
    # exact port used on this runner, including all patches and source hash.
    vcpkg = Path(os.environ["VCPKG_INSTALLATION_ROOT"])
    port = vcpkg / "ports/pthreads"
    recipe = (port / "portfile.cmake").read_text()
    version = json.loads((port / "vcpkg.json").read_text())["version"]
    source_file = re.search(r'FILENAME "([^"]+)"', recipe)[1].replace("${VERSION}", version)
    pthreads = {"version": version, "url": "https://downloads.sourceforge.net/project/pthreads4w/" + source_file,
                "sha512": re.search(r"SHA512 ([0-9a-f]{128})", recipe)[1],
                "vcpkg_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=vcpkg, text=True).strip()}
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for item in sorted(port.rglob("*")):
            if item.is_file():
                archive.write(item, item.relative_to(port))
        archive.write(vcpkg / "installed/x64-windows-static-md/share/pthreads/copyright", "COPYRIGHT")
        archive.writestr("SOURCE.json", json_bytes(pthreads))
    files["pthreads-vcpkg-port.zip"] = buffer.getvalue()
    files["SOURCE.json"] = json_bytes({"cuepool_commit": commit, "pthreads": pthreads})
    write_archive(path, files)
    validate_archive(path, commit)
    print(f"Windows sources: {path} ({path.stat().st_size} bytes; CuePool {commit})")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("out/cuepool-windows-sources.zip"))
    build(parser.parse_args().output)
