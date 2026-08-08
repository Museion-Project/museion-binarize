#!/usr/bin/env python3
"""Builds/updates `release-manifest.json`: the versioned, machine-readable
provenance record for one release's artifacts. See docs/releasing.md.

Schema `museion-binarize-release-manifest` v1.0. Deliberately excludes:
username, hostname, home directory, secret names/values, and absolute
developer filesystem paths — only project version, git SHA, target
triple, artifact filename/digest, and PDFium dependency provenance.

Usage:
    python3 scripts/distribution/release_manifest.py add \\
        --manifest release-manifest.json \\
        --project-version 0.1.0 --git-sha <sha> \\
        --target-triple aarch64-apple-darwin --os macos --arch arm64 \\
        --artifact-filename Museion-Binarize-0.1.0-macos-arm64.dmg \\
        --artifact-path /path/to/the.dmg \\
        --pdfium-build 7920 --pdfium-version 151.0.7920.0 \\
        --pdfium-sha256 <sha256> \\
        --signing-state unsigned --notarization-state not_applicable
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

SCHEMA = "museion-binarize-release-manifest"
SCHEMA_VERSION = "1.0"

VALID_SIGNING_STATES = {"unsigned", "ad_hoc", "signed", "pending_credentials"}
VALID_NOTARIZATION_STATES = {"not_applicable", "notarized", "pending_credentials"}


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_entry(
    *,
    target_triple: str,
    os_name: str,
    arch: str,
    artifact_filename: str,
    artifact_sha256: str,
    pdfium_build: str,
    pdfium_version: str,
    pdfium_sha256: str,
    signing_state: str,
    notarization_state: str,
    build_workflow: str | None = None,
) -> dict:
    if signing_state not in VALID_SIGNING_STATES:
        raise ValueError(f"invalid signing_state '{signing_state}'")
    if notarization_state not in VALID_NOTARIZATION_STATES:
        raise ValueError(f"invalid notarization_state '{notarization_state}'")
    entry = {
        "target_triple": target_triple,
        "os": os_name,
        "arch": arch,
        "artifact_filename": artifact_filename,
        "artifact_sha256": artifact_sha256,
        "pdfium_build": pdfium_build,
        "pdfium_version": pdfium_version,
        "pdfium_sha256": pdfium_sha256,
        "signing_state": signing_state,
        "notarization_state": notarization_state,
    }
    if build_workflow:
        entry["build_workflow"] = build_workflow
    return entry


def load_or_init(manifest_path: Path, project_version: str, git_sha: str) -> dict:
    if manifest_path.exists():
        data = json.loads(manifest_path.read_text())
        if data.get("project_version") != project_version or data.get("git_sha") != git_sha:
            raise SystemExit(
                f"error: {manifest_path} already records project_version="
                f"{data.get('project_version')!r} git_sha={data.get('git_sha')!r}; "
                f"refusing to mix in project_version={project_version!r} "
                f"git_sha={git_sha!r}. Start a fresh manifest for a different build."
            )
        return data
    return {
        "schema": SCHEMA,
        "schema_version": SCHEMA_VERSION,
        "project_version": project_version,
        "git_sha": git_sha,
        "artifacts": [],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    add = sub.add_parser("add", help="add one artifact entry to the manifest")
    add.add_argument("--manifest", type=Path, required=True)
    add.add_argument("--project-version", required=True)
    add.add_argument("--git-sha", required=True)
    add.add_argument("--target-triple", required=True)
    add.add_argument("--os", dest="os_name", required=True)
    add.add_argument("--arch", required=True)
    add.add_argument("--artifact-filename", required=True)
    add.add_argument("--artifact-path", type=Path, required=True)
    add.add_argument("--pdfium-build", required=True)
    add.add_argument("--pdfium-version", required=True)
    add.add_argument("--pdfium-sha256", required=True)
    add.add_argument("--signing-state", required=True, choices=sorted(VALID_SIGNING_STATES))
    add.add_argument(
        "--notarization-state", required=True, choices=sorted(VALID_NOTARIZATION_STATES)
    )
    add.add_argument("--build-workflow", default=None)

    args = parser.parse_args()

    if args.command == "add":
        data = load_or_init(args.manifest, args.project_version, args.git_sha)
        entry = build_entry(
            target_triple=args.target_triple,
            os_name=args.os_name,
            arch=args.arch,
            artifact_filename=args.artifact_filename,
            artifact_sha256=sha256_of(args.artifact_path),
            pdfium_build=args.pdfium_build,
            pdfium_version=args.pdfium_version,
            pdfium_sha256=args.pdfium_sha256,
            signing_state=args.signing_state,
            notarization_state=args.notarization_state,
            build_workflow=args.build_workflow,
        )
        # Replace any existing entry for the same artifact file rather
        # than accumulating duplicates across repeated local runs. Keyed
        # on filename, not target_triple: a single target produces more
        # than one artifact (e.g. the desktop .dmg and the CLI archive),
        # and deduping on target_triple alone silently discarded every
        # artifact but the last one processed for that target.
        data["artifacts"] = [
            e for e in data["artifacts"] if e["artifact_filename"] != args.artifact_filename
        ] + [entry]
        args.manifest.write_text(json.dumps(data, indent=2) + "\n")
        print(f"wrote {args.manifest} ({len(data['artifacts'])} artifact(s))")


if __name__ == "__main__":
    main()
