#!/usr/bin/env python3
"""Copy an already verified OCR runtime into Tauri's stable resource path.

The source runtime must have passed ``verify_ocr_runtime.py``.  This command
does not download or build anything and intentionally writes only the ignored
``apps/desktop/src-tauri/resources/ocr-runtime`` staging directory.
"""
from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import verify_ocr_runtime  # noqa: E402

ROOT = HERE.parents[1]
DEST = ROOT / "apps/desktop/src-tauri/resources/ocr-runtime"


def stage(runtime: Path, target: str, *, pinned_models: Path | None = None,
          release: str = "0.1.0-rc.3") -> Path:
    verify_ocr_runtime.verify(runtime, target, pinned_models or verify_ocr_runtime.DEFAULT_PINNED_MODELS,
                              expected_release=release)
    if DEST.exists():
        if DEST.is_symlink() or not DEST.is_dir():
            raise ValueError(f"Tauri OCR resource path is not a directory: {DEST}")
        shutil.rmtree(DEST)
    shutil.copytree(runtime, DEST, symlinks=False)
    return DEST


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--release", default="0.1.0-rc.3")
    parser.add_argument("--pinned-models", type=Path)
    args = parser.parse_args()
    try:
        print(stage(args.runtime_root, args.target, pinned_models=args.pinned_models, release=args.release))
    except (OSError, ValueError) as exc:
        print(f"desktop OCR staging failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
