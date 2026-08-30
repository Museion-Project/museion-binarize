#!/usr/bin/env python3
"""Run all release PDFium integration gates for a provisioned target."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pdfium", type=Path, required=True)
    parser.add_argument("--target", help="target triple recorded in optional evidence")
    parser.add_argument("--evidence", type=Path, help="write evidence JSON only after all gates pass")
    args = parser.parse_args()
    if not args.pdfium.is_file():
        raise SystemExit(f"provisioned PDFium library not found: {args.pdfium}")
    env = os.environ.copy()
    # Pass an absolute path so each cargo invocation (including Windows
    # PowerShell jobs) observes the exact provisioned file independent of its
    # working directory. No download or fallback is attempted here.
    env["MPDF_PDFIUM_LIBRARY"] = str(args.pdfium.resolve())
    commands = [
        ["cargo", "test", "--offline", "-p", "mpdf-core", "--test", "auto_bookmarks_pdf", "--", "--ignored"],
        ["cargo", "test", "--offline", "-p", "mpdf-cli", "--test", "bookmarks_cli", "--", "--ignored"],
        ["cargo", "test", "--offline", "-p", "mpdf-core", "--test", "searchable_pdf"],
        ["cargo", "test", "--offline", "-p", "mpdf-core", "--test", "pdf_pipeline", "--", "--ignored"],
    ]
    for command in commands:
        subprocess.run(command, env=env, check=True)
    if args.evidence:
        target = args.target or args.pdfium.parent.name
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(json.dumps({
            "target": target,
            "library_sha256": hashlib.sha256(args.pdfium.read_bytes()).hexdigest(),
            "gates": {
                "core_auto_bookmarks_pdf_ignored": "pass",
                "cli_bookmarks_cli_ignored": "pass",
                "core_searchable_pdf": "pass",
                "core_pdf_pipeline_ignored": "pass",
            },
        }, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
