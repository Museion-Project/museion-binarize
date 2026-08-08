#!/usr/bin/env python3
"""Copies exactly one built desktop installer from Tauri's bundle output
directory into `dist-out/`, under this project's deterministic naming
convention (`naming.py`). Shared by the Windows and Linux distribution
jobs — mirrors the same "expected exactly one match, fail loudly
otherwise" discipline the macOS job's inline dmg-collector already uses
(see `docs/desktop-testing.md`, "Milestone 7A," for why that discipline
exists: a stale leftover artifact silently picked from a bundle
directory is exactly how a broken build once nearly shipped again).

Usage:
    python3 scripts/distribution/collect_desktop_artifact.py \\
        --bundle-dir target/x86_64-pc-windows-msvc/release/bundle/msi \\
        --glob "*.msi" \\
        --version 0.1.0-rc.1 --target-triple x86_64-pc-windows-msvc \\
        --ext msi --out-dir dist-out
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import naming  # noqa: E402


def collect(bundle_dir: Path, glob: str, version: str, target_triple: str, ext: str, out_dir: Path) -> Path:
    matches = sorted(bundle_dir.glob(glob)) if bundle_dir.is_dir() else []
    if len(matches) != 1:
        raise SystemExit(
            f"expected exactly 1 match for {glob!r} in {bundle_dir}, found {len(matches)}: {matches}"
        )
    out_dir.mkdir(parents=True, exist_ok=True)
    dest = out_dir / naming.desktop_artifact_name(version, target_triple, ext)
    shutil.copy2(matches[0], dest)
    return dest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-dir", required=True, type=Path)
    parser.add_argument("--glob", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target-triple", required=True)
    parser.add_argument("--ext", required=True)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    dest = collect(args.bundle_dir, args.glob, args.version, args.target_triple, args.ext, args.out_dir)
    print(f"copied {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
