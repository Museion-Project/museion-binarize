#!/usr/bin/env python3
"""Renders the user-facing GitHub Release body from the aggregated
release-manifest.json — never hand-typed in the publish workflow, so the
"Downloads" list can never drift from what was actually aggregated and
uploaded. See docs/releasing.md, "Draft release notes."

The static platform-status/limitations text below is deliberately
conservative: every claim in it must already be true as of M7A2 (macOS
arm64 human-tested, ad-hoc signed only, Windows/Linux not yet
human-tested, no OCR/AI/telemetry). If that state changes in a later
release, this text needs a human to update it — it does not infer
signing/testing state from the manifest, on purpose, since a manifest's
signing_state field describes code-signing, not human runtime
acceptance, and conflating the two is exactly the kind of overclaim
docs/desktop-testing.md's verification-state discipline exists to
prevent.

Usage:
    python3 scripts/distribution/render_release_notes.py \\
        --manifest release-assets/release-manifest.json \\
        --version 0.1.0-rc.1
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

TEMPLATE = """\
## What this is

M PDF Processor converts scanned scholarly books into clean, compact,
true 1-bit (bilevel) PDFs using deterministic thresholding — no OCR, no
AI, no generative restoration. This is a **public release candidate**.

## Highlights

- Deterministic Otsu / Sauvola / manual binarization, true 1-bit PDF
  reconstruction, CCITT Group 4 compression.
- Desktop GUI and CLI, sharing one processing core: open (including
  native single-PDF drag-and-drop), preview, estimate, convert, cancel.
- Every packaged download bundles its own pinned PDFium — no separate
  install needed.
- All processing is local. No upload, no telemetry, no network access
  at runtime.

## Downloads

{downloads_table}

## Platform status

- **macOS (Apple Silicon)** is the primary, human-validated platform for
  this release.
- **Windows and Linux** builds are release-candidate builds — they build
  and package successfully, but do not yet have human runtime
  acceptance.
- The macOS build is **ad-hoc signed**, not Developer ID signed or
  notarized. Right-click (Control-click) the app and choose **Open** on
  first launch. Do not disable Gatekeeper system-wide to work around
  this.
- The Windows installer and Linux packages are unsigned; no
  SmartScreen/reputation claim is made.

## Current limitations

- No OCR, no preservation of hidden OCR text layers.
- No AI, machine learning, or generative restoration of any kind.
- No page dewarping.
- No claim is made that this preserves polytonic Ancient Greek
  typography or critical apparatuses.

## Verification

Every file below is listed with its SHA-256 in `SHA256SUMS`
(`release-manifest.json` carries the same digests plus PDFium
provenance and signing state per artifact).

## Open source and supporting the project

M PDF Processor is free and open source, and the official GitHub
builds are fully functional and freely available.

GitHub Sponsors support is available at
https://github.com/sponsors/pei-haoran.

A paid Mac App Store edition is planned for later as a convenient
installation and update channel that also supports continued
development, not as a feature-gated replacement for the free GitHub
build.

## Source / license

Source: https://github.com/Museion-Project/museion-binarize at this
release's tag. Dual-licensed MIT OR Apache-2.0, at your option.
"""


def render(manifest: dict, version: str) -> str:
    rows = ["| File | Notes |", "|---|---|"]
    for artifact in manifest["artifacts"]:
        filename = artifact["artifact_filename"]
        signing = artifact["signing_state"]
        notarization = artifact["notarization_state"]
        note_parts = [f"signing: {signing}"]
        if notarization != "not_applicable":
            note_parts.append(f"notarization: {notarization}")
        rows.append(f"| `{filename}` | {', '.join(note_parts)} |")
    downloads_table = "\n".join(rows)
    return TEMPLATE.format(downloads_table=downloads_table)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text())
    if manifest.get("project_version") != args.version:
        raise SystemExit(
            f"manifest project_version {manifest.get('project_version')!r} != "
            f"--version {args.version!r}"
        )
    body = render(manifest, args.version)
    if args.out:
        args.out.write_text(body)
        print(f"wrote {args.out}")
    else:
        print(body)
    return 0


if __name__ == "__main__":
    sys.exit(main())
