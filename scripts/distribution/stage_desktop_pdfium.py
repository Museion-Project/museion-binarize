#!/usr/bin/env python3
"""Fetches (via `fetch_pdfium.py`) and stages the pinned PDFium library
where Tauri's `bundle.resources` config picks it up for packaging.

Only one target's library is staged at a time, at a fixed relative path
(`apps/desktop/src-tauri/resources/pdfium/<library-filename>`), so
`tauri.conf.json` can reference it with one static glob regardless of
which platform is currently being packaged. This directory is
gitignored — never commit a staged binary.

Usage:
    python3 scripts/distribution/stage_desktop_pdfium.py <target-triple>
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fetch_pdfium  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
RESOURCE_DIR = REPO_ROOT / "apps" / "desktop" / "src-tauri" / "resources" / "pdfium"


def stage(target_triple: str) -> Path:
    staged_library = fetch_pdfium.fetch_and_verify(target_triple, fetch_pdfium.DEFAULT_DEST)

    if RESOURCE_DIR.exists():
        shutil.rmtree(RESOURCE_DIR)
    RESOURCE_DIR.mkdir(parents=True)

    destination = RESOURCE_DIR / staged_library.name
    shutil.copy2(staged_library, destination)
    print(f"staged for Tauri bundling: {destination}", file=sys.stderr)
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target_triple", help="e.g. aarch64-apple-darwin")
    args = parser.parse_args()
    stage(args.target_triple)


if __name__ == "__main__":
    main()
