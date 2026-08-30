#!/usr/bin/env python3
"""Compare runnable line-geometry candidates on one native control manifest."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


SCHEMA = "mpdf-geometry-bakeoff/1"


def bbox_iou(left: list[float], right: list[float]) -> tuple[float, float, float]:
    ix0, iy0 = max(left[0], right[0]), max(left[1], right[1])
    ix1, iy1 = min(left[2], right[2]), min(left[3], right[3])
    intersection = max(0.0, ix1 - ix0) * max(0.0, iy1 - iy0)
    left_area = max(0.0, left[2] - left[0]) * max(0.0, left[3] - left[1])
    right_area = max(0.0, right[2] - right[0]) * max(0.0, right[3] - right[1])
    union = left_area + right_area - intersection
    return (
        intersection / union if union else 0.0,
        intersection / left_area if left_area else 0.0,
        intersection / right_area if right_area else 0.0,
    )


def match_boxes(gold: list[dict], candidate: list[dict], threshold: float = 0.30) -> list[dict]:
    proposals = []
    for gold_index, gold_line in enumerate(gold):
        for candidate_index, candidate_line in enumerate(candidate):
            iou, gold_coverage, candidate_precision = bbox_iou(
                gold_line["bbox"], candidate_line["bbox"]
            )
            if iou >= threshold:
                proposals.append(
                    (iou, gold_coverage, candidate_precision, gold_index, candidate_index)
                )
    proposals.sort(reverse=True)
    used_gold, used_candidate, matches = set(), set(), []
    for iou, coverage, precision, gold_index, candidate_index in proposals:
        if gold_index in used_gold or candidate_index in used_candidate:
            continue
        used_gold.add(gold_index)
        used_candidate.add(candidate_index)
        matches.append(
            {
                "gold": gold_index,
                "candidate": candidate_index,
                "iou": iou,
                "gold_coverage": coverage,
                "candidate_precision": precision,
            }
        )
    return matches


def kendall_tau(matches: list[dict]) -> float:
    ordered = [
        match["gold"] for match in sorted(matches, key=lambda item: item["candidate"])
    ]
    pairs = len(ordered) * (len(ordered) - 1) // 2
    if pairs == 0:
        return 1.0
    inversions = sum(
        ordered[left] > ordered[right]
        for left in range(len(ordered))
        for right in range(left + 1, len(ordered))
    )
    return 1.0 - 2.0 * inversions / pairs


def evaluate_page(gold: list[dict], candidate: list[dict]) -> dict:
    matches = match_boxes(gold, candidate)
    precision = len(matches) / len(candidate) if candidate else 0.0
    recall = len(matches) / len(gold) if gold else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {
        "gold_lines": len(gold),
        "candidate_lines": len(candidate),
        "matched_lines": len(matches),
        "precision": precision,
        "recall": recall,
        "f1": f1,
        "mean_iou": statistics.fmean(match["iou"] for match in matches) if matches else 0.0,
        "mean_gold_coverage": (
            statistics.fmean(match["gold_coverage"] for match in matches) if matches else 0.0
        ),
        "reading_order_tau": kendall_tau(matches),
    }


def tesseract_lines(
    image: Path, executable: Path, tessdata: Path | None, psm: int
) -> list[dict]:
    # Do not use the special ``stdout`` output base here. Some Homebrew
    # Tesseract/config combinations mix binary renderer output into stdout,
    # which makes a geometry-only runner needlessly dependent on decoding it.
    with tempfile.TemporaryDirectory(prefix="mpdf-tesseract-geometry-") as temp_dir:
        output_base = Path(temp_dir) / "page"
        command = [
            str(executable),
            str(image),
            str(output_base),
            "--oem",
            "1",
            "--psm",
            str(psm),
        ]
        if tessdata:
            command.extend(["--tessdata-dir", str(tessdata)])
        # The shipped runtime contains traineddata only, not Tesseract's
        # ``configs/tsv`` file. Set the renderer flag directly so the benchmark
        # exercises the same redistributable runtime layout as the product.
        command.extend(["-l", "grc+eng", "-c", "tessedit_create_tsv=1"])
        subprocess.run(command, check=True, capture_output=True)
        result = []
        with output_base.with_suffix(".tsv").open(encoding="utf-8", errors="replace") as stream:
            for row in csv.DictReader(stream, delimiter="\t"):
                if row.get("level") != "4":
                    continue
                left, top = float(row["left"]), float(row["top"])
                width, height = float(row["width"]), float(row["height"])
                if width <= 0 or height <= 0:
                    continue
                result.append({"bbox": [left, top, left + width, top + height]})
        return result


def paddle_detector(model_dir: Path):
    os.environ.setdefault("MPLCONFIGDIR", tempfile.mkdtemp(prefix="mpdf-mpl-"))
    from paddleocr import TextDetection

    return TextDetection(model_dir=str(model_dir))


def paddle_lines(detector, image: Path) -> list[dict]:
    result = []
    for page in detector.predict(str(image)):
        payload = page.json.get("res", page.json) if hasattr(page, "json") else page
        for polygon in payload.get("dt_polys", []):
            xs = [float(point[0]) for point in polygon]
            ys = [float(point[1]) for point in polygon]
            result.append({"bbox": [min(xs), min(ys), max(xs), max(ys)]})
    # Paddle does not own reading order; this is the frozen deterministic
    # baseline orderer for the control, and is expected to fail on columns.
    result.sort(key=lambda line: (round(line["bbox"][1] / 12), line["bbox"][0]))
    return result


def apple_lines(executable: Path, image: Path) -> list[dict]:
    completed = subprocess.run(
        [str(executable), str(image)], check=True, capture_output=True, text=True
    )
    return json.loads(completed.stdout)["lines"]


def canonical_lines(lines: list[dict]) -> str:
    rounded = [
        [round(float(value), 3) for value in line["bbox"]]
        for line in lines
    ]
    return hashlib.sha256(json.dumps(rounded, separators=(",", ":")).encode()).hexdigest()


def aggregate(pages: list[dict], durations: list[float], deterministic: bool) -> dict:
    totals = {
        "gold_lines": sum(page["gold_lines"] for page in pages),
        "candidate_lines": sum(page["candidate_lines"] for page in pages),
        "matched_lines": sum(page["matched_lines"] for page in pages),
    }
    precision = totals["matched_lines"] / totals["candidate_lines"] if totals["candidate_lines"] else 0
    recall = totals["matched_lines"] / totals["gold_lines"] if totals["gold_lines"] else 0
    return {
        **totals,
        "precision": precision,
        "recall": recall,
        "f1": 2 * precision * recall / (precision + recall) if precision + recall else 0,
        "mean_iou": statistics.fmean(page["mean_iou"] for page in pages),
        "mean_gold_coverage": statistics.fmean(page["mean_gold_coverage"] for page in pages),
        "mean_reading_order_tau": statistics.fmean(page["reading_order_tau"] for page in pages),
        "median_latency_seconds": statistics.median(durations),
        "max_latency_seconds": max(durations),
        "exact_repeat_deterministic": deterministic,
    }


def run_candidate(name: str, pages: list[dict], root: Path, runner) -> dict:
    page_results, durations, deterministic = [], [], True
    for page in pages:
        image = root / page["image"]["file"]
        attempts = []
        for _ in range(2):
            started = time.perf_counter()
            lines = runner(image)
            durations.append(time.perf_counter() - started)
            attempts.append(lines)
        deterministic &= canonical_lines(attempts[0]) == canonical_lines(attempts[1])
        metrics = evaluate_page(page["lines"], attempts[0])
        metrics["page_number"] = page["page_number"]
        page_results.append(metrics)
    return {
        "candidate": name,
        "pages": page_results,
        "aggregate": aggregate(page_results, durations, deterministic),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--tesseract", type=Path)
    parser.add_argument("--tessdata", type=Path)
    parser.add_argument("--tesseract-psm", type=int, default=3)
    parser.add_argument("--paddle-model-dir", type=Path)
    parser.add_argument("--apple-vision", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    if manifest.get("schema") != "mpdf-native-geometry-control/1":
        raise SystemExit("unsupported control manifest")
    # Homebrew Leptonica on macOS can fail to reopen images through the
    # ``/tmp`` compatibility symlink. Resolve once so every candidate sees the
    # same canonical ``/private/tmp`` file.
    root = args.manifest.parent.resolve()
    candidates = []
    if args.tesseract:
        candidates.append(
            run_candidate(
                f"tesseract-5.5.3-psm{args.tesseract_psm}",
                manifest["pages"],
                root,
                lambda image: tesseract_lines(
                    image, args.tesseract, args.tessdata, args.tesseract_psm
                ),
            )
        )
    if args.paddle_model_dir:
        detector = paddle_detector(args.paddle_model_dir)
        candidates.append(
            run_candidate(
                "paddle-pp-ocrv5-server-det",
                manifest["pages"],
                root,
                lambda image: paddle_lines(detector, image),
            )
        )
    if args.apple_vision:
        candidates.append(
            run_candidate(
                "apple-vision-text-rectangles",
                manifest["pages"],
                root,
                lambda image: apple_lines(args.apple_vision, image),
            )
        )
    if not candidates:
        raise SystemExit("select at least one candidate")
    selected = max(
        candidates,
        key=lambda item: (
            item["aggregate"]["f1"],
            item["aggregate"]["mean_iou"],
            item["aggregate"]["mean_reading_order_tau"],
            -item["aggregate"]["median_latency_seconds"],
        ),
    )
    report = {
        "schema": SCHEMA,
        "control_manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
        "selection_policy": {
            "primary": "highest aggregate F1",
            "tie_breakers": ["mean_iou", "reading_order_tau", "latency"],
            "status": "historical_material_not_validated",
        },
        "control_scope": {
            "human_closed_world_verified": all(
                page["coverage"]["human_closed_world_verified"]
                for page in manifest["pages"]
            ),
            "historical_material_validated": manifest["reference"][
                "historical_material_validated"
            ],
            "claim": "clean_native_geometry_control_only",
        },
        "selected_candidate": selected["candidate"],
        "selected_status": "historical_material_not_validated",
        "candidates": candidates,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({item["candidate"]: item["aggregate"] for item in candidates}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
