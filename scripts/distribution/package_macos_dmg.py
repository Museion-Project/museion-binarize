#!/usr/bin/env python3
"""Builds a `.dmg` from an already-signed macOS `.app` bundle.

This exists because `pnpm tauri build --bundles dmg` (or `--bundles
all`) does not package an already-built `.app` — it recompiles and
re-bundles the `.app` from scratch as part of producing the `.dmg`,
which would silently discard the signature applied by
`sign_macos_app.py` and reintroduce the "is damaged and can't be
opened" bug that script fixes (see its docstring, and
docs/desktop-testing.md, "Milestone 7A"). Verified empirically: running
`--bundles app` then `--bundles dmg` back to back re-runs the "Bundling
Museion Binarize.app" step before producing the dmg.

So the packaged app and the packaged disk image must be built in this
order: `tauri build --bundles app`, sign the resulting `.app` in place,
then build the `.dmg` from that exact signed bundle — which this script
does directly with `hdiutil`, without going through Tauri's bundler
again. This produces a plain drag-to-Applications disk image (the
`.app` plus an `/Applications` symlink) — no custom background image or
icon layout, which Tauri's own dmg bundler provides but which this
project does not otherwise use.

Usage:
    python3 scripts/distribution/package_macos_dmg.py \\
        --app-path "target/aarch64-apple-darwin/release/bundle/macos/Museion Binarize.app" \\
        --version 0.1.0 --target-triple aarch64-apple-darwin \\
        --out-dir target/aarch64-apple-darwin/release/bundle/dmg
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import naming  # noqa: E402


def build_dmg(app_path: Path, version: str, target_triple: str, out_dir: Path) -> Path:
    if not app_path.is_dir() or app_path.suffix != ".app":
        raise SystemExit(f"not an .app bundle: {app_path}")

    out_dir.mkdir(parents=True, exist_ok=True)
    dmg_path = out_dir / naming.desktop_artifact_name(version, target_triple, "dmg")
    if dmg_path.exists():
        dmg_path.unlink()

    with tempfile.TemporaryDirectory() as staging:
        staging_path = Path(staging)
        shutil.copytree(app_path, staging_path / app_path.name)
        (staging_path / "Applications").symlink_to("/Applications")
        subprocess.run(
            [
                "hdiutil",
                "create",
                "-volname",
                app_path.stem,
                "-srcfolder",
                str(staging_path),
                "-ov",
                "-format",
                "UDZO",
                str(dmg_path),
            ],
            check=True,
        )

    return dmg_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-path", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target-triple", required=True)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    dmg_path = build_dmg(args.app_path, args.version, args.target_triple, args.out_dir)
    print(f"built {dmg_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
