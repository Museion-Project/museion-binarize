"""Unit tests for the fixed CGPG Greek-model bake-off."""

from __future__ import annotations

import sys
import unicodedata
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import greek_text_enhancer as enhancer  # noqa: E402
import metrics  # noqa: E402
import run_cgpg_model_bakeoff as bakeoff  # noqa: E402


def test_enhancer_repairs_a_mapped_lookalike_in_a_greek_token():
    result = enhancer.enhance_token("ἀνθpωπος", 0.95)
    assert result.text == "ἀνθρωπος"
    assert result.rewrites == 1
    assert len(result.text) == len("ἀνθpωπος")


def test_enhancer_normalizes_nfc_without_inventing_marks():
    decomposed = unicodedata.normalize("NFD", "ἀρετή")
    result = enhancer.enhance_token(decomposed, 0.20)
    assert result.text == "ἀρετή"
    assert result.normalized
    assert result.rewrites == 0


def test_enhancer_matches_rust_by_rewriting_eligible_token_edges():
    assert enhancer.enhance_token("pἀρετή", 0.95).text == "ρἀρετή"
    assert enhancer.enhance_token("ἀρετήx", 0.95).text == "ἀρετήχ"


def test_enhancer_does_not_rewrite_latin_low_confidence_or_unmapped_letters():
    assert enhancer.enhance_token("parenthetical", 0.99).text == "parenthetical"
    assert enhancer.enhance_token("ἀνθpωπος", 0.9499).text == "ἀνθpωπος"
    assert enhancer.enhance_token("ἀρεuή", 0.99).text == "ἀρεuή"


def test_greek_token_predicate_and_mapping_exactly_match_the_rust_candidate():
    assert enhancer.is_conservative_greek_token("ἀνθpωπος")
    assert not enhancer.is_conservative_greek_token("parenthetical")
    assert not enhancer.is_conservative_greek_token("ἀρεuή")
    assert enhancer.LATIN_TO_GREEK == {
        "A": "Α", "B": "Β", "E": "Ε", "H": "Η", "I": "Ι",
        "K": "Κ", "M": "Μ", "N": "Ν", "O": "Ο", "P": "Ρ",
        "T": "Τ", "X": "Χ", "Y": "Υ", "Z": "Ζ", "o": "ο",
        "p": "ρ", "x": "χ",
    }


def test_tsv_parser_preserves_token_ids_text_and_boxes():
    header = "\t".join(bakeoff.TSV_FIELDS)
    tsv = "\n".join(
        [
            header,
            "2\t1\t1\t0\t0\t0\t10\t20\t100\t30\t-1\t",
            "4\t1\t1\t1\t1\t0\t10\t20\t100\t30\t-1\t",
            "5\t1\t1\t1\t1\t1\t10\t20\t50\t30\t95.0\tἀρεtή",
        ]
    )
    response = bakeoff.parse_tsv(tsv)
    word = response["blocks"][0]["lines"][0]["words"][0]
    assert word["text"] == "ἀρεtή"
    assert word["lineage_id"] == "p1:b1:p1:l1:w1"
    assert word["confidence"] == 0.95
    assert word["bbox"] == {"x": 10.0, "y": 20.0, "width": 50.0, "height": 30.0}


def test_tsv_parser_treats_a_bare_quote_as_text_not_csv_quoting():
    header = "\t".join(bakeoff.TSV_FIELDS)
    tsv = "\n".join(
        [
            header,
            '5\t1\t1\t1\t1\t1\t10\t20\t5\t10\t80.0\t"',
            "5\t1\t1\t1\t2\t1\t10\t40\t30\t10\t95.0\tλόγος",
        ]
    )
    response = bakeoff.parse_tsv(tsv)
    lines = response["blocks"][0]["lines"]
    assert [word["text"] for word in lines[0]["words"]] == ['"']
    assert [word["text"] for word in lines[1]["words"]] == ["λόγος"]


def test_response_enhancement_preserves_every_token_and_box():
    response = {
        "blocks": [
            {
                "reading_order": 0,
                "bbox": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 30.0},
                "lines": [
                    {
                        "reading_order": 0,
                        "bbox": {
                            "x": 10.0,
                            "y": 5.0,
                            "width": 50.0,
                            "height": 20.0,
                        },
                        "words": [
                            {
                                "lineage_id": "p1:b1:p1:l1:w1",
                                "text": "ἀνθpωπος",
                                "bbox": {
                                    "x": 10.0,
                                    "y": 5.0,
                                    "width": 50.0,
                                    "height": 20.0,
                                },
                                "confidence": 0.95,
                            }
                        ],
                    }
                ],
            }
        ]
    }
    changed, audit = bakeoff.enhance_response(response)
    assert changed["blocks"][0]["lines"][0]["words"][0]["text"] == "ἀνθρωπος"
    assert response["blocks"][0]["lines"][0]["words"][0]["text"] == "ἀνθpωπος"
    assert audit["lookalike_rewrites"] == 1
    assert audit["token_lineage_rate"] == 1.0
    assert audit["box_lineage_rate"] == 1.0


def _page(name: str, text: str = "ἐκεῖνο") -> cgpg.Page:
    lines = (cgpg.Line(text, (0, 0, 10, 10)),) if text else ()
    return cgpg.Page(
        name=name,
        image_path=Path(f"{name}.jpg"),
        xml_path=Path(f"{name}.xml"),
        width=100,
        height=100,
        regions=(cgpg.Region("r1", "text", (0, 0, 10, 10), lines),),
    )


