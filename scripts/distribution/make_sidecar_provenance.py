#!/usr/bin/env python3
"""Create frozen-sidecar software provenance and a combined license notice."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build(args: argparse.Namespace) -> tuple[Path, Path]:
    inputs = [
        ("M PDF OCR sidecar source", "0.1.0-rc.3", "MIT OR Apache-2.0", args.sidecar_source,
         [args.project_mit, args.project_apache], "runtime"),
        ("CPython", "3.11.15", "Python-2.0", args.python_library,
         [args.python_license], "runtime"),
        ("Pillow", "12.2.0", "MIT-CMU", args.pillow_wheel,
         [args.pillow_license], "runtime"),
        ("PyInstaller bootloader", "6.22.2", "GPL-2.0-or-later WITH Bootloader-exception",
         args.pyinstaller_bootloader, [args.pyinstaller_license], "build-runtime"),
    ]
    args.output.mkdir(parents=True, exist_ok=True)
    software = []
    notice_parts = []
    for name, version, license_expression, identity, licenses, scope in inputs:
        if not identity.is_file() or identity.is_symlink():
            raise ValueError(f"identity input is not a real file: {identity}")
        software.append({
            "name": name, "version": version, "license": license_expression,
            "sha256": sha256(identity), "scope": scope,
        })
        for license_path in licenses:
            if not license_path.is_file() or license_path.is_symlink():
                raise ValueError(f"license input is not a real file: {license_path}")
            notice_parts.append(
                f"===== {name} {version} — {license_expression} =====\n\n"
                + license_path.read_text(encoding="utf-8", errors="replace").rstrip()
                + "\n"
            )
    manifest = {
        "schema": "mpdf-frozen-sidecar-provenance", "schema_version": "1.0",
        "software": software,
    }
    manifest_path = args.output / "dependency-manifest.json"
    notice_path = args.output / "SIDECAR-NOTICES.txt"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    notice_path.write_text("\n".join(notice_parts), encoding="utf-8")
    return manifest_path, notice_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sidecar-source", type=Path, required=True)
    parser.add_argument("--python-library", type=Path, required=True)
    parser.add_argument("--pillow-wheel", type=Path, required=True)
    parser.add_argument("--pyinstaller-bootloader", type=Path, required=True)
    parser.add_argument("--project-mit", type=Path, required=True)
    parser.add_argument("--project-apache", type=Path, required=True)
    parser.add_argument("--python-license", type=Path, required=True)
    parser.add_argument("--pillow-license", type=Path, required=True)
    parser.add_argument("--pyinstaller-license", type=Path, required=True)
    args = parser.parse_args()
    try:
        for path in build(args):
            print(path)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"sidecar provenance failed: {exc}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
