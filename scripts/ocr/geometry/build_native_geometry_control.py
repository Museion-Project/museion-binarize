#!/usr/bin/env python3
"""Build a text-free geometry control from a born-digital PDF.

The PDF's visible text objects seed line rectangles and content-stream reading
order. This is a clean native control, not human closed-world gold and not
evidence that a detector works on historical scans. No extracted text is
written to the control manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from pathlib import Path

import fitz


SCHEMA = "mpdf-native-geometry-control/1"
DEFAULT_PAGES = (8, 14, 85, 100, 190, 259, 323, 324)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, ensure_ascii=False, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def visible_lines(page: fitz.Page, scale: float) -> list[dict]:
    """Extract visible PDF text lines in content-stream order."""
    result = []
    tree = page.get_text("dict", sort=False)
    for block in tree.get("blocks", []):
        if block.get("type") != 0:
            continue
        for line in block.get("lines", []):
            spans = [span for span in line.get("spans", []) if span.get("text", "").strip()]
            if not spans:
                continue
            left = min(float(span["bbox"][0]) for span in spans)
            top = min(float(span["bbox"][1]) for span in spans)
            right = max(float(span["bbox"][2]) for span in spans)
            bottom = max(float(span["bbox"][3]) for span in spans)
            # Hashes help diagnose extraction changes without redistributing text.
            text = "".join(span.get("text", "") for span in spans)
            result.append(
                {
                    "line_id": f"p{page.number + 1:04d}-l{len(result):04d}",
                    "reading_order": len(result),
                    "bbox": [
                        round(left * scale, 3),
                        round(top * scale, 3),
                        round(right * scale, 3),
                        round(bottom * scale, 3),
                    ],
                    "text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                }
            )
    return result


def build(pdf: Path, output: Path, pages: tuple[int, ...], dpi: int) -> dict:
    if dpi <= 0:
        raise ValueError("dpi must be positive")
    document = fitz.open(pdf)
    if len(set(pages)) != len(pages) or any(page < 1 or page > len(document) for page in pages):
        raise ValueError("pages must be unique, one-based, and inside the PDF")
    output.mkdir(parents=True, exist_ok=True)
    scale = dpi / 72.0
    page_records = []
    for page_number in pages:
        page = document[page_number - 1]
        image_path = output / f"page-{page_number:04d}.png"
        pixmap = page.get_pixmap(matrix=fitz.Matrix(scale, scale), alpha=False)
        pixmap.save(image_path)
        lines = visible_lines(page, scale)
        if not lines:
            raise ValueError(f"page {page_number} has no visible native text lines")
        page_records.append(
            {
                "page_number": page_number,
                "image": {
                    "file": image_path.name,
                    "sha256": sha256_file(image_path),
                    "width": pixmap.width,
                    "height": pixmap.height,
                    "dpi": dpi,
                },
                "coverage": {
                    "kind": "native_pdf_visible_text_candidate",
                    "human_closed_world_verified": False,
                },
                "lines": lines,
            }
        )
    manifest = {
        "schema": SCHEMA,
        "source_pdf_sha256": sha256_file(pdf),
        "source_page_count": len(document),
        "selection": list(pages),
        "reference": {
            "geometry": "visible PDF text-object line boxes",
            "reading_order": "PDF content-stream order",
            "transcription_included": False,
            "historical_material_validated": False,
            "warning": "Clean born-digital control only; not human closed-world gold.",
        },
        "pages": page_records,
    }
    atomic_json(output / "manifest.json", manifest)
    return manifest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pdf", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--pages", nargs="+", type=int, default=DEFAULT_PAGES)
    parser.add_argument("--dpi", type=int, default=300)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = build(args.pdf.resolve(), args.output.resolve(), tuple(args.pages), args.dpi)
    print(
        json.dumps(
            {
                "manifest": str((args.output / "manifest.json").resolve()),
                "pages": len(manifest["pages"]),
                "lines": sum(len(page["lines"]) for page in manifest["pages"]),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
