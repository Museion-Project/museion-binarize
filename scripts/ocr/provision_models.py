#!/usr/bin/env python3
"""Provision and verify the offline OCR model set (developer/packaging tool).

This script is **not** part of the application. The app never downloads a
model; it only reads a directory that was provisioned here or shipped inside a
release bundle.

Two modes:

``--verify``    (default) check that every pinned file is present with the
                exact size and SHA-256 from
                ``distribution/ocr-models/manifest.toml``, then write the
                sidecar-facing ``manifest.json`` into the model directory.

``--download``  additionally fetch missing files from their pinned URLs. Every
                download is hashed and compared before it is kept; a mismatch
                deletes the file and fails. Only model sets whose manifest
                entry says ``redistributable = true`` may be downloaded.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tomllib
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PINNED_MANIFEST = REPO_ROOT / "distribution/ocr-models/manifest.toml"
MAX_MODEL_BYTES = 64 * 1024 * 1024


def load_pinned(path: Path) -> dict:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != "mpdf-ocr-model-manifest":
        raise SystemExit(f"{path}: unexpected manifest schema")
    return data


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def provision(model_set: dict, target: Path, allow_download: bool) -> list[dict]:
    target.mkdir(parents=True, exist_ok=True)
    entries = []
    for spec in model_set["file"]:
        path = target / spec["filename"]
        if not path.is_file():
            if not allow_download:
                raise SystemExit(
                    f"missing model {spec['filename']}; rerun with --download "
                    f"or copy it into {target}"
                )
            if not model_set.get("redistributable", False):
                raise SystemExit(
                    f"model set {model_set['name']} is not marked redistributable; "
                    "it must be provisioned manually"
                )
            if spec["size_bytes"] > MAX_MODEL_BYTES:
                raise SystemExit(f"{spec['filename']} exceeds the model size ceiling")
            print(f"fetching {spec['filename']} ...", file=sys.stderr)
            with urllib.request.urlopen(spec["url"], timeout=120) as response:
                payload = response.read(MAX_MODEL_BYTES + 1)
            if len(payload) > MAX_MODEL_BYTES:
                raise SystemExit(f"{spec['filename']} exceeds the model size ceiling")
            path.write_bytes(payload)
        actual_size = path.stat().st_size
        actual_digest = digest(path)
        if actual_size != spec["size_bytes"] or actual_digest != spec["sha256"]:
            path.unlink(missing_ok=True)
            raise SystemExit(
                f"{spec['filename']} does not match the pinned manifest "
                f"(size {actual_size}, sha256 {actual_digest})"
            )
        entries.append(
            {
                "engine": model_set["engine"],
                "language": spec["language"],
                "filename": spec["filename"],
                "sha256": actual_digest,
                "size_bytes": actual_size,
                "license": model_set["license"],
                "source": spec["url"],
                "model_version": model_set["version"],
            }
        )
    return entries


def write_sidecar_manifest(model_set: dict, entries: list[dict], target: Path) -> Path:
    manifest = {
        "schema": "mpdf-ocr-models",
        "schema_version": "1.0",
        "model_set": model_set["name"],
        "model_set_version": model_set["version"],
        "engine": model_set["engine"],
        "engine_min_version": model_set["engine_min_version"],
        "engine_license": model_set["engine_license"],
        "license": model_set["license"],
        "license_url": model_set["license_url"],
        "upstream_url": model_set["upstream_url"],
        "models": entries,
    }
    path = target / "manifest.json"
    path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--model-set", default="tessdata_best")
    parser.add_argument("--manifest", type=Path, default=PINNED_MANIFEST)
    parser.add_argument(
        "--download",
        action="store_true",
        help="fetch missing pinned files (developer/packaging use only)",
    )
    args = parser.parse_args()

    pinned = load_pinned(args.manifest)
    matches = [
        entry for entry in pinned.get("model_set", []) if entry["name"] == args.model_set
    ]
    if not matches:
        raise SystemExit(f"model set {args.model_set!r} is not pinned in {args.manifest}")
    model_set = matches[0]

    entries = provision(model_set, args.target_dir, args.download)
    manifest_path = write_sidecar_manifest(model_set, entries, args.target_dir)
    total = sum(entry["size_bytes"] for entry in entries)
    print(f"verified {len(entries)} model files ({total / 1_048_576:.1f} MiB)")
    print(f"wrote {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