def test_page_selection_is_fixed_sorted_and_scorable_only():
    pages = [_page(f"page-{index:03}") for index in reversed(range(200))]
    pages.append(_page("page-empty", text=""))
    selected = bakeoff.select_fixed_pages(pages)
    _development, holdout = cgpg.split_corpus(pages)
    expected = sorted(
        (page for page in holdout if page.transcribed_lines),
        key=lambda page: page.name,
    )[: bakeoff.SCORABLE_PAGE_COUNT]
    assert [page.name for page in selected] == [page.name for page in expected]
    assert all(page in holdout for page in selected)


def test_evidence_command_template_contains_no_host_absolute_paths():
    argv = bakeoff.reproducible_harness_argv()
    assert argv[0:2] == [
        "python3",
        "scripts/ocr/gold/run_cgpg_model_bakeoff.py",
    ]
    assert "<CGPG_DATA_DIR>" in argv
    assert "<TESSERACT_BIN>" in argv
    assert "<TESSDATA_DIR>" in argv
    assert all(not value.startswith("/Users/") for value in argv)


def test_candidate_set_and_engine_parameters_are_fixed():
    assert bakeoff.CANDIDATE_ORDER == ("ell", "ell+enhancer", "grc", "ell+grc")
    argv = bakeoff.tesseract_argv(
        Path("/runtime/bin/tesseract"),
        Path("/corpus/page.jpg"),
        Path("/runtime/tessdata"),
        "ell+grc",
    )
    assert argv == [
        "/runtime/bin/tesseract",
        "/corpus/page.jpg",
        "stdout",
        "--tessdata-dir",
        "/runtime/tessdata",
        "-l",
        "ell+grc",
        "--psm",
        "6",
        "--oem",
        "1",
        "-c",
        "preserve_interword_spaces=1",
        "-c",
        "tessedit_create_tsv=1",
    ]


def test_fast_cer_counts_match_the_shared_gold_metric_definition():
    reference = "ἄνθρωπος μέτρον\nἀρετή"
    hypothesis = "ανθρωπος μέτpον ἀρετή"
    expected = metrics.score(reference, hypothesis)
    actual = bakeoff.cer_counts(reference, hypothesis)
    assert actual.reference_chars == expected.reference_chars
    assert actual.char_edits == expected.char_edits
    assert actual.cer == expected.cer


def test_correct_base_rewrite_metric_distinguishes_repair_from_damage():
    repair = bakeoff.correct_base_rewrite_stats("ἀρετή", "ἀρεtή", "ἀρετή")
    assert repair["base_changing_rewrites"] == 1
    assert repair["correct_base_rewrites"] == 0

    damage = bakeoff.correct_base_rewrite_stats("ἀρετή", "ἀρετή", "ἀρεpή")
    assert damage["base_changing_rewrites"] == 1
    assert damage["correct_base_rewrites"] == 1


def test_gate_uses_the_pre_registered_thresholds_without_rounding_them():
    baseline = {
        "polytonic": {"cer": 0.20, "char_edits": 200, "reference_chars": 1000},
        "base_letter": {"cer": 0.10, "char_edits": 100, "reference_chars": 1000},
        "pages_succeeded": 12,
    }
    enhanced = {
        "polytonic": {"cer": 0.13, "char_edits": 130, "reference_chars": 1000},
        "base_letter": {"cer": 0.104, "char_edits": 104, "reference_chars": 1000},
        "pages_succeeded": 12,
    }
    rewrite = {
        "correct_base_rewrite_percent": 0.4,
        "correct_base_rewrites": 4,
        "correct_base_letters_before": 1000,
    }
    lineage = {"token_lineage_percent": 100.0, "box_lineage_percent": 100.0}
    assert bakeoff.evaluate_gate(baseline, enhanced, rewrite, lineage)["outcome"] == "pass"

    # The display CER is deliberately inconsistent: the integer counts must
    # remain authoritative for the gate.
    enhanced["polytonic"].update(cer=0.13, char_edits=141)
    assert bakeoff.evaluate_gate(baseline, enhanced, rewrite, lineage)["outcome"] == "fail"


def test_candidate_comparisons_use_unrounded_counts_and_ell_as_baseline():
    def candidate(polytonic_edits: int, base_edits: int) -> dict:
        return {
            "polytonic": {
                "cer": 999.0,
                "char_edits": polytonic_edits,
                "reference_chars": 1000,
            },
            "base_letter": {
                "cer": 999.0,
                "char_edits": base_edits,
                "reference_chars": 1000,
            },
        }

    comparisons = bakeoff.comparisons_to_ell(
        {
            "ell": candidate(200, 100),
            "ell+enhancer": candidate(140, 103),
            "grc": candidate(250, 120),
            "ell+grc": candidate(180, 90),
        }
    )
    assert comparisons["ell+enhancer"] == {
        "polytonic_cer_delta_percentage_points": -6.0,
        "polytonic_cer_relative_improvement_percent": 30.0,
        "base_letter_cer_delta_percentage_points": 0.3,
    }
    assert comparisons["grc"]["polytonic_cer_relative_improvement_percent"] == -25.0
    assert comparisons["ell+grc"]["base_letter_cer_delta_percentage_points"] == -1.0
