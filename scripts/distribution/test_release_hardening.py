#!/usr/bin/env python3
"""Network-free tests for rc.3 release-hardening primitives."""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import generate_sbom  # noqa: E402
import release_manifest  # noqa: E402
import sign_macos_release  # noqa: E402
import render_release_notes  # noqa: E402
import release_readiness  # noqa: E402
import verify_ocr_runtime  # noqa: E402
import stage_ocr_runtime  # noqa: E402
import ocr_runtime_smoke  # noqa: E402

ROOT = HERE.parents[1]


PDFIUM = {"asset": [{"target_triple": "aarch64-apple-darwin", "version": "151.0.7920.0", "build": "7920", "library_sha256": "a" * 64, "archive_url": "https://example.invalid/pdfium.tgz"}]}


def write_base_install_fixture(path: Path) -> Path:
    """Derive fixture-shaped base evidence without rewriting the audit record."""
    source = ROOT / "docs/evidence/rc3-local-install-arm64.json"
    evidence = json.loads(source.read_text())
    evidence["distribution_profile"] = "base"
    evidence.pop("bundled_ocr", None)
    path.write_text(json.dumps(evidence))
    return path


class HardeningTests(unittest.TestCase):
    def test_legacy_combined_install_evidence_cannot_prove_base(self):
        evidence = HERE.parents[1] / "docs/evidence/rc3-local-install-arm64.json"
        self.assertFalse(release_readiness.validate_macos_install_evidence(evidence))

    def test_base_install_evidence_is_explicit_and_valid(self):
        with tempfile.TemporaryDirectory() as tmp:
            evidence = write_base_install_fixture(Path(tmp) / "base-install.json")
            self.assertTrue(release_readiness.validate_macos_install_evidence(evidence))

    def test_sbom_is_deterministic_and_contains_transitive_fixture_and_pdfium(self):
        metadata = {"packages": [
            {"id": "path+file:///one/crates/foo", "name": "foo", "version": "1.2.3", "license": "MIT", "manifest_path": "/one/crates/foo/Cargo.toml"},
            {"id": "registry+https://example/crates/bar#2.0.0", "name": "bar", "version": "2.0.0", "license": "Apache-2.0", "manifest_path": "/cargo/bar/Cargo.toml"},
        ]}
        metadata["resolve"] = {"nodes": [{"id": metadata["packages"][0]["id"], "dependencies": [{"pkg": metadata["packages"][1]["id"]}]}]}
        graph = [{"name": "react", "version": "19.0.0", "license": "MIT", "downloadLocation": "https://registry.npmjs.org/react/-/react-19.0.0.tgz", "dependencies": {"bar": "^2"}}, {"name": "bar", "version": "2.0.0", "license": "Apache-2.0", "dependencies": {}}]
        one = generate_sbom.build_sbom(project_version="0.1.0-rc.3", target="aarch64-apple-darwin", cargo_metadata=metadata, pnpm_lock=None, node_graph=graph, pdfium_manifest=PDFIUM, created="2026-08-28T00:00:00Z")
        two = generate_sbom.build_sbom(project_version="0.1.0-rc.3", target="aarch64-apple-darwin", cargo_metadata={**metadata, "workspace_root": "/different"}, pnpm_lock=None, node_graph=graph, pdfium_manifest=PDFIUM, created="2026-08-28T00:00:00Z")
        self.assertEqual(json.dumps(one, sort_keys=True), json.dumps(two, sort_keys=True))
        self.assertTrue(any(p["name"] == "bar" for p in one["packages"]))
        pdfium = next(p for p in one["packages"] if p["name"] == "PDFium")
        self.assertEqual(pdfium["checksums"][0]["checksumValue"], "a" * 64)
        self.assertEqual(one["creationInfo"]["created"], "2026-08-28T00:00:00Z")
        later = generate_sbom.build_sbom(project_version="0.1.0-rc.3", target="aarch64-apple-darwin", cargo_metadata=metadata, pnpm_lock=None, node_graph=graph, pdfium_manifest=PDFIUM, created="2026-08-29T00:00:00Z")
        self.assertNotEqual(one["documentNamespace"], later["documentNamespace"])
        self.assertTrue(any(r["relationshipType"] == "DEPENDS_ON" and r["spdxElementId"].startswith("SPDXRef-Node") for r in one["relationships"]))
        self.assertEqual(next(p for p in one["packages"] if p["name"] == "react")["licenseDeclared"], "MIT")
        blob = json.dumps(one)
        self.assertNotIn("RapidOCR", blob)
        self.assertNotIn("/Users/", blob)

    def test_sbom_schema_rejects_duplicate_ids_and_dangling_relationships(self):
        sbom = generate_sbom.build_sbom(
            project_version="0.1.0-rc.3", target="aarch64-apple-darwin",
            cargo_metadata={"packages": []}, pnpm_lock=None,
            node_graph=[{"name": "pkg", "version": "1.0.0", "license": "MIT",
                         "repository": "/Users/private/project", "dependencies": {}}],
            pdfium_manifest=PDFIUM, created="2026-08-28T00:00:00Z",
        )
        generate_sbom.validate_sbom(sbom)
        self.assertEqual(next(p for p in sbom["packages"] if p["name"] == "pkg")["downloadLocation"], "NOASSERTION")
        duplicate = json.loads(json.dumps(sbom))
        duplicate["packages"][0]["SPDXID"] = duplicate["packages"][1]["SPDXID"]
        with self.assertRaises(ValueError):
            generate_sbom.validate_sbom(duplicate)
        dangling = json.loads(json.dumps(sbom))
        dangling["relationships"].append({"spdxElementId": "SPDXRef-DOCUMENT",
                                             "relationshipType": "DEPENDS_ON",
                                             "relatedSpdxElement": "SPDXRef-Missing"})
        with self.assertRaises(ValueError):
            generate_sbom.validate_sbom(dangling)

    def test_manifest_kind_constraints_and_v10_read(self):
        old = {"schema": "mpdf-release-manifest", "schema_version": "1.0", "artifacts": [{"artifact_filename": "a.dmg", "signing_state": "ad_hoc", "notarization_state": "pending_credentials"}]}
        release_manifest.validate_manifest(old)
        with self.assertRaises(ValueError):
            release_manifest.build_entry(target_triple="x", os_name="linux", arch="x", artifact_filename="a.json", artifact_sha256="a" * 64, pdfium_build="1", pdfium_version="1", pdfium_sha256="a" * 64, signing_state="signed", notarization_state="not_applicable", artifact_kind="sbom")

    def test_signing_preflight_rejects_partial_credentials(self):
        with self.assertRaises(ValueError):
            sign_macos_release.credential_mode({"APPLE_CERTIFICATE": "present"})
        self.assertEqual(sign_macos_release.credential_mode({}), "ad_hoc")

    def test_signing_command_sequence_and_notary_acceptance(self):
        commands = []
        def fake(command, **kwargs):
            commands.append(command)
            stdout = '"/Users/test/login.keychain-db"\n' if command[:3] == ["security", "list-keychains", "-d"] and "-s" not in command else ""
            if command[:3] == ["xcrun", "notarytool", "submit"]:
                stdout = '{"status":"Accepted"}'
            return subprocess.CompletedProcess(command, 0, stdout=stdout)
        env = {name: "present" for name in sign_macos_release.ALL_SECRETS}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            marker = root / "marker.json"
            dmg = root / "final.dmg"
            dmg.write_bytes(b"dmg")
            app = root / "M PDF Processor.app"
            resource_dir = app / "Contents" / "Resources"
            resource_dir.mkdir(parents=True)
            (resource_dir / "libpdfium.dylib").write_bytes(b"\xcf\xfa\xed\xfe" + b"test")
            sign_macos_release.sign_app(app, marker=marker, env=env, runner=fake)
            sign_macos_release.notarize_dmg(dmg, marker=marker, env=env, runner=fake)
            marker_state = json.loads(marker.read_text())
        joined = [" ".join(c) for c in commands]
        self.assertTrue(any("codesign" in c and "--options runtime" in c and "--timestamp" in c for c in joined))
        nested_sign = next(c for c in commands if c[:2] == ["codesign", "--force"] and c[-1].endswith("libpdfium.dylib"))
        app_sign = next(c for c in commands if c[:2] == ["codesign", "--force"] and c[-1].endswith(".app"))
        self.assertNotIn("--deep", nested_sign)
        self.assertIn("--deep", app_sign)
        self.assertLess(commands.index(nested_sign), commands.index(app_sign))
        self.assertTrue(any("set-key-partition-list" in c and "apple-tool:,apple:" in c for c in joined))
        self.assertTrue(any("notarytool submit" in c and "--wait" in c for c in joined))
        self.assertTrue(any("--output-format json" in c for c in joined))
        self.assertEqual(marker_state["notarization_state"], "notarized")

    def test_ad_hoc_does_not_use_developer_flags_and_rejected_notary_preserves_marker(self):
        commands = []
        def fake(command, **kwargs):
            commands.append(command)
            stdout = '{"status":"Rejected"}' if command[:3] == ["xcrun", "notarytool", "submit"] else ""
            return subprocess.CompletedProcess(command, 0, stdout=stdout)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); app = root / "M PDF Processor.app"; app.mkdir(); marker = root / "marker.json"; dmg = root / "x.dmg"; dmg.write_bytes(b"x")
            sign_macos_release.sign_app(app, marker=marker, env={}, runner=fake)
            adhoc_commands = list(commands)
            commands.clear()
            sign_macos_release.sign_app(app, marker=marker, env={name: "present" for name in sign_macos_release.ALL_SECRETS}, runner=fake)
            # Signing is complete, while the test below exercises an
            # unsuccessful notarization and marker preservation.
            before = marker.read_text()
            env = {name: "present" for name in sign_macos_release.ALL_SECRETS}
            with self.assertRaises(RuntimeError):
                sign_macos_release.notarize_dmg(dmg, marker=marker, env=env, runner=fake)
            self.assertEqual(marker.read_text(), before)
        codesign = next(" ".join(c) for c in adhoc_commands if c[:1] == ["codesign"])
        self.assertNotIn("--timestamp", codesign)
        self.assertNotIn("--options", codesign)
        self.assertFalse(any("stapler" in " ".join(c) for c in commands))

    def test_invalid_notary_json_fails_before_stapling(self):
        commands = []
        def fake(command, **kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 0, stdout="not-json")
        with tempfile.TemporaryDirectory() as tmp:
            dmg = Path(tmp) / "x.dmg"; dmg.write_bytes(b"x")
            key = Path(tmp) / "key.p8"; key.write_text("key")
            with self.assertRaises(RuntimeError):
                sign_macos_release.notarize(dmg, key, "id", "issuer", runner=fake)
        self.assertFalse(any("stapler" in " ".join(c) for c in commands))

    def test_signing_tool_failures_do_not_echo_keychain_password(self):
        password = "super-secret-password"
        env = {name: "present" for name in sign_macos_release.ALL_SECRETS}
        env["APPLE_CERTIFICATE_PASSWORD"] = password

        def fake(command, **kwargs):
            if command[:3] == ["security", "list-keychains", "-d"] and "-s" not in command:
                return subprocess.CompletedProcess(command, 0, stdout='"login.keychain-db"\n')
            if command[:2] == ["security", "create-keychain"]:
                raise subprocess.CalledProcessError(1, command)
            return subprocess.CompletedProcess(command, 0, stdout="")

        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "M PDF Processor.app"
            app.mkdir()
            with self.assertRaises(RuntimeError) as raised:
                sign_macos_release.sign_app(app, marker=Path(tmp) / "marker.json", env=env, runner=fake)
        self.assertNotIn(password, str(raised.exception))

    def test_keychain_restore_failure_is_fail_closed_and_delete_follows_restore(self):
        env = {name: "present" for name in sign_macos_release.ALL_SECRETS}
        events = []

        def fake(command, **kwargs):
            if command[:3] == ["security", "list-keychains", "-d"] and "-s" not in command:
                events.append("read")
                return subprocess.CompletedProcess(command, 0, stdout='"login.keychain-db"\n')
            if command[:3] == ["security", "list-keychains", "-d"] and "-s" in command:
                events.append("restore")
                raise subprocess.CalledProcessError(1, command)
            if command[:2] == ["security", "delete-keychain"]:
                events.append("delete")
                return subprocess.CompletedProcess(command, 0)
            return subprocess.CompletedProcess(command, 0, stdout="")

        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "M PDF Processor.app"
            app.mkdir()
            with self.assertRaises(RuntimeError):
                sign_macos_release.sign_app(app, marker=Path(tmp) / "marker.json", env=env, runner=fake)
        self.assertEqual(events[-2:], ["restore", "delete"])

    def test_rc3_release_notes_follow_manifest_states_and_type_sbom(self):
        base = {"project_version": "0.1.0-rc.3", "artifacts": [
            {"artifact_filename": "mpdf-a.dmg", "artifact_kind": "desktop", "os": "macos", "signing_state": "ad_hoc", "notarization_state": "pending_credentials"},
            {"artifact_filename": "mpdf-a.sbom.json", "artifact_kind": "sbom", "os": "macos", "signing_state": "unsigned", "notarization_state": "not_applicable"},
        ]}
        body = render_release_notes.render(base, "0.1.0-rc.3")
        self.assertIn("ad-hoc signed and pending", body)
        self.assertIn("SBOM (not an executable)", body)
        self.assertNotIn("Developer ID signed and notarized.", body)
        base["artifacts"][0]["signing_state"] = "signed"
        base["artifacts"][0]["notarization_state"] = "notarized"
        self.assertIn("Developer ID signed and notarized", render_release_notes.render(base, "0.1.0-rc.3"))


