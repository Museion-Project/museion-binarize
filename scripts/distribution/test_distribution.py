#!/usr/bin/env python3
"""Ordinary (network-free) tests for the distribution scripts.

Run:
    python3 scripts/distribution/test_distribution.py
"""

from __future__ import annotations

import hashlib
import io
import json
import plistlib
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
import package_macos_dmg  # noqa: E402
import package_mas  # noqa: E402
import release_manifest  # noqa: E402
import render_mas_entitlements  # noqa: E402
import sign_macos_app  # noqa: E402

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

    def test_add_keeps_both_artifacts_for_the_same_target_triple(self):
        # A single target (e.g. aarch64-apple-darwin) produces two
        # distinct artifacts: the desktop .dmg and the CLI archive.
        # Regression test: the dedup key used to be target_triple alone,
        # which silently dropped whichever of the two was added first.
        # Invokes the real CLI (not release_manifest internals directly)
        # so this exercises the exact code path the build workflow runs.
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            dmg = tmp_path / "Museion-Binarize-0.1.0-macos-arm64.dmg"
            dmg.write_bytes(b"fake dmg bytes")
            cli_archive = tmp_path / "museion-binarize-cli-0.1.0-macos-arm64.tar.gz"
            cli_archive.write_bytes(b"fake cli archive bytes")
            manifest_path = tmp_path / "release-manifest.json"
            script = REPO_ROOT / "scripts" / "distribution" / "release_manifest.py"

            for artifact in (dmg, cli_archive):
                result = subprocess.run(
                    [
                        sys.executable, str(script), "add",
                        "--manifest", str(manifest_path),
                        "--project-version", "0.1.0",
                        "--git-sha", "deadbeef",
                        "--target-triple", "aarch64-apple-darwin",
                        "--os", "macos",
                        "--arch", "arm64",
                        "--artifact-filename", artifact.name,
                        "--artifact-path", str(artifact),
                        "--pdfium-build", "7920",
                        "--pdfium-version", "151.0.7920.0",
                        "--pdfium-sha256", "a" * 64,
                        "--signing-state", "unsigned",
                        "--notarization-state", "not_applicable",
                    ],
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

            loaded = json.loads(manifest_path.read_text())
            filenames = {a["artifact_filename"] for a in loaded["artifacts"]}
            self.assertEqual(filenames, {dmg.name, cli_archive.name})

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


class BundleIdentifierConsistencyTests(unittest.TestCase):
    """The canonical product identity is `me.museion.binarize`, declared
    exactly once, in the base `tauri.conf.json` — see
    docs/mac-app-store-readiness.md, "Bundle identifier." Every overlay
    (GitHub dist, MAS) must inherit it rather than redeclaring it, so it
    is structurally impossible for the two distribution channels to
    represent different products. The previous identifier,
    `org.museionproject.binarize`, must not reappear in any active
    configuration."""

    CANONICAL_IDENTIFIER = "me.museion.binarize"
    OLD_IDENTIFIER = "org.museionproject.binarize"

    BASE_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
    DIST_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.dist.conf.json"
    MAS_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.mas.conf.json"

    def test_base_config_declares_the_canonical_identifier(self):
        config = json.loads(self.BASE_CONFIG.read_text())
        self.assertEqual(config["identifier"], self.CANONICAL_IDENTIFIER)

    def test_dist_and_mas_overlays_never_redeclare_identifier(self):
        # Both must inherit from the base config alone — a distribution
        # overlay declaring its own "identifier" would let the GitHub
        # and MAS builds silently diverge into different products.
        for config_path in (self.DIST_CONFIG, self.MAS_CONFIG):
            config = json.loads(config_path.read_text())
            self.assertNotIn(
                "identifier", config, f"{config_path} must not redeclare \"identifier\""
            )

    def test_old_identifier_is_absent_from_active_configuration(self):
        # Scoped to config/scripts, not the whole repo: CHANGELOG.md and
        # this doc's own "Bundle identifier" section legitimately narrate
        # the migration ("previously org.museionproject.binarize") as
        # historical record, which the M7B1 migration brief explicitly
        # says to preserve, not strip. What must never drift back to the
        # old value is active configuration.
        result = subprocess.run(
            [
                "git", "grep", "-l", "-F", self.OLD_IDENTIFIER,
                "--",
                "*.json", "*.py", "*.toml", "*.rs", "*.ts", "*.tsx",
                # This test file must name the retired identifier
                # literally to check for it.
                ":!scripts/distribution/test_distribution.py",
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        # `git grep` exits 1 when it finds no matches — that is the
        # passing case here, not an error.
        self.assertEqual(
            result.returncode,
            1,
            f"old identifier {self.OLD_IDENTIFIER!r} still appears in active "
            f"configuration/code: {result.stdout}",
        )

    def test_render_mas_entitlements_reads_the_canonical_identifier(self):
        self.assertEqual(render_mas_entitlements.bundle_identifier(), self.CANONICAL_IDENTIFIER)


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


class MasConfigTests(unittest.TestCase):
    """M7B1: Mac App Store config/entitlements must never leak into the
    ordinary or GitHub-distribution paths, and must never carry a
    broader entitlement set than this application has demonstrated a
    need for. See docs/mac-app-store-readiness.md."""

    BASE_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
    DIST_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.dist.conf.json"
    MAS_CONFIG = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.mas.conf.json"
    ENTITLEMENTS_TEMPLATE = (
        REPO_ROOT / "apps" / "desktop" / "src-tauri" / "entitlements.mas.plist.template"
    )

    def test_entitlements_template_has_exactly_the_intended_keys(self):
        with self.ENTITLEMENTS_TEMPLATE.open("rb") as f:
            entitlements = plistlib.load(f)
        self.assertEqual(entitlements["com.apple.security.app-sandbox"], True)
        self.assertEqual(entitlements["com.apple.security.files.user-selected.read-write"], True)
        # Everything else is placeholder identity metadata, not a
        # capability grant — but confirm no unexpected key sneaked in.
        self.assertEqual(
            set(entitlements.keys()),
            {
                "com.apple.security.app-sandbox",
                "com.apple.security.files.user-selected.read-write",
                "com.apple.application-identifier",
                "com.apple.developer.team-identifier",
            },
        )

    def test_entitlements_template_contains_no_forbidden_broad_entitlement(self):
        package_mas.validate_entitlements_plist_file(self.ENTITLEMENTS_TEMPLATE)

    def test_neither_base_nor_dist_config_declares_macos_entitlements(self):
        # A GitHub Developer-ID/ad-hoc build must never accidentally
        # inherit App Sandbox — that is a different distribution
        # channel with a different security model (see
        # docs/mac-app-store-readiness.md, "Signing/provisioning").
        for config_path in (self.BASE_CONFIG, self.DIST_CONFIG):
            config = json.loads(config_path.read_text())
            self.assertNotIn(
                "entitlements",
                config.get("bundle", {}).get("macOS", {}),
                f"{config_path} must not declare a macOS entitlements file",
            )

    def test_mas_config_references_entitlements_and_pdfium_resources(self):
        config = json.loads(self.MAS_CONFIG.read_text())
        self.assertEqual(config["bundle"]["macOS"]["entitlements"], "./entitlements.mas.plist")
        self.assertIn("resources/pdfium/*", config["bundle"]["resources"])

    def test_mas_config_does_not_redeclare_identifier_or_version(self):
        # Both must come from the base config alone, so they can never
        # drift between the GitHub and MAS builds of "the same app."
        config = json.loads(self.MAS_CONFIG.read_text())
        self.assertNotIn("identifier", config)
        self.assertNotIn("version", config)

    def test_mas_config_never_hardcodes_a_signing_identity(self):
        # Team ID / signing identity must come from the environment at
        # build time (APPLE_TEAM_ID / APPLE_SIGNING_IDENTITY) — never a
        # literal value committed to this file.
        config = json.loads(self.MAS_CONFIG.read_text())
        self.assertNotIn("signingIdentity", config.get("bundle", {}).get("macOS", {}))

    def test_mas_config_is_a_pure_overlay_with_no_unexpected_top_level_keys(self):
        config = json.loads(self.MAS_CONFIG.read_text())
        self.assertEqual(set(config.keys()) - {"$schema"}, {"bundle"})
        self.assertEqual(set(config["bundle"].keys()), {"category", "resources", "macOS"})

    def test_rendered_entitlements_are_gitignored(self):
        result = subprocess.run(
            ["git", "check-ignore", "apps/desktop/src-tauri/entitlements.mas.plist"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, "rendered MAS entitlements must stay gitignored")

    def test_provisioning_profile_path_is_gitignored(self):
        result = subprocess.run(
            ["git", "check-ignore", "apps/desktop/src-tauri/embedded.provisionprofile"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, "a real provisioning profile must stay gitignored")


class RenderMasEntitlementsTests(unittest.TestCase):
    def test_fails_closed_when_team_id_is_unset(self):
        env = {k: v for k, v in __import__("os").environ.items() if k != "APPLE_TEAM_ID"}
        result = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "distribution" / "render_mas_entitlements.py")],
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("APPLE_TEAM_ID", result.stderr)

    def test_renders_team_id_and_the_real_bundle_identifier(self):
        rendered_path = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "entitlements.mas.plist"
        real_identifier = json.loads(
            (REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json").read_text()
        )["identifier"]
        env = dict(__import__("os").environ, APPLE_TEAM_ID="TESTTEAM01")
        result = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "distribution" / "render_mas_entitlements.py")],
            capture_output=True,
            text=True,
            env=env,
        )
        try:
            self.assertEqual(result.returncode, 0, result.stderr)
            with rendered_path.open("rb") as f:
                entitlements = plistlib.load(f)
            self.assertEqual(entitlements["com.apple.developer.team-identifier"], "TESTTEAM01")
            self.assertEqual(
                entitlements["com.apple.application-identifier"],
                f"TESTTEAM01.{real_identifier}",
            )
        finally:
            rendered_path.unlink(missing_ok=True)


class PackageMasEntitlementValidationTests(unittest.TestCase):
    VALID = {
        "com.apple.security.app-sandbox": True,
        "com.apple.security.files.user-selected.read-write": True,
    }

    def test_valid_minimal_set_passes(self):
        package_mas.validate_entitlements_dict(dict(self.VALID), source="test")

    def test_missing_app_sandbox_is_rejected(self):
        entitlements = {"com.apple.security.files.user-selected.read-write": True}
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_app_sandbox_false_is_rejected(self):
        entitlements = dict(self.VALID, **{"com.apple.security.app-sandbox": False})
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_missing_file_access_entitlement_is_rejected(self):
        entitlements = {"com.apple.security.app-sandbox": True}
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_forbidden_network_entitlement_is_rejected_even_alongside_valid_ones(self):
        entitlements = dict(self.VALID, **{"com.apple.security.network.client": True})
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_forbidden_automation_entitlement_is_rejected(self):
        entitlements = dict(self.VALID, **{"com.apple.security.automation.apple-events": True})
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_forbidden_temporary_exception_entitlement_is_rejected(self):
        entitlements = dict(
            self.VALID,
            **{"com.apple.security.temporary-exception.files.absolute-path.read-write": ["/"]},
        )
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_find_built_app_fails_closed_when_none_or_multiple_exist(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            macos_dir = tmp_path / "target" / "aarch64-apple-darwin" / "release" / "bundle" / "macos"
            macos_dir.mkdir(parents=True)
            original_root = package_mas.REPO_ROOT
            package_mas.REPO_ROOT = tmp_path
            try:
                with self.assertRaises(SystemExit):
                    package_mas.find_built_app("aarch64-apple-darwin")
                (macos_dir / "A.app").mkdir()
                (macos_dir / "B.app").mkdir()
                with self.assertRaises(SystemExit):
                    package_mas.find_built_app("aarch64-apple-darwin")
            finally:
                package_mas.REPO_ROOT = original_root


class PackageMasNeverAdHocSignsGuardTests(unittest.TestCase):
    """A Mac App Store artifact must never be produced through the same
    ad-hoc structural-signing fallback the GitHub distribution path uses
    for unsigned builds (see `sign_macos_app.py`) — that fallback fixes
    a different problem (Gatekeeper's "damaged" bundle check on an
    *unsigned* build) and is not a substitute for real Apple Distribution
    signing on a submission artifact."""

    PACKAGE_MAS_SOURCE = (
        REPO_ROOT / "scripts" / "distribution" / "package_mas.py"
    ).read_text()

    def test_package_mas_never_imports_the_ad_hoc_signing_script(self):
        self.assertNotIn("import sign_macos_app", self.PACKAGE_MAS_SOURCE)

    def test_package_mas_has_no_ad_hoc_identity_default(self):
        self.assertNotIn('"-"', self.PACKAGE_MAS_SOURCE)
        self.assertNotIn("'-'", self.PACKAGE_MAS_SOURCE)

    def test_sign_required_without_credentials_fails_before_any_build_step(self):
        env = {k: v for k, v in __import__("os").environ.items() if k != "APPLE_SIGNING_IDENTITY"}
        result = subprocess.run(
            [
                sys.executable, str(REPO_ROOT / "scripts" / "distribution" / "package_mas.py"),
                "--sign", "--target-triple", "aarch64-apple-darwin",
                "--version", "0.1.0", "--out-dir", "/tmp/should-not-be-created-mas-out",
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("APPLE_SIGNING_IDENTITY", result.stderr)
        self.assertIn("never falls back to ad-hoc", result.stderr)
        self.assertFalse(Path("/tmp/should-not-be-created-mas-out").exists())


class MacosSigningAndPackagingGuardTests(unittest.TestCase):
    """Only the pre-subprocess argument guards — `codesign` and
    `hdiutil` are macOS-only, and this file also runs on Linux (the
    `version-check` job), so the actual signing/packaging behavior is
    exercised on real macOS CI runners instead (see the workflow's
    "Verify macOS app bundle signature" step), not here.
    """

    def test_sign_macos_app_rejects_a_non_app_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            not_an_app = Path(tmp) / "not-an-app"
            not_an_app.mkdir()
            with self.assertRaises(SystemExit):
                sign_macos_app.sign(not_an_app, "-")

    def test_package_macos_dmg_rejects_a_non_app_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            not_an_app = tmp_path / "not-an-app"
            not_an_app.mkdir()
            with self.assertRaises(SystemExit):
                package_macos_dmg.build_dmg(
                    not_an_app, "0.1.0", "aarch64-apple-darwin", tmp_path / "out"
                )


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
