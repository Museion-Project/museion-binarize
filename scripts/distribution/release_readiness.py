#!/usr/bin/env python3
"""Network-free rc.3 release-readiness gates.

Missing external evidence stays pending/not_run. The macOS-local profile
validates supplied artifact-bound evidence without launching external services.
Use ``--json`` for CI and owner review records.
"""
from __future__ import annotations

import argparse
import contextlib
import io
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_identity_compatibility as identity  # noqa: E402
import check_version_consistency as versions  # noqa: E402
import generate_sbom  # noqa: E402
import release_manifest  # noqa: E402
import verify_ocr_runtime  # noqa: E402
import ocr_runtime_smoke  # noqa: E402
import macos_local_readiness  # noqa: E402


PDFIUM_SMOKE_GATES = {
    "core_auto_bookmarks_pdf_ignored",
    "cli_bookmarks_cli_ignored",
    "core_searchable_pdf",
    "core_pdf_pipeline_ignored",
}

MACOS_INSTALL_CHECKS = {
    "source_build",
    "bundle_identity_me_mpdf_processor",
    "bundle_version_rc3",
    "main_executable_arm64",
    "bundled_pdfium_arm64",
    "codesign_deep_strict",
    "dmg_create_mount_copy_unmount",
    "launchservices_start",
    "native_window_visible",
    "open_four_page_fixture_with_native_picker",
    "processed_preview_rendered",
    "gui_conversion",
    "converted_pdf_reopened_in_app",
    "converted_pdf_qpdf_check",
    "clean_exit",
}

STATIC_REQUIRED = {
    "version_wix", "identity_freeze", "unreleased_rc3_scope",
    "current_state_docs", "sbom_manifest_schema",
}
BASE_REQUIRED = STATIC_REQUIRED | {
    "pdfium_runtime_smoke", "macos_arm64_install_runtime", "reader_matrix",
    "distribution_ci_rc3", "developer_id_notary", "cross_platform_runtime",
    "upgrade_install", "privacy_accessibility_performance",
}
PROFILE_REQUIRED = {
    "macos-local": macos_local_readiness.REQUIRED | {"version_wix", "identity_freeze"},
    "source": STATIC_REQUIRED,
    "base": BASE_REQUIRED,
    "optional-local-ocr-plugin": BASE_REQUIRED | {
        "optional_local_ocr_plugin", "human_gold_bookmarks",
    },
}
CLOUD_GATE_NAMES = {
    "mpdf_credits_complete_ocr_backend",
    "mpdf_credits_payment_integration",
    "cloud_privacy_policy_and_deletion",
}


def validate_pdfium_evidence(path: Path) -> bool:
    """Validate explicit smoke evidence; never infer it from a library file."""
    try:
        evidence = json.loads(path.read_text())
        target = evidence["target"]
        digest = evidence["library_sha256"]
        gates = evidence["gates"]
        manifest = tomllib.loads((ROOT / "distribution/pdfium/manifest.toml").read_text())
        expected = next(a["library_sha256"] for a in manifest["asset"] if a["target_triple"] == target)
    except (KeyError, OSError, StopIteration, TypeError, json.JSONDecodeError):
        return False
    if not isinstance(target, str) or "/" in target or "\\" in target:
        return False
    return (isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest) is not None
            and digest == expected
            and isinstance(gates, dict)
            and set(gates) == PDFIUM_SMOKE_GATES
            and all(value == "pass" for value in gates.values()))


def validate_macos_install_evidence(path: Path) -> bool:
    """Validate the explicit fresh-install/runtime record for rc.3 arm64."""
    try:
        evidence = json.loads(path.read_text())
        artifact = evidence["artifact"]
        checks = evidence["checks"]
        converted = evidence["converted_fixture"]
    except (KeyError, OSError, TypeError, json.JSONDecodeError):
        return False
    if (evidence.get("schema") != "mpdf-release-evidence"
            or evidence.get("schema_version") != "0.1"
            or evidence.get("release") != "0.1.0-rc.3"
            or evidence.get("distribution_profile") != "base"
            or "bundled_ocr" in evidence
            or evidence.get("target") != "aarch64-apple-darwin"):
        return False
    if (artifact.get("kind") != "dmg"
            or artifact.get("signing_state") != "ad_hoc"
            or artifact.get("notarization_state") != "pending_credentials"
            or re.fullmatch(r"[0-9a-f]{64}", str(artifact.get("sha256", ""))) is None):
        return False
    if (not isinstance(checks, dict)
            or not MACOS_INSTALL_CHECKS.issubset(checks)
            or any(checks[name] != "pass" for name in MACOS_INSTALL_CHECKS)
            or checks.get("gatekeeper_assessment") != "expected_rejection_ad_hoc"):
        return False
    return (converted.get("pages") == 4
            and isinstance(converted.get("bytes"), int)
            and converted["bytes"] > 0
            and re.fullmatch(r"[0-9a-f]{64}", str(converted.get("sha256", ""))) is not None)