class CredentialLeakScan(unittest.TestCase):
    """No credential-shaped string may exist in the tracked source tree.

    This is the outermost of the several secret canaries in this project: the
    unit tests check redaction, the integration tests check that evidence and
    reports stay clean, and this checks that nobody committed a real key while
    developing against one. It scans tracked files only — a developer's local
    ``.env`` is their business, but it must never become a commit.
    """

    #: Deliberately assembled at runtime so this file does not itself contain
    #: a string that a naive scanner would flag.
    PATTERNS = (
        "AIza" + "Sy",
        "sk-" + "proj-",
        "ya29." + "a0",
    )

    def tracked_files(self):
        listing = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=ROOT,
            check=True,
            capture_output=True,
        )
        for name in listing.stdout.split(b"\0"):
            if not name:
                continue
            path = ROOT / name.decode()
            if path.is_file():
                yield path

    def test_no_tracked_file_contains_a_provider_key(self):
        offenders = []
        for path in self.tracked_files():
            try:
                text = path.read_text(errors="ignore")
            except OSError:
                continue
            for pattern in self.PATTERNS:
                if pattern in text:
                    # A canary in a *test* is the point of the test; anything
                    # else is a leak. The canaries name themselves.
                    if "CANARY" in text or "canary" in text:
                        continue
                    offenders.append(f"{path.relative_to(ROOT)}: {pattern}")
        self.assertEqual(offenders, [], f"credential-shaped strings found: {offenders}")

    def test_the_processing_core_has_no_network_dependency_at_all(self):
        """Local OCR's offline guarantee is structural, not a code review.

        ``mpdf-core`` owns the provider contract, the prompt, the alignment,
        the gates and the credits state machine — but not a socket. If an HTTP
        or TLS crate ever appears in its dependency graph, "local OCR makes no
        network request" stops being something the type system can enforce and
        starts being something someone has to remember.
        """
        tree = subprocess.run(
            ["cargo", "tree", "-p", "mpdf-core", "--edges", "normal", "--prefix", "none"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.lower()
        forbidden = ("reqwest", "hyper", "tokio", "rustls", "native-tls", "openssl",
                     "ureq", "curl", "keyring", "socket2")
        found = sorted({name for name in forbidden if name in tree})
        self.assertEqual(found, [], f"mpdf-core must stay network-free; found {found}")

    def test_built_frontend_assets_contain_no_credential_shaped_string(self):
        """Scan the built bundle when one exists.

        Source files are covered by the scan above; this catches the case
        where a key reached a bundled asset through an import or an inlined
        fixture. Absent a build, there is nothing to scan and nothing to fail.
        """
        dist = ROOT / "apps/desktop/dist"
        if not dist.is_dir():
            self.skipTest("no built frontend bundle in the working tree")
        offenders = []
        for path in dist.rglob("*"):
            if not path.is_file():
                continue
            text = path.read_text(errors="ignore")
            for pattern in CredentialLeakScan.PATTERNS:
                if pattern in text:
                    offenders.append(f"{path.relative_to(ROOT)}: {pattern}")
        self.assertEqual(offenders, [], f"credential-shaped strings in the bundle: {offenders}")

    def test_readiness_reports_the_cloud_blockers_it_cannot_clear(self):
        gates = release_readiness.run()
        blocked = release_readiness.cloud_blockers(gates)
        self.assertIn("mpdf_credits_complete_ocr_backend", blocked)
        self.assertIn("cloud_privacy_policy_and_deletion", blocked)
        self.assertEqual(gates["gemini_byok_product_mode"], "disabled")
        # The base and optional local plugin must never be gated on cloud.
        self.assertNotIn("local", " ".join(blocked))
        synthetic = dict(gates, optional_local_ocr_plugin="blocked_missing")
        self.assertNotIn(
            "optional_local_ocr_plugin",
            release_readiness.cloud_blockers(synthetic),
        )

    def test_release_profiles_turn_pending_evidence_into_a_machine_failure(self):
        gates = release_readiness.run()
        self.assertEqual(
            set(release_readiness.PROFILE_REQUIRED),
            {"source", "base", "optional-local-ocr-plugin"},
        )
        self.assertEqual(release_readiness.required_failures(gates, "source"), [])
        base = release_readiness.required_failures(gates, "base")
        self.assertNotIn("optional_local_ocr_plugin", base)
        plugin = release_readiness.required_failures(
            gates, "optional-local-ocr-plugin"
        )
        self.assertIn("optional_local_ocr_plugin", plugin)
        self.assertNotIn("gemini_byok_product_mode", plugin)
        self.assertNotIn("mpdf_credits_complete_ocr_backend", plugin)

    def test_optional_plugin_evidence_is_independent_from_base_artifact(self):
        source = ROOT / "docs/evidence/rc3-ocr-runtime-smoke-arm64.json"
        evidence = json.loads(source.read_text())
        evidence["artifact"]["sha256"] = "f" * 64
        with tempfile.TemporaryDirectory() as tmp:
            install = write_base_install_fixture(Path(tmp) / "base-install.json")
            plugin = Path(tmp) / "plugin-smoke.json"
            plugin.write_text(json.dumps(evidence))
            gates = release_readiness.run(
                macos_install_evidence=install,
                ocr_runtime_evidence=plugin,
            )
        self.assertEqual(gates["macos_arm64_install_runtime"], "pass_local")
        self.assertEqual(gates["optional_local_ocr_plugin"], "pass_local")


class OcrRuntimeStructureTests(unittest.TestCase):
    TARGET = "aarch64-apple-darwin"

    def build_fixture(self, root: Path) -> tuple[Path, Path, dict]:
        runtime = root / "runtime"
        runtime.mkdir()
        payloads = {
            "bin/tesseract": b"engine",
            "bin/mpdf-ocr-sidecar": b"sidecar",
            "tessdata/grc.traineddata": b"greek-model",
            "tessdata/deu.traineddata": b"german-model",
            "licenses/TESSERACT.txt": b"Apache License 2.0 - Tesseract",
            "licenses/TESSDATA.txt": b"Apache License 2.0 - tessdata_best",
            "licenses/SIDECAR.txt": b"M PDF sidecar notices",
        }
        for name, payload in payloads.items():
            path = runtime / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(payload)
            if name.startswith("bin/"):
                path.chmod(0o755)

        pinned = root / "models.toml"
        model_specs = []
        for language in ("grc", "deu"):
            filename = f"{language}.traineddata"
            path = runtime / "tessdata" / filename
            model_specs.append(
                {
                    "language": language,
                    "filename": filename,
                    "sha256": verify_ocr_runtime.sha256(path),
                    "size_bytes": path.stat().st_size,
                }
            )
        pinned.write_text(
            "schema = \"mpdf-ocr-model-manifest\"\nschema_version = \"1.0\"\n\n"
            "[[model_set]]\nname = \"fixture-best\"\nversion = \"4.1.0\"\n"
            "engine = \"tesseract\"\nengine_min_version = \"5.0.0\"\n"
            "redistributable = true\n"
            + "".join(
                "\n[[model_set.file]]\n"
                f"language = \"{spec['language']}\"\n"
                f"filename = \"{spec['filename']}\"\n"
                f"sha256 = \"{spec['sha256']}\"\n"
                f"size_bytes = {spec['size_bytes']}\n"
                for spec in model_specs
            ),
            encoding="utf-8",
        )
        roles = {
            "bin/tesseract": "engine",
            "bin/mpdf-ocr-sidecar": "sidecar",
            "tessdata/grc.traineddata": "model",
            "tessdata/deu.traineddata": "model",
            "licenses/TESSERACT.txt": "license-tesseract",
            "licenses/TESSDATA.txt": "license-tessdata",
            "licenses/SIDECAR.txt": "license-sidecar",
        }
        files = []
        for name, role in roles.items():
            path = runtime / name
            entry = {
                "path": name,
                "role": role,
                "sha256": verify_ocr_runtime.sha256(path),
                "size_bytes": path.stat().st_size,
            }
            if role == "model":
                entry["language"] = Path(name).stem
            files.append(entry)
        manifest = {
            "schema": "mpdf-ocr-runtime-bundle",
            "schema_version": "1.0",
            "release": "0.1.0-rc.3",
            "target": self.TARGET,
            "engine": "tesseract",
            "engine_version": "5.5.3",
            "sidecar_protocol": "mpdf-ocr/0.1",
            "model_set": "fixture-best",
            "model_set_version": "4.1.0",
            "requires_system_python": False,
            "requires_system_tesseract": False,
            "files": files,
        }
        (runtime / verify_ocr_runtime.RUNTIME_MANIFEST).write_text(
            json.dumps(manifest), encoding="utf-8"
        )
        return runtime, pinned, manifest

    def test_self_contained_exact_runtime_inventory_passes_structure_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            runtime, pinned, _ = self.build_fixture(Path(tmp))
            evidence = verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)
            self.assertEqual(evidence["status"], "pass_structure_only")
            self.assertEqual(evidence["file_count"], 7)

    def test_system_python_or_tesseract_dependency_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            runtime, pinned, manifest = self.build_fixture(Path(tmp))
            manifest["requires_system_python"] = True
            (runtime / verify_ocr_runtime.RUNTIME_MANIFEST).write_text(json.dumps(manifest))
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "system Python"):
                verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)

    def test_unlisted_file_and_model_byte_change_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            runtime, pinned, _ = self.build_fixture(Path(tmp))
            (runtime / "surprise.dylib").write_bytes(b"not in manifest")
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "inventory mismatch"):
                verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)
            (runtime / "surprise.dylib").unlink()
            (runtime / "tessdata/grc.traineddata").write_bytes(b"replacement")
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "digest mismatch"):
                verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)

    def test_symlink_and_unsafe_target_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            runtime, pinned, manifest = self.build_fixture(Path(tmp))
            engine = runtime / "bin/tesseract"
            engine.unlink()
            engine.symlink_to(runtime / "bin/mpdf-ocr-sidecar")
            manifest["files"][0]["sha256"] = verify_ocr_runtime.sha256(engine)
            manifest["files"][0]["size_bytes"] = engine.stat().st_size
            (runtime / verify_ocr_runtime.RUNTIME_MANIFEST).write_text(json.dumps(manifest))
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "contained real file"):
                verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "invalid release target"):
                verify_ocr_runtime.verify(runtime, "../escape", pinned)

    def test_stager_is_network_free_and_produces_verifiable_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            inputs = root / "inputs"
            (inputs / "bin").mkdir(parents=True)
            engine = inputs / "bin/tesseract"
            engine.write_bytes(b"fixture-engine")
            engine.chmod(0o755)
            sidecar = root / "sidecar"
            sidecar.write_bytes(b"fixture-sidecar")
            sidecar.chmod(0o755)
            models = root / "models"
            models.mkdir()
            pinned = root / "models.toml"
            model_specs = []
            for language in ("grc", "deu"):
                path = models / f"{language}.traineddata"
                path.write_bytes((language + " model").encode())
                model_specs.append((language, path))
            pinned.write_text(
                'schema = "mpdf-ocr-model-manifest"\nschema_version = "1.0"\n\n'
                '[[model_set]]\nname = "fixture-best"\nversion = "4.1.0"\nengine = "tesseract"\n'
                'engine_min_version = "5.0.0"\nredistributable = true\nengine_license = "Apache-2.0"\n'
                'license = "Apache-2.0"\nlicense_url = "https://example.invalid/license"\n'
                'upstream_url = "https://example.invalid/models"\n'
                + "".join(
                    f'\n[[model_set.file]]\nlanguage = "{language}"\nfilename = "{path.name}"\n'
                    f'url = "https://example.invalid/models/{path.name}"\n'
                    f'sha256 = "{stage_ocr_runtime.sha256(path)}"\nsize_bytes = {path.stat().st_size}\n'
                    for language, path in model_specs
                ), encoding="utf-8")
            licenses = []
            for name in ("sidecar", "tesseract", "tessdata"):
                path = root / f"{name}.license"
                path.write_text(f"{name} license\n", encoding="utf-8")
                licenses.append(path)
            runtime = stage_ocr_runtime.stage(
                target=self.TARGET, sidecar=sidecar, tesseract_root=inputs,
                models_dir=models, out_dir=root / "out", model_set="fixture-best",
                tesseract_version="5.5.3", sidecar_license=licenses[0],
                sidecar_license_expression="MIT",
                tesseract_license=licenses[1], tessdata_license=licenses[2], pinned_models=pinned,
                allow_test_fixture=True,
            )
            manifest = json.loads((runtime / "runtime-manifest.json").read_text())
            self.assertEqual(manifest["provenance"]["source_policy"], "explicit-inputs-only; network-disabled")
            self.assertEqual(verify_ocr_runtime.verify(runtime, self.TARGET, pinned, allow_test_fixture=True)["status"], "pass_structure_only")
            with self.assertRaisesRegex(verify_ocr_runtime.VerificationError, "Mach-O"):
                verify_ocr_runtime.verify(runtime, self.TARGET, pinned)

    def test_smoke_without_bundled_runtime_is_explicitly_blocked(self):
        with tempfile.TemporaryDirectory() as tmp:
            evidence = ocr_runtime_smoke.smoke(layout=Path(tmp), target=self.TARGET)
            self.assertEqual(evidence["status"], "blocked")
            self.assertIn("bundled_ocr_runtime_not_found", evidence["reasons"])
            self.assertFalse(release_readiness.validate_ocr_runtime_evidence(Path(tmp) / "missing.json"))

    def test_prepopulated_output_shape_cannot_be_accepted_as_smoke_evidence(self):
        # An old/manual record can say "pass" and contain a four-page output,
        # but it has no harness-created/output-preexistence proof and must not
        # close the release gate.
        evidence = {
            "schema": "mpdf-ocr-runtime-smoke-evidence", "schema_version": "1.0",
            "status": "pass", "target": self.TARGET,
            "artifact": {"sha256": "a" * 64},
            "runner": {"sha256": "b" * 64},
            "runtime": {"manifest_sha256": "c" * 64, "engine_sha256": "d" * 64,
                        "sidecar_sha256": "e" * 64,
                        "model_sha256": {name: "f" * 64 for name in ("grc", "deu", "eng", "lat", "ell", "osd")}},
            "source": {"pages": 4, "sha256_before": "1" * 64, "sha256_after": "1" * 64},
            "output": {"pages": 4, "sha256": "2" * 64},
            "execution": {"exit_status": 0, "duration_seconds": 1,
                           "protocol": "mpdf run + executable-adjacent bundled runtime",
                           "runner_sha256": "b" * 64},
            "checks": {name: "pass" for name in ocr_runtime_smoke.REQUIRED_CHECKS},
        }
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "prepopulated.json"
            path.write_text(json.dumps(evidence))
            self.assertFalse(release_readiness.validate_ocr_runtime_evidence(path))


if __name__ == "__main__":
    unittest.main()
