#!/usr/bin/env python3
"""Static identity freeze checks for release candidates."""
from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CANONICAL = {
    "product": "Museion PDF",
    "bundle_identifier": "me.mpdf.processor",
    "cli": "mpdf",
    "repo": "museion-binarize",
    "repository_url": "https://github.com/Museion-Project/museion-binarize",
}
STABLE_SCHEMAS = {
    "mpdf-document-package": "crates/mpdf-core/src/document_package.rs",
    "mpdf-job": "crates/mpdf-core/src/jobs.rs",
    "mpdf-bookmark-auto": "crates/mpdf-cli/src/commands/bookmarks.rs",
    "mpdf-searchable-pdf": "crates/mpdf-cli/src/commands/pdf.rs",
}
# Keep the checker itself free of a contiguous retired identifier, so the
# repository-wide active-code grep can safely include this script.
RETIRED_ACTIVE_IDENTIFIERS = {"org.museionproject." + "binarize", "me.museion." + "binarize"}


def check() -> list[str]:
    errors: list[str] = []
    config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())
    if config.get("productName") != CANONICAL["product"]:
        errors.append("tauri productName drifted")
    if config.get("identifier") != CANONICAL["bundle_identifier"]:
        errors.append("tauri bundle identifier drifted")
    cli_manifest = tomllib.loads((ROOT / "crates/mpdf-cli/Cargo.toml").read_text())
    cli_package = cli_manifest.get("package", {})
    if cli_package.get("name") != "mpdf-cli":
        errors.append("CLI crate identity drifted")
    bins = cli_manifest.get("bin", [])
    if not any(isinstance(item, dict) and item.get("name") == "mpdf" for item in bins):
        errors.append("Cargo [[bin]] name must remain mpdf")
    root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    if root_manifest.get("workspace", {}).get("package", {}).get("repository") != CANONICAL["repository_url"]:
        errors.append("canonical workspace repository URL drifted")
    if not (ROOT / "crates/mpdf-cli/src/main.rs").exists():
        errors.append("CLI source missing")
    # Distribution/MAS overlays must inherit all product identity/version
    # fields from the base config.  Their deliberately small allowlist keeps
    # a future overlay from silently creating a second product.
    for relative in ("apps/desktop/src-tauri/tauri.dist.conf.json", "apps/desktop/src-tauri/tauri.mas.conf.json"):
        data = json.loads((ROOT / relative).read_text())
        if "identifier" in data:
            errors.append(f"{relative} redeclares bundle identifier")
        if any(key in data for key in ("productName", "version")):
            errors.append(f"{relative} redeclares product/version identity")
        if set(data) - {"$schema", "bundle"}:
            errors.append(f"{relative} has unexpected top-level identity keys")

    # Keep this check an explicit allowlist rather than searching for four
    # convenient strings.  These are the protocol/schema identities whose
    # compatibility is promised across rc.3 upgrades; each must be declared
    # by its owning implementation file.
    if set(STABLE_SCHEMAS) != {
        "mpdf-document-package", "mpdf-job", "mpdf-bookmark-auto", "mpdf-searchable-pdf"
    }:
        errors.append("stable schema allowlist changed unexpectedly")
    # A version bump must not alter protocol/schema identifiers.
    for identity, relative in STABLE_SCHEMAS.items():
        text = (ROOT / relative).read_text(errors="ignore")
        if not re.search(rf"[\"']{re.escape(identity)}[\"']", text):
            errors.append(f"expected stable schema identity absent from {relative}: {identity}")
    active_paths = list((ROOT / "crates").rglob("*.rs")) + list((ROOT / "apps").rglob("*.json"))
    active_paths += [path for path in (ROOT / "scripts").rglob("*.py")
                     if path.name != "test_distribution.py"]
    active = "\n".join(p.read_text(errors="ignore") for p in active_paths)
    for identity in RETIRED_ACTIVE_IDENTIFIERS:
        if identity in active:
            errors.append(f"retired identifier still appears in active code/config: {identity}")
    return errors


def main() -> int:
    errors = check()
    if errors:
        raise SystemExit("identity compatibility FAILED:\n" + "\n".join("- " + e for e in errors))
    print("identity compatibility OK: product, bundle, CLI and schema identities frozen")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
