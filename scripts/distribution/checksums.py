#!/usr/bin/env python3
"""Generates a SHA256SUMS file for a directory of release artifacts.

Must be run AFTER every mutation to an artifact (signing, notarization
stapling, ...) — a checksum computed before those steps describes a
different file and must never be published as if it described the final
one. See docs/releasing.md, "Checksums."

Usage:
    python3 scripts/distribution/checksums.py <artifacts-dir> [--out FILE]
"""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def generate(artifacts_dir: Path) -> str:
    lines = []
    files = sorted(
        p for p in artifacts_dir.iterdir() if p.is_file() and p.name != "SHA256SUMS"
    )
    if not files:
        raise SystemExit(f"error: no artifact files found in {artifacts_dir}")
    for path in files:
        lines.append(f"{sha256_of(path)}  {path.name}")
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts_dir", type=Path)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()

    content = generate(args.artifacts_dir)
    out_path = args.out or (args.artifacts_dir / "SHA256SUMS")
    out_path.write_text(content)
    print(f"wrote {out_path}")
    print(content, end="")


if __name__ == "__main__":
    main()
