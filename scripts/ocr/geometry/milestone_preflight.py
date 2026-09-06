#!/usr/bin/env python3
"""Read-only reference readiness, never an OCR quality evaluation.

Holdout records are hashed as bytes, not parsed for diagnosis. Development
records are excluded by page identity AND actual image digest. D's semantic
review statuses are deliberately not prerequisites for OCR/geometry dev
references. This does not change or replace the frozen Gold validator.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import unicodedata
from collections import Counter
from pathlib import Path


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def asset(root: Path, page_id: str, suffix: str) -> Path:
    if not isinstance(page_id, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", page_id):
        raise ValueError("invalid_page_identity")
    path = root / (page_id + suffix)
    if path.is_symlink() or not path.is_file():
        raise ValueError("missing_or_symlinked_asset")
    return path


def verify_holdout(manifest_path: Path, pages: Path, images: Path) -> dict:
    manifest = json.loads(manifest_path.read_bytes())
    if (
        manifest.get("schema") != "mpdf-closed-world-ocr-gold-manifest"
        or manifest.get("schema_version") != "1.0"
        or manifest.get("coverage") != "closed_world_complete"
    ):
        raise ValueError("invalid_frozen_manifest")
    expected = manifest["expected_page_ids"]
    entries = manifest["pages"]
    if not expected or len(expected) != len(set(expected)):
        raise ValueError("invalid_frozen_page_set")
    if [entry["page_id"] for entry in entries] != expected:
        raise ValueError("frozen_page_set_mismatch")
    for entry in entries:
        if (
            digest(asset(pages, entry["page_id"], ".json")) != entry["record_sha256"]
            or digest(asset(images, entry["page_id"], ".png")) != entry["image_sha256"]
        ):
            raise ValueError("frozen_integrity_failure")
    return manifest


def reference_gaps(record: dict) -> list[str]:
    """Reference evidence only; no provider scores or semantic acceptance."""
    gaps = set()
    coverage = record.get("coverage", {})
    if coverage.get("closed_world") is not True:
        gaps.add("not_closed_world")
    if coverage.get("all_visible_lines_exhaustively_reviewed") is not True:
        gaps.add("visible_line_coverage_unreviewed")
    if not isinstance(coverage.get("reviewer"), str) or not coverage["reviewer"].strip():
        gaps.add("no_reference_reviewer")
    # An unresolved note may concern OCR evidence; do not guess it belongs to D.
    if not isinstance(coverage.get("unresolved_notes"), str) or coverage["unresolved_notes"].strip():
        gaps.add("unresolved_reference_notes")
    image = record.get("image", {})
    width, height = image.get("width"), image.get("height")
    dimensions_valid = all(type(n) is int and n > 0 for n in (width, height))
    if not dimensions_valid:
        gaps.add("invalid_image_dimensions")
    lines = record.get("lines")
    if not isinstance(lines, list) or not lines:
        # Blank-region references need a separately explicit annotation contract.
        return sorted(gaps | {"no_line_reference"})
    ids = set()
    for order, line in enumerate(lines):
        line_id = line.get("line_id")
        if not isinstance(line_id, str) or not line_id or line_id in ids:
            gaps.add("invalid_line_identity")
        else:
            ids.add(line_id)
        if type(line.get("reading_order")) is not int or line["reading_order"] != order:
            gaps.add("invalid_reference_order")
        for layer in ("geometry", "transcription"):
            if line.get(layer + "_status") != "human_verified":
                gaps.add(layer + "_unreviewed")
        text = line.get("text")
        if not isinstance(text, str) or not text.strip() or unicodedata.normalize("NFC", text) != text:
            gaps.add("invalid_reference_text")
        box = line.get("bbox")
        if (
            not dimensions_valid or not isinstance(box, list) or len(box) != 4
            or not all(type(n) in (int, float) and math.isfinite(n) for n in box)
        ):
            gaps.add("invalid_reference_box")
        elif not (0 <= box[0] < box[2] <= width and 0 <= box[1] < box[3] <= height):
            gaps.add("invalid_reference_box")
    return sorted(gaps)


def source_bindings(selections: list[Path]) -> dict:
    bindings = {}
    for path in selections:
        selection = json.loads(path.read_bytes())
        sources = {source["source_id"]: source["sha256"] for source in selection["sources"]}
        for page in selection["pages"]:
            binding = (sources[page["source_id"]], page["pdf_page"], page["image_sha256"])
            if not re.fullmatch(r"[0-9a-f]{64}", binding[0]) or type(binding[1]) is not int or binding[1] < 1:
                raise ValueError("invalid_source_page_binding")
            if page["page_id"] in bindings and bindings[page["page_id"]] != binding:
                raise ValueError("conflicting_source_page_binding")
            bindings[page["page_id"]] = binding
    return bindings


def inspect(manifest_path: Path, holdout_pages: Path, holdout_images: Path,
            dev_pages: Path, dev_images: Path, selections: list[Path]) -> dict:
    manifest = verify_holdout(manifest_path, holdout_pages, holdout_images)
    protected_ids = set(manifest["expected_page_ids"])
    protected_images = {entry["image_sha256"] for entry in manifest["pages"]}
    bindings = source_bindings(selections)
    if not protected_ids.issubset(bindings):
        raise ValueError("missing_holdout_source_binding")
    for entry in manifest["pages"]:
        if bindings[entry["page_id"]][2] != entry["image_sha256"]:
            raise ValueError("holdout_source_image_binding_mismatch")
    protected_sources = {bindings[page_id][:2] for page_id in protected_ids}
    records = []
    excluded = Counter()
    for path in sorted(dev_pages.glob("*.json")):
        if path.stem in protected_ids:
            excluded["frozen_page_id"] += 1
            continue
        if path.is_symlink():
            raise ValueError("symlinked_dev_record")
        record = json.loads(path.read_bytes())
        page_id = record.get("page_id")
        if page_id in protected_ids:
            excluded["frozen_page_id"] += 1
            continue
        if page_id != path.stem:
            raise ValueError("dev_filename_identity_mismatch")
        binding = bindings.get(page_id)
        if binding and binding[:2] in protected_sources:
            excluded["frozen_source_page"] += 1
            continue
        image_sha = digest(asset(dev_images, page_id, ".png"))
        if image_sha in protected_images:
            excluded["frozen_image_digest"] += 1
            continue
        if image_sha != record.get("image", {}).get("sha256"):
            raise ValueError("dev_image_digest_mismatch")
        if binding and binding[2] != image_sha:
            raise ValueError("dev_source_image_binding_mismatch")
        gaps = reference_gaps(record)
        if not binding:
            gaps.append("no_source_page_binding")
        records.append({"page_id": page_id, "record_sha256": digest(path),
                        "image_sha256": image_sha,
                        "source_sha256": binding[0] if binding else None,
                        "pdf_page": binding[1] if binding else None,
                        "reference_gaps": gaps})
    eligible = sum(not record["reference_gaps"] for record in records)
    return {
        "schema": "mpdf-ocr-geometry-dev-preflight/1",
        "frozen_manifest_sha256": digest(manifest_path),
        "selection_sha256": [digest(path) for path in selections],
        "frozen_integrity": "PASS", "frozen_pages": len(protected_ids),
        "holdout_content_used_for_diagnosis": False,
        "excluded": dict(excluded), "nonholdout_pages": len(records),
        "reference_ready_pages": eligible,
        "reference_readiness": "REFERENCES_AVAILABLE" if eligible else "DEV_REFERENCE_EVIDENCE_MISSING",
        "scope": "Reference readiness only; not dev exit, stability, domain coverage or milestone PASS.",
        "d_semantic_review_required": False,
        "inference_calls": 0, "frozen_evaluation_executed": False,
        "records": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("manifest", "holdout-pages", "holdout-images", "dev-pages", "dev-images", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--selection", type=Path, action="append", required=True)
    args = parser.parse_args()
    report = inspect(args.manifest, args.holdout_pages, args.holdout_images,
                     args.dev_pages, args.dev_images, args.selection)
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2, ensure_ascii=False)
        stream.write("\n")
    print(json.dumps({key: report[key] for key in
                      ("frozen_integrity", "nonholdout_pages", "reference_ready_pages", "reference_readiness")}))
    return 0 if report["reference_ready_pages"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
