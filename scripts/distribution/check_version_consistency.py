#!/usr/bin/env python3
"""Fails if the project's version metadata has drifted apart.

Single source of truth: the workspace `Cargo.toml`'s `[workspace.package]
version`. Every other version field in the repository must match it
exactly:

- `apps/desktop/package.json`'s `version`
- `apps/desktop/src-tauri/tauri.conf.json`'s `version`

Run:
    python3 scripts/distribution/check_version_consistency.py

Exits 0 with a summary line if consistent, non-zero listing every
mismatch otherwise. No dependencies beyond the Python standard library.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


def workspace_version() -> str:
    path = REPO_ROOT / "Cargo.toml"
    with path.open("rb") as f:
        data = tomllib.load(f)
    try:
        return data["workspace"]["package"]["version"]
    except KeyError as e:
        raise SystemExit(f"error: could not find [workspace.package].version in {path}") from e


def package_json_version() -> str:
    path = REPO_ROOT / "apps" / "desktop" / "package.json"
    data = json.loads(path.read_text())
    if "version" not in data:
        raise SystemExit(f"error: no 'version' field in {path}")
    return data["version"]


def tauri_conf_version() -> str:
    path = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
    data = json.loads(path.read_text())
    if "version" not in data:
        raise SystemExit(f"error: no 'version' field in {path}")
    return data["version"]


def cargo_crate_versions() -> dict[str, str]:
    """Every workspace-member crate that uses `version.workspace = true`
    inherits the workspace version automatically at build time via
    Cargo's own mechanism — nothing to check there. This function exists
    to catch the opposite mistake: a crate that pins its *own* literal
    version instead of inheriting."""
    mismatches: dict[str, str] = {}
    for cargo_toml in REPO_ROOT.glob("crates/*/Cargo.toml"):
        with cargo_toml.open("rb") as f:
            data = tomllib.load(f)
        package = data.get("package", {})
        version = package.get("version")
        if isinstance(version, str):
            mismatches[str(cargo_toml)] = version
    tauri_cargo = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "Cargo.toml"
    with tauri_cargo.open("rb") as f:
        data = tomllib.load(f)
    version = data.get("package", {}).get("version")
    if isinstance(version, str):
        mismatches[str(tauri_cargo)] = version
    return mismatches


def main() -> None:
    ws_version = workspace_version()
    problems: list[str] = []

    pkg_version = package_json_version()
    if pkg_version != ws_version:
        problems.append(
            f"apps/desktop/package.json version '{pkg_version}' != "
            f"workspace version '{ws_version}'"
        )

    tauri_version = tauri_conf_version()
    if tauri_version != ws_version:
        problems.append(
            f"apps/desktop/src-tauri/tauri.conf.json version '{tauri_version}' != "
            f"workspace version '{ws_version}'"
        )

    literal_crate_versions = cargo_crate_versions()
    for path, version in literal_crate_versions.items():
        if version != ws_version:
            problems.append(
                f"{path} pins a literal version '{version}' instead of "
                f"`version.workspace = true` (workspace version is '{ws_version}')"
            )

    if not re.match(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$", ws_version):
        problems.append(f"workspace version '{ws_version}' is not valid SemVer")

    if problems:
        print(f"version consistency check FAILED (workspace version: {ws_version})", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        sys.exit(1)

    print(f"version consistency OK: {ws_version}")


if __name__ == "__main__":
    main()
