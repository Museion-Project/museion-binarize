#!/usr/bin/env python3
"""Ordinary (network-free) tests for the distribution scripts.

Run:
    python3 scripts/distribution/test_distribution.py
"""

from __future__ import annotations

import hashlib
import io
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

    @staticmethod
    def _run_add(manifest_path: Path, artifact_filename: str, artifact_path: Path, target_triple: str = "aarch64-apple-darwin") -> None:
        script = REPO_ROOT / "scripts" / "distribution" / "release_manifest.py"
        subprocess.run(
            [
                sys.executable, str(script), "add",
                "--manifest", str(manifest_path),
                "--project-version", "0.1.0", "--git-sha", "deadbeef",
                "--target-triple", target_triple, "--os", "macos", "--arch", "arm64",
                "--artifact-filename", artifact_filename,
                "--artifact-path", str(artifact_path),
                "--pdfium-build", "7920", "--pdfium-version", "151.0.7920.0",
                "--pdfium-sha256", "a" * 64,
                "--signing-state", "unsigned", "--notarization-state", "not_applicable",
            ],
            check=True,
        )

    def test_add_accumulates_multiple_artifacts_for_the_same_target(self):
        # Regression test: a single target (e.g. macOS arm64) produces
        # more than one artifact — a desktop .dmg and a CLI archive —
        # and the packaging workflow calls `add` once per artifact using
        # the *same* --target-triple for both. Deduping on target_triple
        # alone (the original implementation) silently discarded every
        # artifact but the last one processed for that target.
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            manifest_path = tmp_path / "release-manifest.json"
            dmg = tmp_path / "app.dmg"
            dmg.write_bytes(b"dmg bytes")
            cli_archive = tmp_path / "cli.tar.gz"
            cli_archive.write_bytes(b"cli bytes")

            self._run_add(manifest_path, "app.dmg", dmg)
            self._run_add(manifest_path, "cli.tar.gz", cli_archive)

            data = json.loads(manifest_path.read_text())
            filenames = {a["artifact_filename"] for a in data["artifacts"]}
            self.assertEqual(filenames, {"app.dmg", "cli.tar.gz"})
            self.assertEqual(len(data["artifacts"]), 2)

    def test_add_replaces_only_the_matching_filename_on_rerun(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            manifest_path = tmp_path / "release-manifest.json"
            dmg = tmp_path / "app.dmg"
            dmg.write_bytes(b"dmg bytes v1")
            cli_archive = tmp_path / "cli.tar.gz"
            cli_archive.write_bytes(b"cli bytes")

            self._run_add(manifest_path, "app.dmg", dmg)
            self._run_add(manifest_path, "cli.tar.gz", cli_archive)

            dmg.write_bytes(b"dmg bytes v2, rebuilt")
            self._run_add(manifest_path, "app.dmg", dmg)

            data = json.loads(manifest_path.read_text())
            self.assertEqual(len(data["artifacts"]), 2)
            by_name = {a["artifact_filename"]: a for a in data["artifacts"]}
            self.assertEqual(by_name["app.dmg"]["artifact_sha256"], release_manifest.sha256_of(dmg))
            self.assertIn("cli.tar.gz", by_name)


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

    # These validate `_validate_archive_member_path` lexically — as
    # portable archive paths, independent of the host running the test
    # — which is exactly the property that broke the original
    # implementation (a naive `str(path).startswith(str(dest) + "/")`
    # host-path check) on Windows, where `Path.resolve()` yields
    # backslash-separated paths and treats an embedded drive letter or
    # UNC prefix as absolute regardless of what it was joined to.
    ACCEPTED_ARCHIVE_MEMBER_NAMES = [
        "LICENSE",
        "lib/pdfium.dll",
        "nested/normal/file.txt",
    ]
    REJECTED_ARCHIVE_MEMBER_NAMES = [
        "../evil",
        "../../evil",
        "lib/../../../evil",
        "/absolute/path",
        "C:\\evil.dll",
        "C:/evil.dll",
        "\\\\server\\share\\evil.dll",
        "lib\\..\\..\\evil.dll",
    ]

    def test_validate_archive_member_path_accepts_normal_relative_members(self):
        for name in self.ACCEPTED_ARCHIVE_MEMBER_NAMES:
            with self.subTest(name=name):
                fetch_pdfium._validate_archive_member_path(name)

    def test_validate_archive_member_path_rejects_every_unsafe_shape(self):
        for name in self.REJECTED_ARCHIVE_MEMBER_NAMES:
            with self.subTest(name=name):
                with self.assertRaises(ValueError):
                    fetch_pdfium._validate_archive_member_path(name)

    @staticmethod
    def _build_tar(path: Path, member_names: list[str]) -> None:
        with tarfile.open(path, "w") as tar:
            for name in member_names:
                data = b"payload"
                info = tarfile.TarInfo(name=name)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))

    def test_safe_extract_accepts_every_normal_member_shape(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            good_tar = tmp_path / "good.tar"
            self._build_tar(good_tar, self.ACCEPTED_ARCHIVE_MEMBER_NAMES)

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(good_tar) as tar:
                fetch_pdfium._safe_extract(tar, dest)
            self.assertTrue((dest / "LICENSE").is_file())
            self.assertTrue((dest / "lib" / "pdfium.dll").is_file())
            self.assertTrue((dest / "nested" / "normal" / "file.txt").is_file())

    def test_safe_extract_rejects_every_unsafe_member_shape(self):
        for name in self.REJECTED_ARCHIVE_MEMBER_NAMES:
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as tmp:
                    tmp_path = Path(tmp)
                    malicious_tar = tmp_path / "evil.tar"
                    self._build_tar(malicious_tar, [name])

                    dest = tmp_path / "extract-dest"
                    dest.mkdir()
                    with tarfile.open(malicious_tar) as tar:
                        with self.assertRaises(SystemExit):
                            fetch_pdfium._safe_extract(tar, dest)
                    # Nothing must have been written outside dest even
                    # when the unsafe member is rejected before
                    # extraction runs.
                    self.assertEqual(list(dest.iterdir()), [])

    def test_safe_extract_rejects_a_symlink_member_even_with_a_safe_name(self):
        # A symlink's own *name* can look perfectly safe while its
        # *target* (linkname) escapes the destination — extraction must
        # reject the member type outright rather than trust the name.
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            malicious_tar = tmp_path / "evil.tar"
            with tarfile.open(malicious_tar, "w") as tar:
                info = tarfile.TarInfo(name="safe_looking_name.dll")
                info.type = tarfile.SYMTYPE
                info.linkname = "../../../etc/passwd"
                tar.addfile(info)

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(malicious_tar) as tar:
                with self.assertRaises(SystemExit):
                    fetch_pdfium._safe_extract(tar, dest)
            self.assertEqual(list(dest.iterdir()), [])

    def test_safe_extract_rejects_a_hardlink_member_even_with_a_safe_name(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            malicious_tar = tmp_path / "evil.tar"
            with tarfile.open(malicious_tar, "w") as tar:
                info = tarfile.TarInfo(name="safe_looking_name.dll")
                info.type = tarfile.LNKTYPE
                info.linkname = "../../../etc/passwd"
                tar.addfile(info)

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(malicious_tar) as tar:
                with self.assertRaises(SystemExit):
                    fetch_pdfium._safe_extract(tar, dest)
            self.assertEqual(list(dest.iterdir()), [])

    def test_safe_extract_accepts_the_real_windows_pdfium_archive_shape(self):
        # The real pdfium-win-x64.tgz archive (build 7920) contains only
        # regular files and directories at exactly this shape: top-level
        # files (LICENSE, VERSION, ...), the library nested one
        # directory deep (bin/pdfium.dll), and several other nested
        # directories (include/, include/cpp/, lib/, licenses/). None of
        # this should ever be rejected as "escaping."
        real_shape = [
            "LICENSE",
            "VERSION",
            "args.gn",
            "PDFiumConfig.cmake",
            "bin/pdfium.dll",
            "include/fpdfview.h",
            "include/cpp/fpdf_scopers.h",
            "lib/pdfium.dll.lib",
            "licenses/pdfium.txt",
        ]
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            good_tar = tmp_path / "pdfium-win-x64.tar"
            self._build_tar(good_tar, real_shape)

            dest = tmp_path / "extract-dest"
            dest.mkdir()
            with tarfile.open(good_tar) as tar:
                fetch_pdfium._safe_extract(tar, dest)
            self.assertTrue((dest / "bin" / "pdfium.dll").is_file())
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
