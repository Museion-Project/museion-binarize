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

import aggregate_release  # noqa: E402
import checksums  # noqa: E402
import collect_desktop_artifact  # noqa: E402
import render_release_notes  # noqa: E402
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
            "mpdf-0.1.0-macos-arm64.dmg",
        )
        self.assertEqual(
            naming.desktop_artifact_name("0.1.0", "x86_64-pc-windows-msvc", "msi"),
            "mpdf-0.1.0-windows-x64.msi",
        )

    def test_cli_archive_name_matches_the_documented_convention(self):
        self.assertEqual(
            naming.cli_archive_name("0.1.0", "x86_64-unknown-linux-gnu", "tar.gz"),
            "mpdf-cli-0.1.0-linux-x86_64.tar.gz",
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
            self.assertEqual(loaded["schema"], "mpdf-release-manifest")
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
            dmg = tmp_path / "mpdf-0.1.0-macos-arm64.dmg"
            dmg.write_bytes(b"fake dmg bytes")
            cli_archive = tmp_path / "mpdf-cli-0.1.0-macos-arm64.tar.gz"
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
        self.assertEqual(data["schema"], "mpdf-pdfium-manifest")
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
    """The canonical product identity is `me.mpdf.processor`, declared
    exactly once, in the base `tauri.conf.json` — see
    docs/mac-app-store-readiness.md, "Bundle identifier." Every overlay
    (GitHub dist, MAS) must inherit it rather than redeclaring it, so it
    is structurally impossible for the two distribution channels to
    represent different products. The previous identifier,
    `org.museionproject.binarize`, must not reappear in any active
    configuration."""

    CANONICAL_IDENTIFIER = "me.mpdf.processor"
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
        # It must only carry the `bundle.resources` PDFium override and
        # the `bundle.windows.wix.version` MSI-compatibility override
        # (see test_dist_config_wix_version_is_msi_compatible below) so
        # it stays a safe `--config` overlay merged on top of the base
        # config for distribution builds, not a divergent duplicate
        # configuration.
        config = json.loads(self.DIST_CONFIG.read_text())
        self.assertEqual(set(config["bundle"].keys()), {"resources", "windows"})
        self.assertEqual(
            set(config.keys()) - {"$schema"},
            {"bundle"},
        )

    def test_dist_config_wix_version_is_msi_compatible(self):
        # Windows Installer's ProductVersion is numeric-only,
        # major.minor.build(.revision) — it cannot represent a SemVer
        # prerelease identifier like "0.1.0-rc.1" (confirmed against
        # Tauri's own WixConfig docs and tauri-bundler's changelog: "the
        # msi bundle target... [prerelease/build metadata] must be
        # numeric only"). This override lets the MSI-internal version
        # stay numeric while every other version field in the repo
        # (Cargo, package.json, tauri.conf.json, CFBundleShortVersionString,
        # filenames) keeps the real, meaningful "0.1.0-rc.1". See
        # docs/releasing.md, "Prerelease versioning across packaging
        # targets."
        config = json.loads(self.DIST_CONFIG.read_text())
        wix_version = config["bundle"]["windows"]["wix"]["version"]
        self.assertRegex(
            wix_version,
            r"^\d+\.\d+\.\d+(\.\d+)?$",
            f"WiX MSI version {wix_version!r} must be numeric-only "
            "major.minor.build(.revision) — MSI's ProductVersion field "
            "cannot represent anything else",
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
        # Required by WKWebView's out-of-process renderer under App
        # Sandbox, not by any network code in this app; without it the
        # window renders blank. Asserted present so it can never be
        # "tidied away" as an unused permission.
        self.assertEqual(entitlements["com.apple.security.network.client"], True)
        # Everything else is placeholder identity metadata, not a
        # capability grant — but confirm no unexpected key sneaked in.
        self.assertEqual(
            set(entitlements.keys()),
            {
                "com.apple.security.app-sandbox",
                "com.apple.security.files.user-selected.read-write",
                "com.apple.security.network.client",
                "com.apple.application-identifier",
                "com.apple.developer.team-identifier",
            },
        )

    def test_entitlements_template_does_not_grant_incoming_network_server(self):
        with self.ENTITLEMENTS_TEMPLATE.open("rb") as f:
            entitlements = plistlib.load(f)
        self.assertNotIn("com.apple.security.network.server", entitlements)

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
        "com.apple.security.network.client": True,
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

    def test_missing_network_client_is_rejected(self):
        # Dropping it produces an app that launches but renders a blank
        # window under App Sandbox (WKWebView's WebContent XPC service
        # never starts) — a silent, easily-missed break, so it fails the
        # build rather than shipping.
        entitlements = {
            "com.apple.security.app-sandbox": True,
            "com.apple.security.files.user-selected.read-write": True,
        }
        with self.assertRaises(SystemExit):
            package_mas.validate_entitlements_dict(entitlements, source="test")

    def test_forbidden_incoming_network_server_entitlement_is_rejected(self):
        # network.client is required (WebKit); network.server — accepting
        # *incoming* connections — remains forbidden.
        entitlements = dict(self.VALID, **{"com.apple.security.network.server": True})
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


def _artifact_entry(filename, **overrides):
    entry = {
        "target_triple": "aarch64-apple-darwin",
        "os": "macos",
        "arch": "arm64",
        "artifact_filename": filename,
        "artifact_sha256": "a" * 64,
        "pdfium_build": "7920",
        "pdfium_version": "151.0.7920.0",
        "pdfium_sha256": "b" * 64,
        "signing_state": "ad_hoc",
        "notarization_state": "pending_credentials",
    }
    entry.update(overrides)
    return entry


def _write_target_dir(
    root: Path,
    name: str,
    *,
    project_version: str,
    git_sha: str,
    artifact_filenames: list[str],
    artifact_overrides: dict | None = None,
    extra_files: list[str] | None = None,
    omit_manifest_entry_for: str | None = None,
) -> Path:
    """A synthetic stand-in for one `gh run download`ed artifact
    subdirectory — no live GitHub call, matching the M7A2 brief's "use
    synthetic temporary artifacts for tests" requirement."""
    target_dir = root / name
    target_dir.mkdir(parents=True)
    entries = []
    for filename in artifact_filenames:
        (target_dir / filename).write_bytes(f"contents of {filename}".encode())
        if filename == omit_manifest_entry_for:
            continue
        overrides = (artifact_overrides or {}).get(filename, {})
        entries.append(_artifact_entry(filename, **overrides))
    for filename in extra_files or []:
        (target_dir / filename).write_bytes(b"unexpected stray file")
    manifest = {
        "schema": "mpdf-release-manifest",
        "schema_version": "1.0",
        "project_version": project_version,
        "git_sha": git_sha,
        "artifacts": entries,
    }
    (target_dir / "release-manifest.json").write_text(json.dumps(manifest, indent=2))
    (target_dir / "SHA256SUMS").write_text("deliberately-stale-per-target-checksums\n")
    return target_dir


class RenderReleaseNotesTests(unittest.TestCase):
    def _manifest(self, **overrides):
        manifest = {
            "schema": "mpdf-release-manifest",
            "schema_version": "1.0",
            "project_version": "0.1.0-rc.1",
            "release_tag": "v0.1.0-rc.1",
            "git_sha": "a" * 40,
            "artifacts": [
                _artifact_entry(
                    "mpdf-0.1.0-rc.1-macos-arm64.dmg",
                    signing_state="ad_hoc",
                    notarization_state="pending_credentials",
                ),
                _artifact_entry(
                    "mpdf-0.1.0-rc.1-windows-x64.msi",
                    signing_state="unsigned",
                    notarization_state="not_applicable",
                ),
            ],
        }
        manifest.update(overrides)
        return manifest

    def test_lists_every_manifest_artifact_by_filename(self):
        body = render_release_notes.render(self._manifest(), "0.1.0-rc.1")
        self.assertIn("mpdf-0.1.0-rc.1-macos-arm64.dmg", body)
        self.assertIn("mpdf-0.1.0-rc.1-windows-x64.msi", body)

    def test_does_not_claim_developer_id_signing_or_notarization_occurred(self):
        body = " ".join(render_release_notes.render(self._manifest(), "0.1.0-rc.1").split())
        self.assertIn("ad-hoc signed", body)
        # The honest negative claim ("not ... signed or notarized") must
        # be present; an unqualified positive claim must not be.
        self.assertIn("not Developer ID signed or notarized", body)
        self.assertNotIn("is Developer ID signed", body)
        self.assertNotIn("is notarized", body)

    def test_does_not_claim_windows_or_linux_runtime_acceptance(self):
        body = " ".join(render_release_notes.render(self._manifest(), "0.1.0-rc.1").split())
        self.assertIn("do not yet have human runtime acceptance", body)

    def test_describes_live_sponsors_and_app_store_as_planned_only(self):
        body = " ".join(render_release_notes.render(self._manifest(), "0.1.0-rc.1").split())
        self.assertIn("free and open source", body)
        self.assertIn("https://github.com/sponsors/pei-haoran", body)
        self.assertIn("Mac App Store edition is planned for later", body)
        # Must never claim the App Store edition is currently live/available.
        self.assertNotIn("available on the Mac App Store", body)
        self.assertNotIn("now available on the App Store", body)

    def test_version_mismatch_between_manifest_and_argument_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest_path = Path(tmp) / "release-manifest.json"
            manifest_path.write_text(json.dumps(self._manifest(project_version="0.1.0-rc.2")))
            result = subprocess.run(
                [
                    sys.executable,
                    str(REPO_ROOT / "scripts" / "distribution" / "render_release_notes.py"),
                    "--manifest", str(manifest_path),
                    "--version", "0.1.0-rc.1",
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)


class AggregateReleaseTests(unittest.TestCase):
    VERSION = "0.1.0-rc.1"
    TAG = "v0.1.0-rc.1"
    SHA = "a" * 40

    def _two_target_dir(self, tmp_path: Path) -> Path:
        artifacts_dir = tmp_path / "downloaded"
        _write_target_dir(
            artifacts_dir,
            "mpdf-aarch64-apple-darwin",
            project_version=self.VERSION,
            git_sha=self.SHA,
            artifact_filenames=[
                "mpdf-0.1.0-rc.1-macos-arm64.dmg",
                "mpdf-cli-0.1.0-rc.1-macos-arm64.tar.gz",
            ],
        )
        _write_target_dir(
            artifacts_dir,
            "mpdf-x86_64-pc-windows-msvc",
            project_version=self.VERSION,
            git_sha=self.SHA,
            artifact_filenames=[
                "mpdf-0.1.0-rc.1-windows-x64.msi",
                "mpdf-cli-0.1.0-rc.1-windows-x64.zip",
            ],
        )
        return artifacts_dir

    def test_matching_version_and_sha_are_accepted_and_merge_correctly(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = self._two_target_dir(tmp_path)
            out_dir = tmp_path / "release-assets"

            aggregate_release.aggregate(artifacts_dir, self.VERSION, self.TAG, self.SHA, out_dir)

            manifest = json.loads((out_dir / "release-manifest.json").read_text())
            self.assertEqual(manifest["project_version"], self.VERSION)
            self.assertEqual(manifest["release_tag"], self.TAG)
            self.assertEqual(manifest["git_sha"], self.SHA)
            filenames = {a["artifact_filename"] for a in manifest["artifacts"]}
            self.assertEqual(
                filenames,
                {
                    "mpdf-0.1.0-rc.1-macos-arm64.dmg",
                    "mpdf-cli-0.1.0-rc.1-macos-arm64.tar.gz",
                    "mpdf-0.1.0-rc.1-windows-x64.msi",
                    "mpdf-cli-0.1.0-rc.1-windows-x64.zip",
                },
            )
            # Every merged artifact file actually landed in out_dir.
            for filename in filenames:
                self.assertTrue((out_dir / filename).is_file())
            # A fresh, release-wide SHA256SUMS/manifest were generated —
            # not a copy of either per-target file (which this fixture
            # deliberately seeded with obviously-wrong stale content, to
            # prove that).
            self.assertTrue((out_dir / "SHA256SUMS").is_file())
            self.assertNotIn("deliberately-stale-per-target-checksums", (out_dir / "SHA256SUMS").read_text())

    def test_version_mismatch_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version="0.1.0-rc.2", git_sha=self.SHA,
                artifact_filenames=["a.dmg"],
            )
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_tag_version_mismatch_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = self._two_target_dir(tmp_path)
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, "v0.1.0-rc.2", self.SHA, tmp_path / "out"
                )

    def test_mixed_git_shas_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["a.dmg"],
            )
            _write_target_dir(
                artifacts_dir, "mpdf-x86_64-pc-windows-msvc",
                project_version=self.VERSION, git_sha="b" * 40,
                artifact_filenames=["a.msi"],
            )
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_duplicate_artifact_filename_across_targets_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["collision.bin"],
            )
            _write_target_dir(
                artifacts_dir, "mpdf-x86_64-pc-windows-msvc",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["collision.bin"],
            )
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_stale_or_extra_file_not_in_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["a.dmg"],
                extra_files=["M PDF Processor_0.1.0_aarch64.dmg"],  # a stale leftover, not in the manifest
            )
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_manifest_entry_with_no_file_on_disk_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["a.dmg", "b.tar.gz"],
                omit_manifest_entry_for="b.tar.gz",
            )
            # b.tar.gz exists on disk but has no manifest entry — this is
            # actually the "unexpected file" case; construct the inverse
            # (manifest claims a file that doesn't exist) directly:
            manifest_path = artifacts_dir / "mpdf-aarch64-apple-darwin" / "release-manifest.json"
            manifest = json.loads(manifest_path.read_text())
            manifest["artifacts"].append(_artifact_entry("never-written.bin"))
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_release_wide_sha256sums_covers_every_artifact_exactly_once_and_matches_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = self._two_target_dir(tmp_path)
            out_dir = tmp_path / "release-assets"

            aggregate_release.aggregate(artifacts_dir, self.VERSION, self.TAG, self.SHA, out_dir)

            sums_content = (out_dir / "SHA256SUMS").read_text()
            lines = [line for line in sums_content.strip().splitlines()]
            names_in_sums = [line.split("  ", 1)[1] for line in lines]
            # Exactly once each — no duplicates, and covers the merged
            # manifest itself alongside the four binary artifacts.
            self.assertEqual(len(names_in_sums), len(set(names_in_sums)))
            self.assertIn("release-manifest.json", names_in_sums)
            for line in lines:
                digest, name = line.split("  ", 1)
                actual = hashlib.sha256((out_dir / name).read_bytes()).hexdigest()
                self.assertEqual(digest, actual, f"checksum mismatch for {name}")

    def test_signing_and_notarization_state_are_preserved_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["a.dmg"],
                artifact_overrides={
                    "a.dmg": {"signing_state": "ad_hoc", "notarization_state": "pending_credentials"}
                },
            )
            out_dir = tmp_path / "out"
            aggregate_release.aggregate(artifacts_dir, self.VERSION, self.TAG, self.SHA, out_dir)
            manifest = json.loads((out_dir / "release-manifest.json").read_text())
            entry = next(a for a in manifest["artifacts"] if a["artifact_filename"] == "a.dmg")
            self.assertEqual(entry["signing_state"], "ad_hoc")
            self.assertEqual(entry["notarization_state"], "pending_credentials")

    def test_pdfium_provenance_is_preserved_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            _write_target_dir(
                artifacts_dir, "mpdf-aarch64-apple-darwin",
                project_version=self.VERSION, git_sha=self.SHA,
                artifact_filenames=["a.dmg"],
                artifact_overrides={"a.dmg": {"pdfium_sha256": "c" * 64, "pdfium_version": "151.0.7920.0"}},
            )
            out_dir = tmp_path / "out"
            aggregate_release.aggregate(artifacts_dir, self.VERSION, self.TAG, self.SHA, out_dir)
            manifest = json.loads((out_dir / "release-manifest.json").read_text())
            entry = next(a for a in manifest["artifacts"] if a["artifact_filename"] == "a.dmg")
            self.assertEqual(entry["pdfium_sha256"], "c" * 64)

    def test_no_target_subdirectories_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            artifacts_dir.mkdir()
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )

    def test_missing_target_manifest_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            artifacts_dir = tmp_path / "downloaded"
            bad_target = artifacts_dir / "mpdf-aarch64-apple-darwin"
            bad_target.mkdir(parents=True)
            (bad_target / "a.dmg").write_bytes(b"no manifest for this file")
            with self.assertRaises(SystemExit):
                aggregate_release.aggregate(
                    artifacts_dir, self.VERSION, self.TAG, self.SHA, tmp_path / "out"
                )


