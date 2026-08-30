"""Tests for the frozen open-world OCR coverage contract."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import complete_ocr_coverage as coverage  # noqa: E402
import run_cloud_complete_ocr_bakeoff as common  # noqa: E402


def _page(text: str = "λόγοςʼ") -> cgpg.Page:
    return cgpg.Page(
        name="page",
        image_path=Path("page.jpg"),
        xml_path=Path("page.xml"),
        width=100,
        height=100,
        regions=(
            cgpg.Region(
                "r",
                "text",
                (0, 0, 100, 100),
                (cgpg.Line(text, (10, 10, 90, 20)),),
            ),
        ),
    )


def test_inline_unannotated_anchor_and_apostrophe_variants_are_equivalent():
    result = coverage.score_open_world_target(
        _page(),
        (
            common.ProviderLine(
                "λόγος (12)´", common.Box(10, 10, 90, 20), 0
            ),
        ),
    )
    assert result["target_polytonic"]["cer"] == 0.0
    assert result["target_base_letter"]["cer"] == 0.0
    assert result["target_recall_at_0_50"] == 1.0


def test_unmatched_visible_line_is_unassessed_not_false_positive():
    result = coverage.score_open_world_target(
        _page("λόγος"),
        (
            common.ProviderLine("λόγος", common.Box(10, 10, 90, 20), 0),
            common.ProviderLine(
                "visible Latin", common.Box(10, 30, 90, 40), 1
            ),
        ),
    )
    assert result["target_polytonic"]["cer"] == 0.0
    assert result["unassessed_provider_lines"] == 1
    assert result["provider_completeness_assessed"] is False
    assert result["production_provider_selection_permitted"] is False


def test_missing_annotated_target_is_a_recall_and_text_error():
    result = coverage.score_open_world_target(_page("λόγος"), ())
    assert result["target_recall_at_0_50"] == 0.0
    assert result["target_polytonic"]["cer"] == 1.0
    assert result["unassessed_provider_lines"] == 0


def test_contract_configuration_forbids_complete_ocr_selection():
    configuration = coverage.contract_configuration()
    assert configuration["coverage_mode"] == "open_world_annotated_target"
    assert configuration["production_provider_selection_permitted"] is False
    assert configuration["provider_completeness_claim_permitted"] is False


def test_frozen_contract_binds_configuration():
    assert coverage.verify_contract() == coverage.CONTRACT_SHA256
