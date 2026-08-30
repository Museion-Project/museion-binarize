#!/usr/bin/env python3
"""Scores the OCR sidecar against the CGPG corpus, combined vs routed.

This is the external counterpart to ``run_eval.py``.  The synthetic gold set
measures whether a change broke something this repository already knew how to
do; CGPG measures whether it helps on human-transcribed material this
repository did not produce.

Regions, not pages, are the unit, and each region is bucketed by the script of
its *human transcription*.  That keeps the router scored on exactly what it
claims to do -- improve the Greek without touching the Latin -- where a
page-level number would let a Greek gain hide a Latin regression.

The bucket cannot come from the corpus's own ``MainText_ColGreek`` /
``MainText_ColLatin`` labels: those exist only on the 38 layout-only pages,
while all 268 transcribed regions are typed generically as ``text``.  See
``cgpg.script_of_text``.

Usage::

    python3 scripts/ocr/gold/run_cgpg.py \\
        --corpus /tmp/cgpg/data --models /path/to/tessdata_best \\
        --split dev --limit 24 --json out.json

The corpus lives in a temporary directory and is never read by the
application.  See ``cgpg.py`` for provenance and licensing.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import metrics as metrics_module  # noqa: E402

SIDECAR = Path(__file__).resolve().parents[1] / "mpdf_ocr_sidecar.py"
PROTOCOL = "mpdf-ocr"
PROTOCOL_VERSION = "0.1"


def recognize(
    image: Path, models: Path, routing: str, profile: str, psm: int
) -> tuple[dict | None, str | None, float]:
    request = json.dumps(
        {
            "protocol": PROTOCOL,
            "protocol_version": PROTOCOL_VERSION,
            "page_index": 0,
            "input_asset_sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "language_profile": profile,
        }
    )
    argv = [
        sys.executable, str(SIDECAR),
        "--protocol", PROTOCOL,
        "--protocol-version", PROTOCOL_VERSION,
        "--model-dir", str(models),
        "--input", str(image),
        "--engine", "tesseract",
        "--language-profile", profile,
        "--psm", str(psm),
        "--routing", "off",
        "--small-type-latin", "off",
        "--detached-greek-accents", "off",
    ]
    if routing == "block-script-v1":
        argv[-5] = routing
    elif routing == "detached":
        argv[-1] = "on"
    started = time.perf_counter()
    completed = subprocess.run(
        argv, input=request + "\n", capture_output=True, text=True,
        timeout=900, check=False,
    )
    elapsed = time.perf_counter() - started
    if completed.returncode != 0:
        return None, (completed.stderr.strip() or "provider failed"), elapsed
    try:
        return json.loads(completed.stdout), None, elapsed
    except json.JSONDecodeError as error:
        return None, f"unparseable response: {error}", elapsed


def _centre(bbox: dict) -> tuple[float, float]:
    return bbox["x"] + bbox["width"] / 2.0, bbox["y"] + bbox["height"] / 2.0


def lines_in_region(response: dict, region: cgpg.Region) -> list[str]:
    """Recognized lines whose centre falls inside the labelled region.

    Centre containment rather than overlap: a line that merely grazes the
    column edge belongs to the neighbouring column, and counting it in both
    would flatter a router that leaks across the gutter.
    """
    left, top, right, bottom = region.bbox
    rows: list[tuple[int, int, str]] = []
    for block in response.get("blocks", []):
        for line in block.get("lines", []):
            words = line.get("words", [])
            if not words:
                continue
            x, y = _centre(line["bbox"])
            if not (left <= x <= right and top <= y <= bottom):
                continue
            text = " ".join(word["text"] for word in words).strip()
            if text:
                rows.append((block["reading_order"], line["reading_order"], text))
    rows.sort(key=lambda row: (row[0], row[1]))
    return [row[2] for row in rows]


def score_page(page: cgpg.Page, response: dict) -> dict:
    """Scores each transcribed region, bucketed by its ground-truth script."""
    per_script: dict[str, metrics_module.Metrics] = {}
    missing = extra = 0
    for region in page.regions:
        reference = region.text
        if not reference.strip():
            continue
        # Labelled from the human transcription, never from the OCR output:
        # the corpus's own ColGreek/ColLatin labels sit on layout-only pages
        # and never coincide with a transcription. See cgpg.script_of_text.
        script = cgpg.script_of_text(reference)
        hypothesis = "\n".join(lines_in_region(response, region))
        result = metrics_module.score(reference, hypothesis)
        per_script[script] = (
            result if script not in per_script else per_script[script].merge(result)
        )
        reference_lines = len([line for line in region.lines if line.text])
        hypothesis_lines = len(hypothesis.splitlines()) if hypothesis else 0
        missing += max(0, reference_lines - hypothesis_lines)
        extra += max(0, hypothesis_lines - reference_lines)
    return {
        "per_script": {name: value for name, value in per_script.items()},
        "missing_lines": missing,
        "extra_lines": extra,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--models", required=True, type=Path)
    parser.add_argument("--split", default="dev", choices=("dev", "holdout", "all"))
    parser.add_argument("--limit", type=int, default=24)
    parser.add_argument("--profile", default="auto")
    parser.add_argument("--psm", type=int, default=6)
    parser.add_argument(
        "--routing", action="append", default=None,
        help="routing mode to evaluate; repeat for an ablation",
    )
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    routings = args.routing or ["off", "block-script-v1"]
    pages = cgpg.load_corpus(args.corpus)
    dev, holdout = cgpg.split_corpus(pages)
    chosen = {"dev": dev, "holdout": holdout, "all": pages}[args.split]
    # Only pages that carry line transcriptions can be scored; the remaining
    # 38 are layout annotation.
    chosen = [page for page in chosen if page.transcribed_lines]
    chosen = chosen[: args.limit]

    report: dict = {
        "schema": "mpdf-cgpg-eval",
        "schema_version": "1.0",
        "corpus": {
            "doi": "10.5281/zenodo.20008699",
            "archive_sha256": (
                "2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7"
            ),
            "license": "CC BY 4.0",
            "pages_total": len(pages),
            "dev_pages": len(dev),
            "holdout_pages": len(holdout),
        },
        "split": args.split,
        "pages_scored": [page.name for page in chosen],
        "profile": args.profile,
        "psm": args.psm,
        "note": (
            "The public package is not the paper's 30-page test split; these "
            "numbers may not be compared with its headline accuracy."
        ),
        "candidates": {},
    }

    for routing in routings:
        totals: dict[str, metrics_module.Metrics] = {}
        missing = extra = failures = 0
        seconds = 0.0
        rerouted = considered = 0
        for page in chosen:
            response, error, elapsed = recognize(
                page.image_path, args.models, routing, args.profile, args.psm
            )
            seconds += elapsed
            if response is None:
                failures += 1
                print(f"  ! {page.name}: {error}", file=sys.stderr)
                continue
            for decision in response.get("routing_decisions", []):
                considered += 1
                if decision.get("selected_profile") == "grc":
                    rerouted += 1
            scored = score_page(page, response)
            for name, value in scored["per_script"].items():
                totals[name] = (
                    value if name not in totals else totals[name].merge(value)
                )
            missing += scored["missing_lines"]
            extra += scored["extra_lines"]
            print(
                f"  {routing:16s} {page.name:34s} {elapsed:6.1f}s", file=sys.stderr
            )
        report["candidates"][routing] = {
            "seconds_total": round(seconds, 2),
            "seconds_per_page": round(seconds / max(1, len(chosen)), 3),
            "pages": len(chosen),
            "failures": failures,
            "missing_lines": missing,
            "extra_lines": extra,
            "regions_considered": considered,
            "regions_rerouted": rerouted,
            "by_script": {
                name: value.as_dict() for name, value in sorted(totals.items())
            },
        }

    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.json:
        args.json.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