def validate_ocr_runtime_evidence(path: Path) -> bool:
    """Accept only a real four-page installed smoke, never structure alone."""
    try:
        evidence = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    if (evidence.get("schema") != "mpdf-ocr-runtime-smoke-evidence"
            or evidence.get("schema_version") != "1.0"
            or evidence.get("status") != "pass"):
        return False
    target = evidence.get("target")
    runtime = evidence.get("runtime")
    artifact = evidence.get("artifact")
    source = evidence.get("source")
    output = evidence.get("output")
    if target != "aarch64-apple-darwin" or not isinstance(runtime, dict) or not isinstance(artifact, dict):
        return False
    checks = evidence.get("checks")
    if (not isinstance(checks, dict)
            or set(checks) != set(ocr_runtime_smoke.REQUIRED_CHECKS)
            or any(value != "pass" for value in checks.values())):
        return False
    if (not isinstance(source, dict) or source.get("pages") != 4
            or source.get("sha256_before") != source.get("sha256_after")):
        return False
    if (not isinstance(evidence.get("runner"), dict)
            or re.fullmatch(r"[0-9a-f]{64}", str(evidence["runner"].get("sha256", ""))) is None
            or not isinstance(output, dict) or output.get("pages") != 4
            or output.get("created_by_harness") is not True
            or output.get("preexisting") is not False
            or not isinstance(evidence.get("execution"), dict)
            or evidence["execution"].get("exit_status") != 0
            or evidence["execution"].get("protocol") != "mpdf run + executable-adjacent bundled runtime"
            or evidence["execution"].get("runner_sha256") != evidence.get("runner", {}).get("sha256")
            or not isinstance(evidence["execution"].get("duration_seconds"), (int, float))):
        return False
    for key in ("manifest_sha256", "engine_sha256", "sidecar_sha256"):
        if re.fullmatch(r"[0-9a-f]{64}", str(runtime.get(key, ""))) is None:
            return False
    models = runtime.get("model_sha256")
    if (not isinstance(models, dict) or set(models) != {"grc", "deu", "eng", "lat", "ell", "osd"
                                                        } or any(re.fullmatch(r"[0-9a-f]{64}", str(value)) is None
                                                                 for value in models.values())):
        return False
    return (re.fullmatch(r"[0-9a-f]{64}", str(artifact.get("sha256", ""))) is not None
            and re.fullmatch(r"[0-9a-f]{64}", str(output.get("sha256", ""))) is not None)


