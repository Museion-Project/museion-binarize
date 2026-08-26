#!/usr/bin/env python3
"""Merges the per-target artifacts of one explicitly-selected, already
successful `build-distribution.yml` run into one release-wide asset set:
a single `release-manifest.json`, a single `SHA256SUMS`, and every
public downloadable binary/archive copied into one output directory.

This is deliberately *not* the step that talks to the GitHub API or
downloads anything — that is `publish-release.yml`'s job (`gh run
download`), which is not something a unit test should have to perform
live. This script operates on an already-downloaded local directory
tree and is fully exercised by `test_distribution.py` with synthetic
fixtures. See docs/releasing.md, "Release-wide artifact aggregation."

Expected input layout (exactly what `gh run download --dir <X>` with no
`--name` produces: one subdirectory per uploaded artifact name):

    <artifacts-dir>/
      mpdf-aarch64-apple-darwin/
        mpdf-0.1.0-rc.1-macos-arm64.dmg
        mpdf-cli-0.1.0-rc.1-macos-arm64.tar.gz
        release-manifest.json
        SHA256SUMS
      mpdf-x86_64-pc-windows-msvc/
        ...

Fails closed (non-zero exit, no partial output) on:

- a per-target manifest's `project_version` or `git_sha` disagreeing
  with the explicitly expected values;
- the expected tag not matching `v<expected-version>`;
- a file present in a per-target directory with no corresponding entry
  in that target's own `release-manifest.json` (an "unexpected" file —
  classified and rejected, never silently included);
- a manifest artifact entry with no corresponding file on disk;
- two different source files that would collide under the same output
  filename;
- two manifest entries claiming the same `artifact_filename`.

Usage:
    python3 scripts/distribution/aggregate_release.py \\
        --artifacts-dir downloaded/ \\
        --expected-version 0.1.0-rc.1 \\
        --expected-tag v0.1.0-rc.1 \\
        --expected-git-sha <full 40-hex sha> \\
        --out-dir release-assets/
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import checksums  # noqa: E402

SCHEMA = "mpdf-release-manifest"
SCHEMA_VERSION = "1.0"

# Files a per-target job's dist-out/ legitimately contains that are not
# themselves public release assets — never copied to the aggregated
# output, and never expected to have their own manifest artifact entry.
NON_ASSET_FILENAMES = {"release-manifest.json", "SHA256SUMS"}


class AggregationError(SystemExit):
    """Raised (as SystemExit) for any fail-closed condition below —
    named so tests can assert on it specifically rather than on bare
    SystemExit, which argparse itself also raises."""


def expected_tag_for(version: str) -> str:
    return f"v{version}"


def validate_tag(expected_tag: str, expected_version: str) -> None:
    want = expected_tag_for(expected_version)
    if expected_tag != want:
        raise AggregationError(
            f"tag/version mismatch: expected tag {want!r} for version "
            f"{expected_version!r}, got {expected_tag!r}"
        )


def load_target_manifest(target_dir: Path) -> dict:
    manifest_path = target_dir / "release-manifest.json"
    if not manifest_path.is_file():
        raise AggregationError(f"{target_dir} has no release-manifest.json")
    return json.loads(manifest_path.read_text())


def validate_target_manifest(
    manifest: dict, target_dir: Path, expected_version: str, expected_git_sha: str
) -> None:
    if manifest.get("project_version") != expected_version:
        raise AggregationError(
            f"{target_dir}: manifest project_version "
            f"{manifest.get('project_version')!r} != expected {expected_version!r}"
        )
    if manifest.get("git_sha") != expected_git_sha:
        raise AggregationError(
            f"{target_dir}: manifest git_sha {manifest.get('git_sha')!r} != "
            f"expected {expected_git_sha!r}"
        )


def validate_no_unexpected_files(manifest: dict, target_dir: Path) -> None:
    known_filenames = {a["artifact_filename"] for a in manifest["artifacts"]}
    for path in target_dir.iterdir():
        if not path.is_file():
            continue
        if path.name in NON_ASSET_FILENAMES:
            continue
        if path.name not in known_filenames:
            raise AggregationError(
                f"{target_dir}: unexpected file {path.name!r} has no matching "
                "release-manifest.json artifact entry — refusing to guess "
                "whether it is a public release asset"
            )
    for filename in known_filenames:
        if not (target_dir / filename).is_file():
            raise AggregationError(
                f"{target_dir}: manifest lists artifact {filename!r} but the "
                "file does not exist on disk"
            )


def aggregate(
    artifacts_dir: Path,
    expected_version: str,
    expected_tag: str,
    expected_git_sha: str,
    out_dir: Path,
) -> tuple[Path, Path]:
    validate_tag(expected_tag, expected_version)

    target_dirs = sorted(p for p in artifacts_dir.iterdir() if p.is_dir())
    if not target_dirs:
        raise AggregationError(f"no target subdirectories found under {artifacts_dir}")

    merged_artifacts: list[dict] = []
    seen_filenames: dict[str, Path] = {}  # artifact_filename -> source target_dir

    for target_dir in target_dirs:
        manifest = load_target_manifest(target_dir)
        validate_target_manifest(manifest, target_dir, expected_version, expected_git_sha)
        validate_no_unexpected_files(manifest, target_dir)

        for entry in manifest["artifacts"]:
            filename = entry["artifact_filename"]
            if filename in seen_filenames:
                raise AggregationError(
                    f"duplicate artifact filename {filename!r}: claimed by both "
                    f"{seen_filenames[filename]} and {target_dir}"
                )
            seen_filenames[filename] = target_dir
            merged_artifacts.append(entry)

    out_dir.mkdir(parents=True, exist_ok=True)
    for filename, target_dir in seen_filenames.items():
        shutil.copy2(target_dir / filename, out_dir / filename)

    release_manifest = {
        "schema": SCHEMA,
        "schema_version": SCHEMA_VERSION,
        "project_version": expected_version,
        "release_tag": expected_tag,
        "git_sha": expected_git_sha,
        "artifacts": merged_artifacts,
    }
    manifest_path = out_dir / "release-manifest.json"
    manifest_path.write_text(json.dumps(release_manifest, indent=2) + "\n")

    # Only now, after every artifact file and the merged manifest itself
    # are in their final place — a checksum computed any earlier would
    # describe a directory that doesn't match what actually ships.
    checksums_content = checksums.generate(out_dir)
    checksums_path = out_dir / "SHA256SUMS"
    checksums_path.write_text(checksums_content)

    return manifest_path, checksums_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--artifacts-dir", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--expected-tag", required=True)
    parser.add_argument("--expected-git-sha", required=True)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()

    manifest_path, checksums_path = aggregate(
        args.artifacts_dir,
        args.expected_version,
        args.expected_tag,
        args.expected_git_sha,
        args.out_dir,
    )
    print(f"wrote {manifest_path}")
    print(f"wrote {checksums_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
