#!/usr/bin/env python3
"""Generate a four-page synthetic scanned-text PDF for installed OCR smoke."""
from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

PAGES = (
    ("ΕΛΛΗΝΙΚΑ", "Πλάτωνος Σοφιστής", "τὸ ὂν καὶ τὸ μὴ ὂν", "λόγος ἀλήθεια ψυχὴ σοφία"),
    ("DEUTSCH", "Über das Sein und Nichtsein", "Die Frage nach Wahrheit und Erkenntnis", "Zwölf Bücher begründen die Untersuchung"),
    ("LATINA", "Platonis Sophista", "de ente et non ente disputatio", "ratio veritas anima sapientia"),
    ("MIXED", "Σοφιστής — Der Sophist — Sophista", "ἀλήθεια · Wahrheit · veritas", "ψυχὴ · Seele · anima"),
)


def generate(output: Path, font_path: Path) -> None:
    if output.exists():
        raise ValueError("smoke fixture output already exists")
    font = ImageFont.truetype(str(font_path), 58)
    heading = ImageFont.truetype(str(font_path), 78)
    images = []
    for page_number, lines in enumerate(PAGES, 1):
        image = Image.new("RGB", (1600, 2200), "white")
        draw = ImageDraw.Draw(image)
        draw.text((140, 180), lines[0], font=heading, fill="black")
        y = 430
        for line in lines[1:]:
            draw.text((140, y), line, font=font, fill="black")
            y += 180
        for repeat in range(5):
            draw.text((140, 1050 + repeat * 150), lines[1 + repeat % 3], font=font, fill="black")
        draw.text((760, 2050), str(page_number), font=font, fill="black")
        images.append(image)
    output.parent.mkdir(parents=True, exist_ok=True)
    images[0].save(output, "PDF", resolution=200.0, save_all=True, append_images=images[1:])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--font", type=Path, required=True)
    args = parser.parse_args()
    try:
        generate(args.output, args.font)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"fixture generation failed: {exc}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
