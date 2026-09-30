"""Check archive contents, checksums, permissions, and missing-file failures."""

import hashlib
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile


SCRIPT = Path(__file__).with_name("package-release.py")


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binaries = self.root / "binaries"
        self.images = self.root / "images"
        self.output = self.root / "dist"
        self.binaries.mkdir()
        self.images.mkdir()
        for name in ("mb-cable", "mb-uploader", "mb-dumper"):
            for suffix in ("", ".exe"):
                (self.binaries / (name + suffix)).write_bytes(name.encode())
        for name in ("pico-multibooter.uf2", "hello-world_mb.gba", "rom-dumper_mb.gba"):
            (self.images / name).write_bytes(name.encode())

    def run_package(self, target, tag="v1.0"):
        return subprocess.run([
            sys.executable, str(SCRIPT), "--tag", tag, "--commit", "a" * 40,
            "--target", target, "--binaries", str(self.binaries),
            "--images", str(self.images), "--output", str(self.output),
        ], capture_output=True, text=True)

    def test_platform_archives(self):
        for target in (
            "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc",
            "x86_64-apple-darwin", "aarch64-apple-darwin",
        ):
            with self.subTest(target=target):
                result = self.run_package(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                archive = Path(result.stdout.strip())
                if archive.suffix == ".zip":
                    with zipfile.ZipFile(archive) as package:
                        files = {Path(name).name: package.read(name) for name in package.namelist()}
                    suffix = ".exe"
                else:
                    with tarfile.open(archive) as package:
                        files = {}
                        for member in package.getmembers():
                            if member.isfile():
                                files[Path(member.name).name] = package.extractfile(member).read()
                                if Path(member.name).name.startswith("mb-"):
                                    self.assertEqual(member.mode & 0o777, 0o755)
                    suffix = ""
                self.assertEqual(set(files), {
                    "mb-cable" + suffix, "mb-uploader" + suffix, "mb-dumper" + suffix,
                    "pico-multibooter.uf2", "hello-world_mb.gba", "rom-dumper_mb.gba",
                    "README.md", "BUILDINFO.json", "SHA256SUMS",
                })
                hashes = files["SHA256SUMS"].decode().splitlines()
                self.assertEqual(len(hashes), 8)
                for line in hashes:
                    digest, name = line.split("  ")
                    self.assertEqual(hashlib.sha256(files[name]).hexdigest(), digest)

    def test_missing_or_empty_file_fails_before_archive(self):
        path = self.images / "rom-dumper_mb.gba"
        path.unlink()
        for missing in (True, False):
            if not missing:
                path.touch()
            result = self.run_package("x86_64-unknown-linux-gnu")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing or empty", result.stderr)
            self.assertFalse(self.output.exists())

    def test_tag_cannot_escape_output_directory(self):
        result = self.run_package("x86_64-unknown-linux-gnu", "v1/../../escape")
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
