#!/usr/bin/env python3
"""Coverage-aware scoring for incomplete Complete OCR ground truth.

CGPG PAGE XML annotates the target Greek transcription but does not exhaustively
annotate every visible printed item.  This module therefore treats the current
holdout as open-world annotated-target gold: missing target lines are errors,
while unmatched provider lines are explicitly unassessed rather than false
positives.  Such results compare target recognition and geometry fairly, but
cannot establish closed-world Complete OCR eligibility.
"""

from __future__ import annotations

import re
import unicodedata
from pathlib import Path
from typing import Sequence

import run_cloud_complete_ocr_bakeoff as common


CONTRACT_SCHEMA = "mpdf-complete-ocr-coverage-contract"
CONTRACT_SCHEMA_VERSION = "2.0"
COVERAGE_MODE = "open_world_annotated_target"
FOOTNOTE_ANCHOR_PATTERN = r"\s*\(\s*[0-9]+\s*\)"
FOOTNOTE_ANCHOR_RE = re.compile(FOOTNOTE_ANCHOR_PATTERN)
APOSTROPHE_EQUIVALENTS = "'´ʹʼ΄᾽᾿’"
APOSTROPHE_CANONICAL = "ʼ"
APOSTROPHE_TRANSLATION = str.maketrans(
    {character: APOSTROPHE_CANONICAL for character in APOSTROPHE_EQUIVALENTS}
)
DEFAULT_CONTRACT_PATH = (
    common.REPO_ROOT
    / "docs/evidence/cloud-complete-ocr-coverage-contract-v2-2026-08-30.json"
)
CONTRACT_SHA256 = (
    "b3115b5d245634d9c51d5f3609c2925d69848bf81a07e42107c6c802d2f64324"
)


def contract_configuration() -> dict[str, object]:
    return {
        "schema": CONTRACT_SCHEMA,
        "schema_version": CONTRACT_SCHEMA_VERSION,
        "coverage_mode": COVERAGE_MODE,
        "formal_input_manifest_sha256": common.FORMAL_INPUT_MANIFEST_SHA256,
        "page_ids": [item["page_id"] for item in common.FIXED_INPUT_RECORDS],
        "target": "PAGE_XML_annotated_Greek_lines",
        "unmatched_reference_line": "missing_target_error",
        "unmatched_provider_line": "unassessed_not_false_positive",
        "inline_unannotated_footnote_anchor_pattern": (
            FOOTNOTE_ANCHOR_PATTERN
        ),
        "apostrophe_equivalence": {
            "members": list(APOSTROPHE_EQUIVALENTS),
            "canonical": APOSTROPHE_CANONICAL,
        },
        "text_identity": "IoU_at_least_0.50_maximum_weight_matching",
        "geometry_metrics": [
            "target_recall_at_0.50",
            "target_recall_at_0.75",
            "matched_mean_iou_at_0.50",
            "unassessed_provider_lines",
        ],
        "text_metrics": [
            "target_polytonic_cer",
            "target_base_letter_cer",
        ],
        "reading_order_scope": "IoU_at_least_0.50_matched_target_lines",
        "provider_completeness_claim_permitted": False,
        "production_provider_selection_permitted": False,
        "closed_world_requirement": (
            "Every visible printed line must have verified text, geometry, "
            "reading order, content class, and language, with zero unreviewed "
            "visible-text regions."
        ),
    }


def verify_contract(path: Path = DEFAULT_CONTRACT_PATH) -> str:
    raw = path.read_bytes()
    if common.sha256_bytes(raw) != CONTRACT_SHA256:
        raise common.BakeoffError("coverage_contract_digest_mismatch")
    value = common.strict_json_loads(raw, maximum_bytes=1024 * 1024)
    configuration_sha256 = common.canonical_digest(contract_configuration())
    try:
        valid = (
            isinstance(value, dict)
            and value["schema"] == CONTRACT_SCHEMA
            and value["schema_version"] == CONTRACT_SCHEMA_VERSION
            and value["status"] == "frozen"
            and value["configuration_sha256"] == configuration_sha256
            and value["coverage"]["mode"] == COVERAGE_MODE
            and value["eligibility"]["production_provider_selection"]
            is False
        )
    except (KeyError, TypeError):
        valid = False
    if not valid:
        raise common.BakeoffError("coverage_contract_invalid")
    return CONTRACT_SHA256


