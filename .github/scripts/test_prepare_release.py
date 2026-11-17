"""Packaging-only release preparation: scope guards and the real pinned tool."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("prepare_release", ROOT / "scripts/prepare-release.py")
prepare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prepare)
TOOL = os.environ.get("RELEASE_PLZ", "release-plz")
NAMES = sorted(tomllib.loads(p.read_text())["package"]["name"] for p in (ROOT / "crates").glob("*/Cargo.toml"))
HISTORY = "## [0.12.3]\n\n- Existing release must stay byte-for-byte.\n\n## [0.12.2]\n\n- Older release.\n"


class PackagingPatchTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="cuepool-packaging-patch-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Packaging policy test")
        self.git("config", "user.email", "release@example.invalid")
        self.write("Cargo.toml", '[workspace]\nmembers=["crates/*"]\nresolver="3"\n'
                   '[workspace.package]\nversion="0.12.3"\nedition="2024"\n')
        for name in NAMES:
            self.write(f"crates/{name}/Cargo.toml", f'[package]\nname="{name}"\nversion.workspace=true\nedition.workspace=true\n')
            self.write(f"crates/{name}/src/lib.rs", "pub fn sample() {}\n")
        self.write("Cargo.lock", "version = 4\n" + "".join(
            f'\n[[package]]\nname = "{name}"\nversion = "0.12.3"\n' for name in NAMES))
        self.write("release-plz.toml", (ROOT / "release-plz.toml").read_text())
        self.write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n\n" + HISTORY)
        self.commit("chore: baseline")
        self.git("tag", "v0.12.3")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True, stderr=subprocess.PIPE).strip()

    def write(self, filename, content):
        path = self.root / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-m", message)

    def packaging_fix(self, message="fix: correct Windows package publisher"):
        self.write(".github/scripts/make-msi.ps1", "# Manufacturer kovvbojAV\n")
        self.commit(message)

    def test_packaging_fix_and_maintenance_are_eligible(self):
        self.packaging_fix()
        self.write("docs/releases.md", "Packaging release instructions.\n")
        self.commit("docs: explain packaging patch preparation")
        self.assertEqual(prepare.packaging_fixes(self.root, "v0.12.3"), ["correct Windows package publisher"])

    def reject_input_change(self, filename):
        self.packaging_fix()
        path = self.root / filename
        original = path.read_text()
        path.write_text(original + "\n# application or dependency edit\n")
        self.commit("fix: change application input")
        path.write_text(original)
        self.commit("chore: revert application input")
        with self.assertRaisesRegex(ValueError, "application/dependency"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_application_change_is_rejected_even_when_reverted(self):
        self.reject_input_change("crates/cuepool/src/lib.rs")

    def test_crate_manifest_change_is_rejected_even_when_reverted(self):
        self.reject_input_change("crates/cuepool/Cargo.toml")

    def test_root_manifest_change_is_rejected_even_when_reverted(self):
        self.reject_input_change("Cargo.toml")

    def test_lockfile_change_is_rejected_even_when_reverted(self):
        self.reject_input_change("Cargo.lock")

    def test_feature_is_not_a_patch_release(self):
        self.packaging_fix("feat: new installer behavior")
        with self.assertRaisesRegex(ValueError, "feature or breaking"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_breaking_subject_is_not_a_patch_release(self):
        self.packaging_fix("fix!: incompatible installer")
        with self.assertRaisesRegex(ValueError, "feature or breaking"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_breaking_footer_is_not_a_patch_release(self):
        self.packaging_fix("fix: revise installer\n\nBREAKING CHANGE: incompatible")
        with self.assertRaisesRegex(ValueError, "feature or breaking"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_ci_only_change_is_not_a_packaging_fix(self):
        self.packaging_fix("ci: maintain packaging checks")
        with self.assertRaisesRegex(ValueError, "requires a fix:"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_documentation_fix_has_no_packaging_input(self):
        self.write("packaging/README.md", "Installer documentation.\n")
        self.commit("fix: correct documentation")
        with self.assertRaisesRegex(ValueError, "requires a changed installer"):
            prepare.packaging_fixes(self.root, "v0.12.3")

    def test_dirty_checkout_and_remote_mode_are_rejected(self):
        self.packaging_fix()
        self.write("operator-notes.txt", "Uncommitted work.\n")
        with self.assertRaisesRegex(ValueError, "clean checkout"):
            prepare.prepare(self.root, "update", True)
        with self.assertRaisesRegex(ValueError, "update only"):
            prepare.prepare(self.root, "release-pr", True)

    def test_missing_or_nonancestral_tag_is_rejected(self):
        self.packaging_fix()
        self.git("tag", "-d", "v0.12.3")
        with self.assertRaises(subprocess.CalledProcessError):
            prepare.prepare(self.root, "update", True)
        tree = self.git("rev-parse", "HEAD^{tree}")
        unrelated = self.git("commit-tree", tree, "-m", "Unrelated baseline")
        self.git("tag", "v0.12.3", unrelated)
        with self.assertRaises(subprocess.CalledProcessError):
            prepare.prepare(self.root, "update", True)

    def test_notes_preserve_pending_copy_and_release_history(self):
        original = "# Changelog\n\n## [Unreleased]\n\nOperator impact.\n\n" + HISTORY
        notes = prepare.packaging_notes(original, "0.12.4", ["correct publisher"])
        self.assertIn("Operator impact.", notes)
        self.assertIn("- correct publisher", notes)
        self.assertEqual(notes.count("## [Unreleased]"), 1)
        self.assertTrue(notes.endswith(HISTORY))

    def test_tool_failure_leaves_original_checkout_clean(self):
        self.packaging_fix()
        original_run = subprocess.run
        original_output = subprocess.check_output

        def output(args, **kwargs):
            if args == [TOOL, "--version"]:
                return "release-plz 0.3.169\n"
            return original_output(args, **kwargs)

        def run(args, **kwargs):
            if args[:2] == [TOOL, "set-version"]:
                raise subprocess.CalledProcessError(1, args)
            return original_run(args, **kwargs)

        with patch.object(prepare, "TOOL", TOOL), patch.object(subprocess, "check_output", side_effect=output), patch.object(subprocess, "run", side_effect=run):
            with self.assertRaises(subprocess.CalledProcessError):
                prepare.prepare(self.root, "update", True)
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertEqual(prepare.product_version(self.root), "0.12.3")

    @unittest.skipUnless(shutil.which(TOOL), "Real release-plz tool runs in the release-policy job")
    def test_real_tool_preserves_workspace_history_and_external_dependencies(self):
        # Lock an older external dependency before the baseline to detect an
        # accidental general cargo update during workspace version changes.
        manifest = self.root / "crates/cuepool/Cargo.toml"
        manifest.write_text(manifest.read_text() + '\n[dependencies]\nitoa="1"\n')
        subprocess.run(["cargo", "generate-lockfile"], cwd=self.root, check=True, capture_output=True)
        subprocess.run(["cargo", "update", "-p", "itoa", "--precise", "1.0.15"], cwd=self.root, check=True, capture_output=True)
        self.commit("chore: baseline external dependency")
        self.git("tag", "-f", "v0.12.3")
        self.packaging_fix()
        before = prepare.external_dependencies(self.root)
        with patch.object(prepare, "TOOL", TOOL):
            prepare.prepare(self.root, "update", True)
        self.assertEqual(prepare.validate_workspace(self.root), "0.12.4")
        self.assertEqual(prepare.external_dependencies(self.root), before)
        self.assertEqual(set(self.git("diff", "--name-only").splitlines()), prepare.PRODUCT_FILES)
        notes = (self.root / "CHANGELOG.md").read_text()
        self.assertTrue(notes.endswith(HISTORY))
        self.assertIn("- correct Windows package publisher", notes)
        for manifest in (self.root / "crates").glob("*/Cargo.toml"):
            self.assertEqual(tomllib.loads(manifest.read_text())["package"]["version"], {"workspace": True})


if __name__ == "__main__":
    unittest.main()
