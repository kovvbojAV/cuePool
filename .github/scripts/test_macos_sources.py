import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import macos_sources as sources


class MacOSSourceTests(unittest.TestCase):
    def test_rpath_closure_handles_spaces_aliases_cycles_and_system_libraries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            binary = root / "Cue Pool"
            library = root / "libone.1.2.dylib"
            alias = root / "libone.1.dylib"
            for path in (binary, library):
                path.touch()
            alias.symlink_to(library.name)
            metadata = {
                binary: (["/usr/lib/libSystem.B.dylib", "@rpath/libone.1.dylib"], ["@executable_path"], ["APP"]),
                library: ([str(alias), "/System/Library/Frameworks/Test"], [], ["ONE"]),
            }
            with patch.object(sources, "macho", side_effect=lambda p: metadata[p]):
                self.assertEqual(sources.library_closure(binary), {library.name: (library, ["ONE"])})
            with self.assertRaisesRegex(ValueError, "Unresolved non-system"):
                sources.resolve_dependency("@rpath/missing.dylib", library, binary, [str(root)])

    def test_bundle_requires_every_original_build_and_accepts_load_name_alias(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            original = root / "libone.1.2.dylib"
            original.touch()
            (root / "libone.1.dylib").symlink_to(original.name)
            bundled = root / "bundle"
            bundled.mkdir()
            (bundled / "libone.1.dylib").touch()
            closure = {original.name: (original, ["ORIGINAL"])}
            with patch.object(sources, "macho", return_value=([], [], ["ORIGINAL"])):
                self.assertEqual(set(sources.verify_bundle(closure, bundled)), {"libone.1.dylib"})
                (bundled / "extra.dylib").touch()
                with self.assertRaisesRegex(ValueError, "closure differs"):
                    sources.verify_bundle(closure, bundled)
                (bundled / "extra.dylib").unlink()
            with patch.object(sources, "macho", return_value=([], [], ["ANOTHER-BUILD"])):
                with self.assertRaisesRegex(ValueError, "build UUID differs"):
                    sources.verify_bundle(closure, bundled)
            (bundled / "libone.1.dylib").unlink()
            with self.assertRaisesRegex(ValueError, "missing library"):
                sources.verify_bundle(closure, bundled)

    def test_installed_recipe_is_used_and_missing_licenses_or_moving_sources_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            keg = Path(tmp) / "ffmpeg/9.0.1_1"
            (keg / ".brew").mkdir(parents=True)
            recipe = keg / ".brew/ffmpeg.rb"
            recipe.write_text("installed recipe, not latest")
            (keg / "INSTALL_RECEIPT.json").write_text(json.dumps({
                "source": {"spec": "stable", "versions": {"stable": "9.0.1"}},
            }))
            formula = {"versions": {"stable": "9.0.1"}, "revision": 1, "license": "GPL-3.0-or-later",
                       "urls": {"stable": {"url": "https://example/9.0.1.tar.xz", "checksum": "a" * 64}}}
            with patch.object(sources, "command", return_value=json.dumps({"formulae": [formula]})) as run:
                with self.assertRaisesRegex(ValueError, "No installed dependency notices"):
                    sources.keg_files(keg)
                (keg / "COPYING.GPLv3").write_text("GPL license")
                entry, files = sources.keg_files(keg)
                run.assert_called_with("brew", "info", "--json=v2", "--formula", str(recipe))
                self.assertEqual(entry["version"], "9.0.1_1")
                self.assertEqual(entry["source"]["sha256"], "a" * 64)
                self.assertEqual(files[entry["recipe"]], b"installed recipe, not latest")
                formula["versions"]["stable"] = "9.0.2"
                run.return_value = json.dumps({"formulae": [formula]})
                with self.assertRaisesRegex(ValueError, "version mismatch"):
                    sources.keg_files(keg)
            with self.assertRaisesRegex(ValueError, "no exact checksum"):
                sources.source_spec({"urls": {"stable": {"url": "https://example/main", "revision": "HEAD"}}})
            self.assertEqual(sources.source_spec({"urls": {"stable": {
                "url": "https://example/repo.git", "revision": "b" * 40,
            }}})["git_commit"], "b" * 40)

    def test_source_archive_rejects_other_commit_missing_notice_and_tampering(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sources.zip"
            files = {name: b"source bytes" for name in sources.SOURCE_FILES}
            source = {"url": "https://example/ffmpeg.tar.xz", "sha256": sources.digest(files["ffmpeg-source.tar.xz"])}
            entry = {"name": "ffmpeg", "source": source, "recipe": "homebrew/recipe.rb",
                     "receipt": "homebrew/receipt.json", "metadata": "homebrew/formula.json",
                     "notices": ["homebrew/COPYING"]}
            files.update({"homebrew/recipe.rb": b"build instructions", "homebrew/receipt.json": b"{}",
                          "homebrew/COPYING": b"GPL license", "homebrew/formula.json": sources.json_bytes({
                              "urls": {"stable": {"url": source["url"], "checksum": source["sha256"]}},
                          })})
            files["SOURCE.json"] = sources.json_bytes({"platform": "macos", "cuepool_commit": "a" * 40,
                                                       "homebrew": [entry], "libraries": [
                                                           {"name": "libavcodec.dylib", "formula": "ffmpeg", "uuids": ["BUILD"]},
                                                       ]})
            sources.write_archive(path, files)
            sources.validate_archive(path, "a" * 40)
            with self.assertRaisesRegex(ValueError, "different CuePool commit"):
                sources.validate_archive(path, "b" * 40)
            del files["homebrew/COPYING"]
            sources.write_archive(path, files)
            with self.assertRaisesRegex(ValueError, "source/notice missing"):
                sources.validate_archive(path)
            files["homebrew/COPYING"] = b"GPL license"
            sources.write_archive(path, files)
            with zipfile.ZipFile(path) as archive:
                entries = {name: archive.read(name) for name in archive.namelist()}
            entries["Cargo.lock"] = b"modified"
            with zipfile.ZipFile(path, "w") as archive:
                for name, data in entries.items():
                    archive.writestr(name, data)
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                sources.validate_archive(path)


if __name__ == "__main__":
    unittest.main()
