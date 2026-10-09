"""Tests for release_tools.py. Run: python3 -m unittest discover .github/scripts"""
import hashlib
import json
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_tools as rt  # noqa: E402

CHANGELOG = """# Changelog

## [Unreleased]

## [1.2.0] - 2026-10-01

### Added
- A feature.

## [1.1.0] - Unreleased

### Fixed
- Something.

[Unreleased]: https://example.com
"""


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "1.2.0"\n'
            'repository = "https://github.com/acme/pdfjiff"\n'
        )
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)
        for name in rt.BUNDLED_DOCS:
            (self.root / name).write_text(f"{name}\n")

    def tearDown(self):
        self.tmp.cleanup()


class VersionAndNotes(Fixture):
    def test_matching_tag_with_dated_section_passes(self):
        self.assertEqual(rt.check_tag("v1.2.0", self.root), "1.2.0")

    def test_stale_core_dependency_version_is_rejected(self):
        manifest = (self.root / "Cargo.toml").read_text()
        (self.root / "Cargo.toml").write_text(
            manifest + '\n[workspace.dependencies]\npdfjiff-core = { path = "crates/pdfjiff-core", version = "1.1.0" }\n'
        )
        with self.assertRaisesRegex(rt.ReleaseError, "set it to 1.2.0"):
            rt.check_tag("v1.2.0", self.root)

    def test_mismatched_tag_is_rejected(self):
        with self.assertRaisesRegex(rt.ReleaseError, "does not match"):
            rt.check_tag("v1.2.1", self.root)

    def test_undated_section_is_rejected(self):
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.1.0"\n')
        with self.assertRaisesRegex(rt.ReleaseError, "set the release date"):
            rt.check_tag("v1.1.0", self.root)

    def test_missing_section_is_rejected(self):
        with self.assertRaisesRegex(rt.ReleaseError, "no '## \\[9.9.9\\]'"):
            rt.changelog_section("9.9.9", CHANGELOG)

    def test_notes_stop_at_next_section(self):
        heading, body = rt.changelog_section("1.2.0", CHANGELOG)
        self.assertEqual(heading, "## [1.2.0] - 2026-10-01")
        self.assertEqual(body, "### Added\n- A feature.")
        self.assertEqual(rt.changelog_section("1.1.0", CHANGELOG)[1], "### Fixed\n- Something.")


class Packaging(Fixture):
    def binary(self):
        path = self.root / "bin"
        path.write_bytes(b"\x7fELF fake binary")
        return path

    def test_unix_archive_layout_and_modes(self):
        out = rt.package("x86_64-unknown-linux-musl", self.binary(), self.root / "dist", self.root)
        self.assertEqual(out.name, "pdfjiff-x86_64-unknown-linux-musl.tar.gz")
        with tarfile.open(out) as archive:
            members = {m.name: m for m in archive.getmembers()}
        folder = "pdfjiff-x86_64-unknown-linux-musl"
        self.assertEqual(set(members), {folder, f"{folder}/pdfjiff", *(f"{folder}/{d}" for d in rt.BUNDLED_DOCS)})
        self.assertEqual(members[f"{folder}/pdfjiff"].mode, 0o755)
        self.assertEqual(members[f"{folder}/README.md"].mode, 0o644)

    def test_third_party_notices_are_bundled_when_generated(self):
        (self.root / rt.NOTICES).write_text("# Third-party licenses\n")
        out = rt.package("x86_64-unknown-linux-musl", self.binary(), self.root / "dist", self.root)
        with tarfile.open(out) as archive:
            self.assertIn(f"pdfjiff-x86_64-unknown-linux-musl/{rt.NOTICES}", archive.getnames())

    def test_archives_are_reproducible(self):
        first = rt.package("aarch64-apple-darwin", self.binary(), self.root / "a", self.root).read_bytes()
        second = rt.package("aarch64-apple-darwin", self.binary(), self.root / "b", self.root).read_bytes()
        self.assertEqual(first, second)

    def test_windows_archive_is_zip_with_exe(self):
        out = rt.package("x86_64-pc-windows-msvc", self.binary(), self.root / "dist", self.root)
        with zipfile.ZipFile(out) as archive:
            self.assertIn("pdfjiff-x86_64-pc-windows-msvc/pdfjiff.exe", archive.namelist())

    def test_checksums_match_sha256sum_format(self):
        dist = self.root / "dist"
        out = rt.package("x86_64-unknown-linux-musl", self.binary(), dist, self.root)
        sums = rt.write_checksums(dist)
        line = sums.read_text().strip()
        self.assertEqual(line, f"{hashlib.sha256(out.read_bytes()).hexdigest()}  {out.name}")
        self.assertEqual(rt.read_checksums(sums), {out.name: hashlib.sha256(out.read_bytes()).hexdigest()})


class Manifests(unittest.TestCase):
    SUMS = {f"pdfjiff-{t}.tar.gz": c * 64 for t, c in zip(rt.UNIX_TARGETS.values(), "abcd")}
    SUMS.update({f"pdfjiff-{t}.zip": c * 64 for t, c in zip(rt.WINDOWS_TARGETS.values(), "ef")})

    def test_formula_pairs_each_url_with_its_checksum(self):
        formula = rt.homebrew_formula("1.2.0", self.SUMS, "acme/pdfjiff")
        for target, char in zip(rt.UNIX_TARGETS.values(), "abcd"):
            url = f"https://github.com/acme/pdfjiff/releases/download/v1.2.0/pdfjiff-{target}.tar.gz"
            self.assertIn(f'url "{url}"\n      sha256 "{char * 64}"', formula)
        self.assertIn('generate_completions_from_executable(bin/"pdfjiff", "completions", shells: [:bash, :zsh, :fish, :pwsh])', formula)
        self.assertIn('shell_output("#{bin}/pdfjiff --version")', formula)

    def test_missing_checksum_fails_loudly(self):
        with self.assertRaisesRegex(rt.ReleaseError, "no entry"):
            rt.homebrew_formula("1.2.0", {}, "acme/pdfjiff")

    def test_scoop_manifest(self):
        manifest = json.loads(rt.scoop_manifest("1.2.0", self.SUMS, "acme/pdfjiff"))
        self.assertEqual(manifest["version"], "1.2.0")
        self.assertEqual(manifest["architecture"]["64bit"]["hash"], "e" * 64)
        self.assertEqual(manifest["architecture"]["arm64"]["extract_dir"], "pdfjiff-aarch64-pc-windows-msvc")
        self.assertEqual(manifest["bin"], "pdfjiff.exe")


if __name__ == "__main__":
    unittest.main()
