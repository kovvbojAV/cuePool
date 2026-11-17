import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import windows_sources as sources


class WindowsSourceTests(unittest.TestCase):
    def test_registry_and_git_sources_keep_exact_versions_and_hashes(self):
        lock = b'''[[package]]
name = "crate"
version = "1.2.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "checksum"
[[package]]
name = "git-crate"
version = "1.0.0"
source = "git+https://github.com/owner/repo?rev=abc#abcdef"
[[package]]
name = "local-crate"
version = "1.0.0"
'''
        self.assertEqual(sources.rust_sources(lock), [
            {"name": "crate", "version": "1.2.3", "url": "https://static.crates.io/crates/crate/crate-1.2.3.crate", "sha256": "checksum"},
            {"name": "git-crate", "repository": "https://github.com/owner/repo", "commit": "abcdef"},
        ])

    def test_source_download_rejects_mismatched_archive(self):
        with patch.object(sources.urllib.request, "urlopen", return_value=io.BytesIO(b"wrong source")):
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                sources.download({"url": "https://example.invalid/source", "sha256": "0" * 64})

    def test_dependency_recipes_include_secondary_repositories_and_svn_revisions(self):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for path, body in {
                "src/scripts.d/one.sh": 'SCRIPT_REPO="https://example/a"\nSCRIPT_COMMIT="abc"\nSCRIPT_REPO2="https://example/b"\nSCRIPT_COMMIT2="def"\n',
                "src/scripts.d/two.sh": 'SCRIPT_REPO="https://example/svn"\nSCRIPT_REV="123"\n',
            }.items():
                data = body.encode()
                member = tarfile.TarInfo(path)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
        self.assertEqual([e["revision"] for e in sources.ffmpeg_sources(buffer.getvalue())], ["abc", "def", "123"])

    def test_source_archive_rejects_missing_files_tampering_and_other_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sources.zip"
            files = {name: b"source bytes" for name in sources.SOURCE_FILES}
            files["SOURCE.json"] = json.dumps({"cuepool_commit": "a" * 40}).encode()
            with self.assertRaisesRegex(ValueError, "Missing Windows sources"):
                sources.write_archive(path, {"SOURCE.json": files["SOURCE.json"]})
            sources.write_archive(path, files)
            sources.validate_archive(path, "a" * 40)
            with self.assertRaisesRegex(ValueError, "different CuePool commit"):
                sources.validate_archive(path, "b" * 40)
            with zipfile.ZipFile(path) as original:
                entries = {n: original.read(n) for n in original.namelist()}
            entries["Cargo.lock"] = b"changed after packaging"
            with zipfile.ZipFile(path, "w") as altered:
                for name, data in entries.items():
                    altered.writestr(name, data)
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                sources.validate_archive(path)


if __name__ == "__main__":
    unittest.main()