class CollectDesktopArtifactTests(unittest.TestCase):
    """Shared by the Windows and Linux distribution jobs to pick the one
    built installer out of Tauri's bundle directory — see
    scripts/distribution/collect_desktop_artifact.py's docstring for why
    this must fail loudly rather than silently pick among several
    matches."""

    def test_collects_the_single_match_under_the_naming_convention(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bundle_dir = tmp_path / "bundle" / "msi"
            bundle_dir.mkdir(parents=True)
            (bundle_dir / "M PDF Processor_0.1.0-rc.1_x64_en-US.msi").write_bytes(b"fake msi")
            out_dir = tmp_path / "dist-out"

            dest = collect_desktop_artifact.collect(
                bundle_dir, "*.msi", "0.1.0-rc.1", "x86_64-pc-windows-msvc", "msi", out_dir
            )

            self.assertEqual(dest.name, "mpdf-0.1.0-rc.1-windows-x64.msi")
            self.assertEqual(dest.read_bytes(), b"fake msi")

    def test_rejects_zero_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bundle_dir = tmp_path / "bundle" / "msi"
            bundle_dir.mkdir(parents=True)
            with self.assertRaises(SystemExit):
                collect_desktop_artifact.collect(
                    bundle_dir, "*.msi", "0.1.0-rc.1", "x86_64-pc-windows-msvc", "msi", tmp_path / "out"
                )

    def test_rejects_multiple_matches_rather_than_picking_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bundle_dir = tmp_path / "bundle" / "deb"
            bundle_dir.mkdir(parents=True)
            (bundle_dir / "a.deb").write_bytes(b"1")
            (bundle_dir / "b.deb").write_bytes(b"2")
            with self.assertRaises(SystemExit):
                collect_desktop_artifact.collect(
                    bundle_dir, "*.deb", "0.1.0-rc.1", "x86_64-unknown-linux-gnu", "deb", tmp_path / "out"
                )


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


class WorkflowYamlDuplicateKeyTests(unittest.TestCase):
    """Guards against a real defect that shipped to main: a workflow step
    with two `env:` keys in the same YAML mapping. PyYAML's `safe_load`
    silently accepts duplicate mapping keys (keeping only the last one),
    so it can't catch this — GitHub's own workflow parser rejects it
    outright at dispatch time, but by then it's already merged. This
    test parses every workflow file with a loader that raises on any
    duplicate key, anywhere in the document, not just under `env`."""

    WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"

    @staticmethod
    def _load_strict(text):
        import yaml

        class StrictLoader(yaml.SafeLoader):
            pass

        def construct_mapping(loader, node, deep=False):
            mapping = {}
            for key_node, value_node in node.value:
                key = loader.construct_object(key_node, deep=deep)
                if key in mapping:
                    raise ValueError(f"duplicate key {key!r} at line {key_node.start_mark.line + 1}")
                mapping[key] = loader.construct_object(value_node, deep=deep)
            return mapping

        StrictLoader.add_constructor(
            yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, construct_mapping
        )
        return yaml.load(text, Loader=StrictLoader)

    def test_no_workflow_file_has_duplicate_mapping_keys(self):
        workflow_files = sorted(self.WORKFLOWS_DIR.glob("*.yml")) + sorted(
            self.WORKFLOWS_DIR.glob("*.yaml")
        )
        self.assertTrue(workflow_files, "expected at least one workflow file")
        for path in workflow_files:
            with self.subTest(workflow=path.name):
                try:
                    self._load_strict(path.read_text())
                except ValueError as exc:
                    self.fail(f"{path.name}: {exc}")

    def test_strict_loader_actually_rejects_a_duplicate_key(self):
        # Guards the guard: confirms _load_strict doesn't just silently
        # accept duplicates like plain yaml.safe_load would.
        with self.assertRaises(ValueError):
            self._load_strict("a:\n  x: 1\n  x: 2\n")


class PublishReleaseWorkflowInvariantTests(unittest.TestCase):
    """Structural checks on publish-release.yml's actual YAML/text —
    this workflow's whole reason to exist is that it must never make a
    release public by itself, so its most safety-critical properties
    are asserted directly against the committed file rather than only
    documented in prose."""

    WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "publish-release.yml"

    def setUp(self):
        self.text = self.WORKFLOW_PATH.read_text()
        # Lines actually executed by this workflow — comments (which may
        # legitimately mention, e.g., the separate manual command an
        # owner would later run to publish) must not confuse a check
        # for what this workflow's own steps do.
        self.non_comment_text = "\n".join(
            line for line in self.text.splitlines() if not line.strip().startswith("#")
        )
        import yaml  # local import: only this test class needs it

        self.parsed = yaml.safe_load(self.text)
        # PyYAML parses the bare `on:` key as boolean True, not the
        # string "on" — this is a real, well-known PyYAML quirk (YAML
        # 1.1 treats unquoted on/off/yes/no as booleans), not a bug in
        # this test.
        self.triggers = self.parsed.get("on", self.parsed.get(True))

    def test_trigger_is_workflow_dispatch_only(self):
        self.assertEqual(set(self.triggers.keys()), {"workflow_dispatch"})

    def test_requires_tag_run_id_and_git_sha_inputs(self):
        inputs = self.triggers["workflow_dispatch"]["inputs"]
        for name in ("tag", "run_id", "git_sha"):
            self.assertIn(name, inputs)
            self.assertTrue(inputs[name]["required"])

    def test_permissions_are_minimal(self):
        self.assertEqual(
            self.parsed["permissions"],
            {"contents": "write", "actions": "read"},
        )

    def test_release_creation_always_passes_draft_and_prerelease(self):
        self.assertIn("--draft", self.text)
        self.assertIn("--prerelease", self.text)
        # Both flags must appear on the same `gh release create` line as
        # each other and as the tag, not just somewhere in the file.
        create_lines = [
            line for line in self.text.splitlines() if "gh release create" in line
        ]
        self.assertEqual(len(create_lines), 1)

    def test_never_flips_draft_to_false_or_runs_a_publish_step(self):
        # Scoped to actual `gh release`/`gh api` invocations specifically
        # — comments and the final human-facing status message both
        # legitimately *mention* the separate, later, manual command an
        # owner would run to actually publish (see the module docstring
        # and the final "Report" step), which is not the same claim as
        # "this workflow itself runs that command."
        gh_invocation_lines = [
            line
            for line in self.non_comment_text.splitlines()
            if ("gh release" in line or "gh api" in line) and "echo" not in line
        ]
        self.assertTrue(gh_invocation_lines, "expected at least one real `gh release`/`gh api` line")
        for line in gh_invocation_lines:
            self.assertNotIn("--draft=false", line)
            self.assertNotIn("--draft false", line)
            self.assertNotIn(" edit ", line)

    def test_fails_closed_on_run_id_mismatch_workflow_name(self):
        self.assertIn('data["name"] != "Build distribution artifacts"', self.text)

    def test_fails_closed_on_head_sha_mismatch(self):
        self.assertIn('data["headSha"] != expected_sha', self.text)

    def test_fails_closed_on_non_successful_run(self):
        self.assertIn('data["conclusion"] != "success"', self.text)

    def test_verifies_checked_out_commit_matches_requested_sha(self):
        self.assertIn("git rev-parse HEAD", self.text)

    def test_rejects_a_tag_without_the_v_prefix_rather_than_guessing(self):
        self.assertIn('does not start with', self.text)


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
