#!/usr/bin/env python3
"""Prepare CuePool releases using release-plz and an explicit local Git baseline.

No registry baseline or publication command is used. The pinned tool's built-in
Git-only reconstruction runs cargo package on historical tags, which rejects our
private path dependencies and fork-only wgpu features. Its supported local
baseline option preserves those manifests and does not compile historical code.
"""
import argparse
import contextlib
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".github/scripts"))
from release import validate_workspace, product_version, tag_name, release_eligible

REPOSITORY = "https://github.com/kovvbojAV/cuePool"
TOOL = os.environ.get("RELEASE_PLZ", "release-plz")
TOOL_VERSION = "0.3.169"
PRODUCT_FILES = {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}
PACKAGING_FILES = {
    "package-windows.ps1", "package-macos.sh", ".github/scripts/make-msi.ps1",
    ".github/scripts/prepare-winget.ps1", ".github/scripts/windows_sources.py",
    ".github/scripts/macos_sources.py",
}
PACKAGING_SUPPORT_FILES = {
    "AGENTS.md", "CHANGELOG.md", "release-plz.toml",
    "scripts/prepare-release.py", "scripts/test-release-policy.py",
}


def local_tool_env():
    return {k: v for k, v in os.environ.items() if k not in ("GIT_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")}


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()



@contextlib.contextmanager
def baseline(root, tag):
    tag_name(tag)
    subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", tag, "HEAD"], check=True)
    with tempfile.TemporaryDirectory(prefix="cuepool-release-baseline-") as tmp:
        path = Path(tmp)
        subprocess.run(["git", "clone", "--quiet", "--shared", str(root), str(path)], check=True)
        subprocess.run(["git", "-C", str(path), "checkout", "--quiet", "--detach", tag], check=True)
        if "v" + product_version(path) != tag:
            raise ValueError("Git baseline tag and workspace version disagree")
        # release-plz's comparison expects the original manifest beside the
        # analysis manifest. Cargo metadata resolves workspace inheritance from
        # this complete checkout. Legacy tags mark the harness publish=false:
        # expose it to analysis so its pre-tag history is not treated as new.
        # Preserve the byte-for-byte historical original for source comparison.
        for manifest in (path / "crates").glob("*/Cargo.toml"):
            shutil.copyfile(manifest, manifest.with_name("Cargo.toml.orig"))
            manifest.write_text(manifest.read_text().replace("publish = false\n", "").replace("publish=false\n", ""))
        # Its lockfile comparator expects the binary package's lock beside the
        # manifest. This is the exact checked-in workspace lock, not a re-resolve.
        shutil.copyfile(path / "Cargo.lock", path / "crates/cuepool/Cargo.lock")
        yield path


def external_dependencies(root):
    return [p for p in tomllib.loads((root / "Cargo.lock").read_text())["package"] if "source" in p]


def verify_update(root, before):
    validate_workspace(root)
    if external_dependencies(root) != before:
        raise ValueError("Release preparation changed unrelated external dependencies")
    changed = set(git(root, "diff", "--name-only").splitlines())
    if changed != PRODUCT_FILES:
        raise ValueError(f"Expected exactly product version, lockfile and changelog updates; got {changed}")
    notes = (root / "CHANGELOG.md").read_text()
    if notes.count("## [Unreleased]") != 1:
        raise ValueError("Release preparation duplicated the changelog baseline")


def update(root, previous):
    before = external_dependencies(root)
    subprocess.run([TOOL, "update", "--config", "release-plz.toml", "--registry-manifest-path",
                    str(previous / "Cargo.toml"), "--repo-url", REPOSITORY], cwd=root,
                   env=local_tool_env(), check=True)
    verify_update(root, before)


def packaging_input(path):
    return path in PACKAGING_FILES or path.startswith(".github/packaging/")


def packaging_fixes(root, tag):
    """An explicit packaging patch cannot absorb application or dependency changes."""
    net_paths = git(root, "diff", "--name-only", "--no-renames", tag, "HEAD").splitlines()
    if not any(packaging_input(path) for path in net_paths):
        raise ValueError("Packaging patch requires a changed installer or packaging input")
    fixes = []
    commits = git(root, "log", "--first-parent", "--reverse", "--format=%H", f"{tag}..HEAD").splitlines()
    for commit in commits:
        message = git(root, "show", "-s", "--format=%B", commit)
        subject = message.splitlines()[0]
        if (re.match(r"^feat(?:\([^)]*\))?!?:", subject) or
                re.match(r"^[a-z]+(?:\([^)]*\))?!:", subject) or
                re.search(r"(?m)^BREAKING[ -]CHANGE:", message)):
            raise ValueError("Packaging patch cannot include feature or breaking-change commits")
        paths = git(root, "diff", "--name-only", "--no-renames", commit + "^", commit).splitlines()
        unsupported = [path for path in paths if not (
            packaging_input(path) or path in PACKAGING_SUPPORT_FILES or
            path.startswith(("packaging/", "docs/", ".github/workflows/", ".github/scripts/test_",
                             ".github/scripts/test-")))]
        if unsupported:
            raise ValueError(f"Packaging patch cannot include application/dependency or unrelated changes: {unsupported}")
        match = re.fullmatch(r"fix(?:\([^)]*\))?:\s+(.+)", subject)
        if match and any(packaging_input(path) for path in paths):
            fixes.append(match[1])
    if not fixes:
        raise ValueError("Packaging patch requires a fix: commit changing a packaging input")
    return fixes


