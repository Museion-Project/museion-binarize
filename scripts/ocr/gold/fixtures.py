#!/usr/bin/env python3
"""Deterministic renderer for the OCR gold corpus.

Fixtures are rendered, never committed: the ground truth lives in
``corpus.py`` and the pixels are reproduced on demand from the OFL-licensed
Noto Sans vendored at ``crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf``.
That keeps the repository free of third-party page images while still giving
every machine byte-identical inputs.

Rendering is intentionally plain (black text on white, no anti-alias tricks,
no engine-specific preprocessing) so the numbers compare recognizers, not
fixture tuning.
"""

from __future__ import annotations

import hashlib
import os
import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

REPO_ROOT = Path(__file__).resolve().parents[3]
FONT_PATH = REPO_ROOT / "crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf"
FONT_LICENSE = REPO_ROOT / "crates/mpdf-core/assets/fonts/OFL.txt"

#: Point size at the canonical 300 dpi; scaled linearly for other DPIs.
BASE_POINT_SIZE = 11.0
CANONICAL_DPI = 300
MARGIN_INCHES = 0.5
LINE_SPACING = 1.55


def font_for(dpi: int) -> ImageFont.FreeTypeFont:
    pixels = max(8, round(BASE_POINT_SIZE * dpi / 72.0))
    return ImageFont.truetype(str(FONT_PATH), pixels)


def render_sample(sample, output_path: Path) -> Path:
    """Renders one corpus sample to a deterministic grayscale PNG."""
    if not FONT_PATH.is_file():
        raise FileNotFoundError(f"gold fixture font is missing: {FONT_PATH}")
    font = font_for(sample.dpi)
    margin = round(MARGIN_INCHES * sample.dpi)
    line_step = round(font.size * LINE_SPACING)

    rendered_lines = list(sample.lines)
    if sample.columns > 1:
        # A "|" in the source marks the column break; the gold text keeps it
        # so the reading-order expectation stays explicit and checkable.
        left = [line.split("|")[0].rstrip() for line in rendered_lines]
        right = [line.split("|")[-1].strip() for line in rendered_lines]
        column_width = max(
            max((round(font.getlength(line)) for line in left), default=0),
            max((round(font.getlength(line)) for line in right), default=0),
        )
        gutter = round(0.35 * sample.dpi)
        width = 2 * margin + 2 * column_width + gutter
        height = 2 * margin + line_step * len(rendered_lines)
        image = Image.new("L", (width, height), 255)
        draw = ImageDraw.Draw(image)
        for index, (left_line, right_line) in enumerate(zip(left, right)):
            y = margin + index * line_step
            draw.text((margin, y), left_line, font=font, fill=0)
            draw.text((margin + column_width + gutter, y), right_line, font=font, fill=0)
    else:
        width = 2 * margin + max(
            (round(font.getlength(line)) for line in rendered_lines), default=1
        )
        height = 2 * margin + line_step * len(rendered_lines)
        image = Image.new("L", (width, height), 255)
        draw = ImageDraw.Draw(image)
        for index, line in enumerate(rendered_lines):
            draw.text((margin, margin + index * line_step), line, font=font, fill=0)

    if sample.noise > 0.0:
        # Seeded from the sample id so the same fixture always carries the
        # same speckle; a candidate cannot get lucky on a rerun.
        rng = random.Random(
            int.from_bytes(
                hashlib.sha256(sample.sample_id.encode("utf-8")).digest()[:8], "big"
            )
        )
        pixels = image.load()
        count = round(width * height * sample.noise)
        for _ in range(count):
            pixels[rng.randrange(width), rng.randrange(height)] = rng.choice((0, 255))

    if sample.rotation:
        image = image.rotate(-sample.rotation, expand=True, fillcolor=255)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    # `optimize` is off so the bytes depend only on the pixels and Pillow's
    # deterministic default filter choice.
    image.save(output_path, format="PNG")
    return output_path


def render_all(samples, directory: Path) -> dict[str, Path]:
    """Renders every sample into ``directory`` and returns id -> path."""
    directory.mkdir(parents=True, exist_ok=True)
    rendered: dict[str, Path] = {}
    for sample in samples:
        rendered[sample.sample_id] = render_sample(
            sample, directory / f"{sample.sample_id}.png"
        )
    return rendered


def fixture_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    import argparse
    import sys

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import corpus  # noqa: PLC0415

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    rendered = render_all(corpus.ALL_SAMPLES, args.out)
    for sample_id, path in sorted(rendered.items()):
        print(f"{fixture_digest(path)}  {sample_id}  {path.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
