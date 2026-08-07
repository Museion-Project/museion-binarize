#!/usr/bin/env python3
"""Ordinary (network-free) tests for the distribution scripts.

Run:
    python3 scripts/distribution/test_distribution.py
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import checksums  # noqa: E402
import fetch_pdfium  # noqa: E402
import naming  # noqa: E402
import release_manifest  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]


class NamingTests(unittest.TestCase):
    def test_desktop_artifact_name_matches_the_documented_convention(self):
        self.assertEqual(
            naming.desktop_artifact_name("0.1.0", "aarch64-apple-darwin", "dmg"),
            "Museion-Binarize-0.1.0-macos-arm64.dmg",
        )
        self.assertEqual(
            naming.desktop_artifact_name("0.1.0", "x86_64-pc-windows-msvc", "msi"),
            "Museion-Binarize-0.1.0-windows-x64.msi",
        )

    def test_cli_archive_name_matches_the_documented_convention(self):
        self.assertEqual(
            naming.cli_archive_name("0.1.0", "x86_64-unknown-linux-gnu", "tar.gz"),
            "museion-binarize-cli-0.1.0-linux-x86_64.tar.gz",
        )

    def test_unknown_target_triple_is_rejected(self):
        with self.assertRaises(ValueError):
            naming.desktop_artifact_name("0.1.0", "riscv64-unknown-linux-gnu", "dmg")

    def test_no_filename_contains_a_workflow_run_number_or_timestamp(self):
        name = naming.desktop_artifact_name("0.1.0", "aarch64-apple-darwin", "dmg")
        # A version-and-target-only name has no digits beyond the version
        # itself and the (fixed-format) target label.
        self.assertNotIn("_run_", name)
        self.assertNotIn("T00", name)  # a stray ISO-timestamp fragment


class ChecksumsTests(unittest.TestCase):
    def test_generates_correct_sha256_for_every_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "a.txt").write_bytes(b"hello")
            (tmp_path / "b.bin").write_bytes(b"\x00\x01\x02world")

            content = checksums.generate(tmp_path)
            lines = {}
            for line in content.strip().splitlines():
                digest, name = line.split("  ", 1)
                lines[name] = digest

            self.assertEqual(
                lines["a.txt"], hashlib.sha256(b"hello").hexdigest()
            )
            self.assertEqual(
                lines["b.bin"], hashlib.sha256(b"\x00\x01\x02world").hexdigest()
            )

    def test_rejects_an_empty_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(SystemExit):
                checksums.generate(Path(tmp))

    def test_excludes_a_preexisting_sha256sums_file_from_itself(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "artifact.bin").write_bytes(b"payload")
            (tmp_path / "SHA256SUMS").write_text("stale content\n")
            content = checksums.generate(tmp_path)
            self.assertIn("artifact.bin", content)
            self.assertNotIn("SHA256SUMS", content)


class ReleaseManifestTests(unittest.TestCase):
    def test_build_entry_rejects_invalid_signing_state(self):
        with self.assertRaises(ValueError):
            release_manifest.build_entry(
                target_triple="aarch64-apple-darwin",
                os_name="macos",
                arch="arm64",
                artifact_filename="x.dmg",
                artifact_sha256="0" * 64,
                pdfium_build="7920",
                pdfium_version="151.0.7920.0",
                pdfium_sha256="0" * 64,
                signing_state="definitely-signed-trust-me",
                notarization_state="not_applicable",
            )

    def test_full_manifest_round_trip_has_correct_artifact_digest(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifact = tmp_path / "artifact.dmg"
            artifact.write_bytes(b"fake dmg bytes")
            manifest_path = tmp_path / "release-manifest.json"

            data = release_manifest.load_or_init(manifest_path, "0.1.0", "deadbeef")
            entry = release_manifest.build_entry(
                target_triple="aarch64-apple-darwin",
                os_name="macos",
                arch="arm64",
                artifact_filename="artifact.dmg",
                artifact_sha256=release_manifest.sha256_of(artifact),
                pdfium_build="7920",
                pdfium_version="151.0.7920.0",
                pdfium_sha256="a" * 64,
                signing_state="unsigned",
                notarization_state="not_applicable",
            )
            data["artifacts"].append(entry)
            manifest_path.write_text(json.dumps(data, indent=2))

            loaded = json.loads(manifest_path.read_text())
            self.assertEqual(loaded["schema"], "museion-binarize-release-manifest")
            self.assertEqual(loaded["schema_version"], "1.0")
            self.assertEqual(
                loaded["artifacts"][0]["artifact_sha256"],
                hashlib.sha256(b"fake dmg bytes").hexdigest(),
            )

    def test_manifest_contains_no_private_path_or_secret_looking_keys(self):
        entry = release_manifest.build_entry(
            target_triple="aarch64-apple-darwin",
            os_name="macos",
            arch="arm64",
            artifact_filename="artifact.dmg",
            artifact_sha256="a" * 64,
            pdfium_build="7920",
            pdfium_version="151.0.7920.0",
            pdfium_sha256="b" * 64,
            signing_state="unsigned",
            notarization_state="not_applicable",
        )
        blob = json.dumps(entry).lower()
        for forbidden in ["/users/", "/home/", "password", "token", "secret", "apikey"]:
            self.assertNotIn(forbidden, blob)

    def test_load_or_init_refuses_to_mix_different_builds_into_one_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = Path(tmp) / "release-manifest.json"
            data = release_manifest.load_or_init(manifest_path, "0.1.0", "sha-a")
            manifest_path.write_text(json.dumps(data))
            with self.assertRaises(SystemExit):
                release_manifest.load_or_init(manifest_path, "0.1.0", "sha-b")


class FetchPdfiumSafetyTests(unittest.TestCase):
    def test_manifest_parses_and_every_entry_has_64_hex_char_checksums(self):
        with fetch_pdfium.MANIFEST_PATH.open("rb") as f:
            import tomllib

            data = tomllib.load(f)
        self.assertEqual(data["schema"], "museion-binarize-pdfium-manifest")
        assets = data["asset"]
        self.assertGreaterEqual(len(assets), 4)
        for asset in assets:
            for key in ("archive_sha256", "library_sha256"):
                value = asset[key]
                self.assertEqual(len(value), 64, f"{asset['target_triple']}.{key}")
                int(value, 16)  # must be valid hex

    def test_manifest_has_no_target_named_latest_or_using_a_latest_url(self):
        with fetch_pdfium.MANIFEST_PATH.open("rb") as f:
            import tomllib

            data = tomllib.load(f)
        for asset in data["asset"]:
            self.assertNotIn("latest", asset["archive_url"].lower())

    def test_safe_extract_rejects_a_path_traversal_member(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            malicious_tar = tmp_path / "evil.tar"
            evil_file = tmp_path / "evil.txt"
            evil_file.write_text("pwned")
            with tarfile.open(malicious_tar, "w") as tar:
                tar.add(evil_file, arcname="../../../etc/evil.txt")

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(malicious_tar) as tar:
                with self.assertRaises(SystemExit):
                    fetch_pdfium._safe_extract(tar, dest)

    def test_safe_extract_allows_a_normal_nested_member(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            good_tar = tmp_path / "good.tar"
            payload = tmp_path / "payload.txt"
            payload.write_text("fine")
            with tarfile.open(good_tar, "w") as tar:
                tar.add(payload, arcname="lib/payload.txt")

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(good_tar) as tar:
                fetch_pdfium._safe_extract(tar, dest)
            self.assertTrue((dest / "lib" / "payload.txt").is_file())

    def test_safe_extract_allows_a_top_level_member(self):
        # Regression test: a naive `str(path).startswith(str(dest) +
        # "/")` containment check (the original implementation) breaks on
        # Windows, where `Path.resolve()` yields backslash-separated
        # paths, causing every extracted member — including ordinary
        # top-level files like the archive's LICENSE — to be rejected as
        # "escaping" the destination. `Path.is_relative_to` must be used
        # instead, since it is separator-agnostic.
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            good_tar = tmp_path / "good.tar"
            payload = tmp_path / "LICENSE"
            payload.write_text("license text")
            with tarfile.open(good_tar, "w") as tar:
                tar.add(payload, arcname="LICENSE")

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(good_tar) as tar:
                fetch_pdfium._safe_extract(tar, dest)
            self.assertTrue((dest / "LICENSE").is_file())


class TauriResourceConfigTests(unittest.TestCase):
    """Regression coverage for the ordinary-vs-distribution Tauri config
    split: `tauri.conf.json` must never require a staged PDFium resource
    to compile/lint/test, while `tauri.dist.conf.json` (merged in only by
    the distribution packaging steps) must reference it."""

    BASE_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
    DIST_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.dist.conf.json"

    def test_base_config_does_not_reference_pdfium_resources(self):
        config = json.loads(self.BASE_CONFIG.read_text())
        resources = config.get("bundle", {}).get("resources", {})
        self.assertEqual(
            resources,
            {},
            "tauri.conf.json must not declare bundle.resources — ordinary "
            "`cargo build`/`clippy`/`test` compile against this file via "
            "tauri::generate_context! and must not require a staged "
            "PDFium binary that only distribution packaging provides",
        )

    def test_dist_config_references_the_staged_pdfium_glob(self):
        config = json.loads(self.DIST_CONFIG.read_text())
        resources = config["bundle"]["resources"]
        self.assertIn("resources/pdfium/*", resources)

    def test_dist_config_is_pure_overlay_with_no_top_level_conflicts(self):
        # It must only carry the `bundle.resources` override so it stays a
        # safe `--config` overlay merged on top of the base config for
        # distribution builds, not a divergent duplicate configuration.
        config = json.loads(self.DIST_CONFIG.read_text())
        self.assertEqual(set(config["bundle"].keys()), {"resources"})
        self.assertEqual(
            set(config.keys()) - {"$schema"},
            {"bundle"},
        )

    def test_staging_fails_closed_for_a_target_with_no_pinned_pdfium(self):
        result = subprocess.run(
            [
                sys.executable,
                str(REPO_ROOT / "scripts" / "distribution" / "stage_desktop_pdfium.py"),
                "not-a-real-target-triple",
            ],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no PDFium asset pinned", result.stderr)

    def test_no_pdfium_dynamic_library_is_tracked_in_git(self):
        result = subprocess.run(
            ["git", "ls-files"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        offenders = [
            path
            for path in result.stdout.splitlines()
            if path.lower().endswith((".dylib", ".dll", ".so"))
        ]
        self.assertEqual(offenders, [], f"committed native binaries found: {offenders}")

    def test_resources_pdfium_directory_is_gitignored(self):
        result = subprocess.run(
            ["git", "check-ignore", "apps/desktop/src-tauri/resources/pdfium/libpdfium.dylib"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, "staged PDFium resource dir must stay gitignored")


class VersionConsistencyScriptTests(unittest.TestCase):
    def test_passes_on_the_real_repository_as_committed(self):
        result = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "distribution" / "check_version_consistency.py")],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("version consistency OK", result.stdout)


if __name__ == "__main__":
    unittest.main()