def packaging_notes(notes, version, fixes):
    marker = re.search(r"(?m)^## \[Unreleased\][^\n]*\n", notes)
    if not marker or notes.count("## [Unreleased]") != 1:
        raise ValueError("Packaging patch requires exactly one Unreleased heading")
    released = re.search(r"(?m)^## \[", notes[marker.end():])
    if not released:
        raise ValueError("Packaging patch requires existing release history")
    boundary = marker.end() + released.start()
    pending = notes[marker.end():boundary].strip()
    body = (pending + "\n\n" if pending else "") + "### Packaging fixes\n\n"
    body += "\n".join("- " + fix for fix in fixes) + "\n\n"
    date = datetime.now(timezone.utc).date().isoformat()
    return notes[:marker.end()] + f"\n## [{version}] - {date}\n\n" + body + notes[boundary:]


def packaging_patch(root, tag, version):
    fixes = packaging_fixes(root, tag)
    tool_version = subprocess.check_output([TOOL, "--version"], text=True, env=local_tool_env()).strip()
    if tool_version != f"release-plz {TOOL_VERSION}":
        raise ValueError(f"Packaging patch requires release-plz {TOOL_VERSION}; got {tool_version}")
    major, minor, patch = map(int, version.split("."))
    next_version = f"{major}.{minor}.{patch + 1}"
    head = git(root, "rev-parse", "HEAD")
    before = external_dependencies(root)
    # set-version edits an existing candidate heading. Prepare that heading first
    # so the tool never renames the previous published release's notes.
    notes = packaging_notes((root / "CHANGELOG.md").read_bytes().decode(), next_version, fixes)
    with tempfile.TemporaryDirectory(prefix="cuepool-packaging-patch-") as tmp:
        proposal = Path(tmp)
        subprocess.run(["git", "clone", "--quiet", "--shared", str(root), str(proposal)], check=True)
        (proposal / "CHANGELOG.md").write_bytes(notes.encode())
        subprocess.run([TOOL, "set-version", next_version, "--config", "release-plz.toml"],
                       cwd=proposal, env=local_tool_env(), check=True)
        verify_update(proposal, before)
        if product_version(proposal) != next_version or (proposal / "CHANGELOG.md").read_bytes() != notes.encode():
            raise ValueError("Packaging patch changed the requested version or existing changelog history")
        if git(root, "rev-parse", "HEAD") != head or git(root, "status", "--porcelain"):
            raise ValueError("Checkout changed during packaging patch preparation")
        for filename in sorted(PRODUCT_FILES):
            shutil.copyfile(proposal / filename, root / filename)


def prepare(root, command, packaging_patch_requested=False):
    if packaging_patch_requested and command != "update":
        raise ValueError("--packaging-patch supports update only; review and create the maintainer PR separately")
    root = root.resolve()
    if git(root, "status", "--porcelain"):
        raise ValueError("Release preparation requires a clean checkout")
    version = validate_workspace(root)
    tag = "v" + version
    with baseline(root, tag) as previous:
        if packaging_patch_requested:
            packaging_patch(root, tag, version)
            return
        if not release_eligible(root, tag):
            print(f"No release-worthy commits since {tag}; build identity changes only.")
            return
        if command == "update":
            update(root, previous)
        else:
            # Verify the proposal before a remote write. Native release-plz then
            # maintains its existing PR using the same source, config and baseline.
            with tempfile.TemporaryDirectory(prefix="cuepool-release-proposal-") as tmp:
                proposal = Path(tmp)
                subprocess.run(["git", "clone", "--quiet", "--shared", str(root), str(proposal)], check=True)
                update(proposal, previous)
            subprocess.run([TOOL, "release-pr", "--config", "release-plz.toml", "--registry-manifest-path",
                            str(previous / "Cargo.toml"), "--repo-url", REPOSITORY], cwd=root, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["update", "release-pr"])
    parser.add_argument("--packaging-patch", action="store_true",
                        help="Prepare an explicit patch for installer-only fixes outside Cargo packages")
    args = parser.parse_args()
    prepare(Path.cwd(), args.command, args.packaging_patch)