def run(*, pdfium_evidence: Path | None = None,
        macos_install_evidence: Path | None = None,
        ocr_runtime_evidence: Path | None = None) -> dict[str, str]:
    gates: dict[str, str] = {}
    try:
        # Keep --json strictly machine-readable; the standalone version
        # checker intentionally prints a human summary.
        with contextlib.redirect_stdout(io.StringIO()):
            versions.main()
        gates["version_wix"] = "pass_static"
    except SystemExit:
        gates["version_wix"] = "fail"
    gates["identity_freeze"] = "pass_static" if not identity.check() else "fail"
    changelog = (ROOT / "CHANGELOG.md").read_text()
    gates["unreleased_rc3_scope"] = "pass_static" if "rc.3" in changelog and "Unreleased" in changelog else "fail"
    current_docs = "\n".join((ROOT / name).read_text(errors="ignore") for name in (
        "README.md", "README.zh-CN.md", "spec.md", "docs/ocr-providers.md",
        "docs/distribution.md", "docs/limitations.md", "THIRD_PARTY_LICENSES.md",
    ))
    stale_phrases = ("there is no networking, telemetry, account, OCR", "无 OCR、AI、书签")
    required_phrases = (
        "optional local OCR plugin", "mpdf-ocr-provider/2",
        "gemini-byok is disabled in this version", "mpdf-credits",
    )
    normalized_docs = current_docs.lower()
    gates["current_state_docs"] = (
        "pass_static"
        if all(phrase.lower() in normalized_docs for phrase in required_phrases)
        and not any(phrase in current_docs for phrase in stale_phrases)
        else "fail"
    )
    try:
        with (ROOT / "distribution/pdfium/manifest.toml").open("rb") as f:
            pdfium = tomllib.load(f)
        release_manifest.validate_manifest({"schema": "mpdf-release-manifest", "schema_version": "1.1", "artifacts": []})
        # Build a fixture-shaped SBOM through the exact production function.
        generate_sbom.build_sbom(project_version="0.1.0-rc.3", target="aarch64-apple-darwin",
                                 cargo_metadata={"packages": []}, pnpm_lock=None, pdfium_manifest=pdfium)
        gates["sbom_manifest_schema"] = "pass_local"
    except (ValueError, KeyError, OSError):
        gates["sbom_manifest_schema"] = "fail"
    macos_install_valid = bool(macos_install_evidence and validate_macos_install_evidence(macos_install_evidence))
    ocr_runtime_valid = bool(ocr_runtime_evidence and validate_ocr_runtime_evidence(ocr_runtime_evidence))
    gates.update({
        # Runtime smoke is evidence-driven. Merely having a provisioned
        # library is never sufficient; callers must pass --pdfium-evidence.
        "pdfium_runtime_smoke": "pass_local" if pdfium_evidence and validate_pdfium_evidence(pdfium_evidence) else "pending",
        "macos_arm64_install_runtime": "pass_local" if macos_install_valid else "pending",
        "reader_matrix": "pending",
        "distribution_ci_rc3": "not_run",
        "developer_id_notary": "pending_owner_credentials",
        "cross_platform_runtime": "pending",
        "upgrade_install": "not_run",
        # This is an independent optional-plugin artifact gate. It is not a
        # base-release dependency, and its artifact digest need not equal the
        # base desktop artifact digest.
        "optional_local_ocr_plugin": "pass_local" if ocr_runtime_valid else "pending",
        "human_gold_bookmarks": "pending",
        "privacy_accessibility_performance": "pending_evidence_review",
        # Gemini BYOK is intentionally not a product mode. Legacy values stay
        # readable but cannot start work or appear in provider discovery.
        "gemini_byok_product_mode": "disabled",
        # The only planned cloud product is paid/brokered mpdf-credits. It has
        # no production complete-coordinate OCR service. These are product
        # availability blockers, not base-release gates.
        "mpdf_credits_complete_ocr_backend": "blocked_no_service",
        "mpdf_credits_payment_integration": "blocked_no_service",
        "cloud_privacy_policy_and_deletion": "blocked_not_published",
    })
    return gates


def cloud_blockers(gates: dict[str, str]) -> list[str]:
    """Gates that must be cleared before mpdf-credits is advertised.

    ``blocked_*`` is deliberately distinct from ``pending``: pending means
    evidence has not been gathered, blocked means the thing being evidenced
    does not exist yet.
    """
    return sorted(
        name for name in CLOUD_GATE_NAMES
        if gates.get(name, "missing").startswith("blocked_")
    )


def required_failures(gates: dict[str, str], profile: str) -> list[str]:
    """Required gates that are not an explicit pass for one release profile."""
    required = PROFILE_REQUIRED[profile]
    return sorted(name for name in required
                  if not gates.get(name, "missing").startswith("pass_"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--profile", choices=sorted(PROFILE_REQUIRED), default="source")
    parser.add_argument("--pdfium-evidence", type=Path,
                        help="explicit four-gate evidence JSON; without it smoke remains pending")
    parser.add_argument("--macos-install-evidence", type=Path,
                        help="explicit rc.3 arm64 install/runtime evidence JSON")
    parser.add_argument("--ocr-runtime-evidence", type=Path,
                        help="explicit installed four-page OCR smoke evidence JSON")
    parser.add_argument("--macos-local-evidence", type=Path, help="artifact-bound macOS-only release evidence")
    args = parser.parse_args()
    result = run(pdfium_evidence=args.pdfium_evidence,
                 macos_install_evidence=args.macos_install_evidence,
                 ocr_runtime_evidence=args.ocr_runtime_evidence)
    if args.profile == "macos-local":
        local = macos_local_readiness.evaluate(args.macos_local_evidence)
        result = {key: result[key] for key in ("version_wix", "identity_freeze")} | local
    missing_required = required_failures(result, args.profile)
    if args.json:
        print(json.dumps({"release": versions.workspace_version(), "status": ("ready" if not missing_required else "not_ready") if args.profile == "macos-local" else "pre-release-source",
                          "profile": args.profile, "gates": result,
                          "required_failures": missing_required,
                          "cloud_blockers": cloud_blockers(result)},
                         indent=2, sort_keys=True))
    else:
        for key, value in result.items():
            print(f"{key}: {value}")
        blocked = cloud_blockers(result)
        if blocked:
            print("\nmpdf-credits is unavailable; base and local plugin releases are unaffected:")
            for name in blocked:
                print(f"  - {name}: {result[name]}")
        if missing_required:
            print(f"\n{args.profile} release profile is not ready:")
            for name in missing_required:
                print(f"  - {name}: {result.get(name, 'missing')}")
    return 1 if missing_required or any(value == "fail" for value in result.values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
