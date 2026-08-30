#!/usr/bin/env python3
"""Render the user-facing GitHub Release body from the aggregated manifest.

The rc.3 macOS trust wording is derived from actual two-architecture DMG
entries; signing metadata is not treated as human runtime acceptance.
The body is never hand-typed in the publish workflow, so the
"Downloads" list can never drift from what was actually aggregated and
uploaded. See docs/releasing.md, "Draft release notes."

Platform wording remains deliberately conservative: signing metadata is
not runtime acceptance evidence, and the rc.3 macOS trust sentence is
derived only from manifest states.

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
- Every executable package bundles its own pinned PDFium — no separate
  install needed. SBOM files are dependency inventories, not executables.
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
- Binarization and bookmark decisions are deterministic; OCR can use an
  explicitly configured user-provided RapidOCR model. No generative
  restoration or inpainting is performed.
- No page dewarping.
- No claim is made that this preserves polytonic Ancient Greek
  typography or critical apparatuses.

## Verification

Every file below is listed with its SHA-256 in `SHA256SUMS`
(`release-manifest.json` carries the same digests plus PDFium
provenance and signing state per artifact). Each target's deterministic
SPDX 2.3 SBOM is included in the asset list and is labeled as an SBOM,
not as an executable containing PDFium.

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
        signing = artifact.get("signing_state", "unsigned")
        notarization = artifact.get("notarization_state", "not_applicable")
        if artifact.get("artifact_kind", "desktop") == "sbom":
            note_parts = ["SBOM (not an executable)"]
        else:
            note_parts = [f"signing: {signing}"]
        if notarization != "not_applicable":
            note_parts.append(f"notarization: {notarization}")
        rows.append(f"| `{filename}` | {', '.join(note_parts)} |")
    downloads_table = "\n".join(rows)
    body = TEMPLATE.format(downloads_table=downloads_table)
    # Keep rc.1/rc.2 historical release notes reproducible while making the
    # rc.3 source's actual feature/privacy state accurate.
    if version.endswith("-rc.3"):
        dmgs = [a for a in manifest.get("artifacts", [])
                if a.get("artifact_kind", "desktop") == "desktop"
                and a.get("os") == "macos"
                and a.get("artifact_filename", "").endswith(".dmg")]
        targets = {a.get("target_triple") for a in dmgs}
        known_targets = {target for target in targets if target}
        expected_targets = {"aarch64-apple-darwin", "x86_64-apple-darwin"}
        states = {(a.get("signing_state"), a.get("notarization_state", "not_applicable")) for a in dmgs}
        # v1.0 manifests lack target metadata; retain compatibility for that
        # legacy shape while requiring both explicit rc.3 architectures when
        # target triples are present.
        if (known_targets == expected_targets or not known_targets) and states == {("signed", "notarized")}:
            mac_status = "The macOS DMG is Developer ID signed and notarized."
        elif (known_targets == expected_targets or not known_targets) and states == {("ad_hoc", "pending_credentials")}:
            mac_status = "The macOS DMG is ad-hoc signed and pending Developer ID/notarization credentials."
        elif dmgs:
            detail = ", ".join(f"{signing or 'unknown'}/{notary or 'unknown'}"
                               for signing, notary in sorted(states))
            mac_status = f"macOS DMG signing/notarization states are mixed ({detail}); no uniform trust claim is made."
        else:
            mac_status = "No macOS DMG signing evidence is present in the manifest; no trust claim is made."
        body = body.replace(
            "true 1-bit (bilevel) PDFs using deterministic thresholding — no OCR, no\nAI, no generative restoration.",
            "true 1-bit (bilevel) PDFs using deterministic binarization and\nbookmark decisions, with evidence packages, native-text searchable output,\nand automatic bookmarks v2. The base artifact does not require OCR.",
        )
        body = body.replace(
            "- All processing is local. No upload, no telemetry, no network access\n  at runtime.",
            "- Base conversion and native-text processing are offline. No cloud OCR\n  path is available: mpdf-credits has no production service and Gemini BYOK\n  is disabled. Cloud bookmark generation does not exist.",
        )
        body = body.replace(
            "- No OCR, no preservation of hidden OCR text layers.",
            "- The base artifact does not include an OCR runtime. Native-text PDFs\n  work without one; scanned-page OCR requires the separate optional local OCR\n  plugin and fails explicitly when that plugin is absent.",
        )
        body = body.replace(
            "- The macOS build is **ad-hoc signed**, not Developer ID signed or\n  notarized. Right-click (Control-click) the app and choose **Open** on\n  first launch. Do not disable Gatekeeper system-wide to work around\n  this.",
            f"- {mac_status} Right-click (Control-click) the app and choose **Open**\n  on first launch when using an ad-hoc build. Do not disable Gatekeeper\n  system-wide to work around this.",
        )
    return body


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
