#!/usr/bin/env python3
"""Builds/updates `release-manifest.json`: the versioned, machine-readable
provenance record for one release's artifacts. See docs/releasing.md.

Schema `mpdf-release-manifest` v1.1. Version 1.0 manifests remain readable;
new entries carry an explicit artifact kind. Deliberately excludes:
username, hostname, home directory, secret names/values, and absolute
developer filesystem paths — only project version, git SHA, target
triple, artifact filename/digest, and PDFium dependency provenance.

Usage:
    python3 scripts/distribution/release_manifest.py add \\
        --manifest release-manifest.json \\
        --project-version 0.1.0 --git-sha <sha> \\
        --target-triple aarch64-apple-darwin --os macos --arch arm64 \\
        --artifact-filename mpdf-0.1.0-macos-arm64.dmg \\
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

SCHEMA = "mpdf-release-manifest"
SCHEMA_VERSION = "1.1"
SUPPORTED_SCHEMA_VERSIONS = {"1.0", "1.1"}
VALID_ARTIFACT_KINDS = {"desktop", "cli", "sbom", "checksums", "release-notes"}

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
    artifact_kind: str = "desktop",
) -> dict:
    if signing_state not in VALID_SIGNING_STATES:
        raise ValueError(f"invalid signing_state '{signing_state}'")
    if notarization_state not in VALID_NOTARIZATION_STATES:
        raise ValueError(f"invalid notarization_state '{notarization_state}'")
    if artifact_kind not in VALID_ARTIFACT_KINDS:
        raise ValueError(f"invalid artifact_kind '{artifact_kind}'")
    if artifact_kind in {"sbom", "checksums", "release-notes"} and (
        signing_state != "unsigned" or notarization_state != "not_applicable"
    ):
        raise ValueError(f"non-binary artifact kind '{artifact_kind}' cannot have signing state")
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
        "artifact_kind": artifact_kind,
    }
    if build_workflow:
        entry["build_workflow"] = build_workflow
    return entry


def validate_manifest(data: dict) -> None:
    """Validate additive v1.1 fields and kind-specific signing semantics.

    v1.0 manifests intentionally remain valid on the compatibility read path;
    their missing ``artifact_kind`` is interpreted as ``desktop``.
    """
    if data.get("schema") != SCHEMA:
        raise ValueError(f"unsupported manifest schema {data.get('schema')!r}")
    if str(data.get("schema_version")) not in SUPPORTED_SCHEMA_VERSIONS:
        raise ValueError(f"unsupported manifest schema version {data.get('schema_version')!r}")
    artifacts = data.get("artifacts")
    if not isinstance(artifacts, list):
        raise ValueError("manifest artifacts must be a list")
    names: set[str] = set()
    for entry in artifacts:
        if not isinstance(entry, dict):
            raise ValueError("manifest artifact entry must be an object")
        name = entry.get("artifact_filename")
        if not isinstance(name, str) or not name or name in names:
            raise ValueError(f"invalid or duplicate artifact filename {name!r}")
        names.add(name)
        kind = entry.get("artifact_kind", "desktop")
        if kind not in VALID_ARTIFACT_KINDS:
            raise ValueError(f"invalid artifact kind {kind!r} for {name!r}")
        if entry.get("signing_state") not in {None, *VALID_SIGNING_STATES}:
            raise ValueError(f"invalid signing state for {name!r}")
        if entry.get("notarization_state") not in {None, *VALID_NOTARIZATION_STATES}:
            raise ValueError(f"invalid notarization state for {name!r}")
        if entry.get("notarization_state") == "notarized" and entry.get("signing_state") != "signed":
            raise ValueError(f"notarized artifact {name!r} must also claim real signing")
        if kind == "cli" and not name.startswith("mpdf-cli-"):
            raise ValueError(f"CLI artifact has non-CLI filename {name!r}")
        if kind == "sbom" and not name.endswith((".sbom.json", ".spdx.json")):
            raise ValueError(f"SBOM artifact must use a deterministic JSON filename: {name!r}")
        if kind in {"sbom", "checksums", "release-notes"}:
            if entry.get("signing_state") not in {None, "unsigned"}:
                raise ValueError(f"{kind} artifact must not claim signing")
            if entry.get("notarization_state") not in {None, "not_applicable"}:
                raise ValueError(f"{kind} artifact must not claim notarization")


def load_or_init(manifest_path: Path, project_version: str, git_sha: str) -> dict:
    if manifest_path.exists():
        data = json.loads(manifest_path.read_text())
        validate_manifest(data)
        if data.get("project_version") != project_version or data.get("git_sha") != git_sha:
            raise SystemExit(
                f"error: {manifest_path} already records project_version="
                f"{data.get('project_version')!r} git_sha={data.get('git_sha')!r}; "
                f"refusing to mix in project_version={project_version!r} "
                f"git_sha={git_sha!r}. Start a fresh manifest for a different build."
            )
        return data
    # Keep the programmatic initializer's v1.0 shape readable for callers
    # that use it as a compatibility fixture.  The CLI upgrades a newly
    # written manifest to v1.1 before adding its first typed entry.
    return {
        "schema": SCHEMA,
        "schema_version": "1.0",
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
    add.add_argument("--artifact-kind", choices=sorted(VALID_ARTIFACT_KINDS), default="desktop")

    args = parser.parse_args()

    if args.command == "add":
        data = load_or_init(args.manifest, args.project_version, args.git_sha)
        data["schema_version"] = SCHEMA_VERSION
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
            artifact_kind=args.artifact_kind,
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
        validate_manifest(data)
        args.manifest.write_text(json.dumps(data, indent=2) + "\n")
        print(f"wrote {args.manifest} ({len(data['artifacts'])} artifact(s))")


if __name__ == "__main__":
    main()