def normalize_target_text(text: str) -> str:
    without_anchors = FOOTNOTE_ANCHOR_RE.sub("", text)
    canonical_apostrophes = without_anchors.translate(APOSTROPHE_TRANSLATION)
    return " ".join(unicodedata.normalize("NFC", canonical_apostrophes).split())


def normalize_base_target_text(text: str) -> str:
    return normalize_target_text(common.prior_bakeoff.base_letter_text(text))


def _counts(reference: str, hypothesis: str) -> common.prior_bakeoff.CerCounts:
    if (
        (len(reference) + 1) * (len(hypothesis) + 1)
        > common.MAX_CER_CELL_UPDATES
    ):
        raise common.BakeoffError("cer_work_limit_exceeded")
    return common.prior_bakeoff.CerCounts(
        reference_chars=len(reference),
        char_edits=common.gold_metrics.levenshtein(reference, hypothesis),
        samples=1,
        nfc_violations=int(not unicodedata.is_normalized("NFC", hypothesis)),
    )


def _target_text_counts(
    references: Sequence[common.ProviderLine],
    providers: Sequence[common.ProviderLine],
    matches: Sequence[common.Match],
) -> tuple[common.prior_bakeoff.CerCounts, common.prior_bakeoff.CerCounts]:
    by_reference = {match.left_index: match.right_index for match in matches}
    polytonic = common.prior_bakeoff.CerCounts()
    base = common.prior_bakeoff.CerCounts()
    for reference_index, reference in enumerate(references):
        provider_index = by_reference.get(reference_index)
        hypothesis = "" if provider_index is None else providers[provider_index].text
        polytonic = polytonic.merge(
            _counts(
                normalize_target_text(reference.text),
                normalize_target_text(hypothesis),
            )
        )
        base = base.merge(
            _counts(
                normalize_base_target_text(reference.text),
                normalize_base_target_text(hypothesis),
            )
        )
    return polytonic, base


def score_open_world_target(
    page: common.cgpg.Page,
    provider_lines: Sequence[common.ProviderLine],
) -> dict[str, object]:
    references = common.reference_lines(page)
    providers = common.lines_in_scorable_regions(page, provider_lines)
    reference_boxes = [line.bbox for line in references]
    provider_boxes = [line.bbox for line in providers]
    matches_050 = common.maximum_weight_matching(
        reference_boxes, provider_boxes, minimum_iou=0.5
    )
    matches_075 = common.maximum_weight_matching(
        reference_boxes, provider_boxes, minimum_iou=0.75
    )
    polytonic, base = _target_text_counts(
        references, providers, matches_050
    )
    order = common.reading_order_metrics(
        matches_050, len(references), providers
    )
    return {
        "coverage_mode": COVERAGE_MODE,
        "reference_target_lines": len(references),
        "provider_lines_in_target_regions": len(providers),
        "matched_target_lines_at_0_50": len(matches_050),
        "matched_target_lines_at_0_75": len(matches_075),
        "target_recall_at_0_50": common._display_float(
            common._ratio(len(matches_050), len(references))
        ),
        "target_recall_at_0_75": common._display_float(
            common._ratio(len(matches_075), len(references))
        ),
        "matched_mean_iou_at_0_50": common._display_float(
            sum(match.iou for match in matches_050) / len(matches_050)
            if matches_050
            else 0.0
        ),
        "unassessed_provider_lines": len(providers) - len(matches_050),
        "target_polytonic": polytonic.as_dict(),
        "target_base_letter": base.as_dict(),
        "reading_order": {
            key: common._display_float(value)
            if isinstance(value, float)
            else value
            for key, value in order.items()
        },
        "provider_completeness_assessed": False,
        "production_provider_selection_permitted": False,
    }
