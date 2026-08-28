#!/usr/bin/env python3
"""Tests for the automatic-bookmark evaluation metrics.

These run in ordinary CI and need no external corpus: the formulas are
checked against synthetic gold data, and the absent-corpus paths are checked
to report `not_run`/`pending` rather than a score.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import auto_bookmark_eval as evaluation  # noqa: E402


GOLD = {
    "document_id": "book",
    "expected_behavior": "bookmarks_required",
    "annotated": True,
    "bookmarks": [
        {"title": "Introduction", "level": 0, "target_physical_page_index": 3},
        {"title": "Ἀρχὴ τῆς σοφίας", "level": 1, "target_physical_page_index": 5},
    ],
}

RUN = {
    "candidates": [
        {
            "candidate_id": "a",
            "effective_title": "Introduction",
            "effective_parent_id": None,
            "physical_page_index": 3,
            "status": "auto_confirmed",
        },
        {
            "candidate_id": "b",
            "effective_title": "Ἀρχὴ τῆς σοφίας",
            "effective_parent_id": "a",
            "physical_page_index": 5,
            "status": "auto_confirmed",
        },
    ]
}

REPORT = {"auto_confirmed": 2, "mode": "toc_aligned"}


class MetricTests(unittest.TestCase):
    def test_the_formula_self_test_passes(self) -> None:
        self.assertEqual(evaluation.self_test()["status"], "self_test_passed")

    def test_accents_are_not_folded_when_comparing_titles(self) -> None:
        run = json.loads(json.dumps(RUN))
        run["candidates"][1]["effective_title"] = "Αρχη της σοφιας"
        metrics = evaluation.aggregate([evaluation.evaluate_document(GOLD, run, REPORT)])
        self.assertEqual(metrics["hallucination_count"], 1)
        self.assertEqual(metrics["gold_coverage"]["numerator"], 1)
        self.assertEqual(metrics["gold_coverage"]["denominator"], 2)

    def test_only_written_statuses_count_as_output(self) -> None:
        run = json.loads(json.dumps(RUN))
        run["candidates"].append(
            {
                "candidate_id": "c",
                "effective_title": "A Skipped Entry",
                "effective_parent_id": None,
                "physical_page_index": 8,
                "status": "skipped",
            }
        )
        run["candidates"].append(
            {
                "candidate_id": "d",
                "effective_title": "A Rejected Entry",
                "effective_parent_id": None,
                "physical_page_index": 9,
                "status": "rejected",
            }
        )
        metrics = evaluation.aggregate([evaluation.evaluate_document(GOLD, run, REPORT)])
        self.assertEqual(metrics["hallucination_count"], 0)
        self.assertEqual(metrics["title_precision"]["denominator"], 2)

    def test_a_missing_corpus_is_reported_as_not_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = evaluation.evaluate_pack(Path(directory))
        self.assertEqual(report["status"], "not_run")
        self.assertNotIn("metrics", report)

    def test_an_unannotated_document_is_pending_not_scored(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pack = Path(directory)
            (pack / "gold").mkdir()
            unannotated = dict(GOLD, annotated=False)
            (pack / "gold/book.json").write_text(
                json.dumps(unannotated), encoding="utf-8"
            )
            report = evaluation.evaluate_pack(pack)
        self.assertEqual(report["status"], "pending")
        self.assertEqual(report["pending_documents"], ["book"])
        self.assertNotIn("metrics", report)

    def test_a_complete_pack_is_scored_end_to_end(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pack = Path(directory)
            (pack / "gold").mkdir()
            (pack / "runs/book").mkdir(parents=True)
            (pack / "gold/book.json").write_text(json.dumps(GOLD), encoding="utf-8")
            (pack / "runs/book/candidates.json").write_text(
                json.dumps(RUN), encoding="utf-8"
            )
            (pack / "runs/book/generation-report.json").write_text(
                json.dumps(REPORT), encoding="utf-8"
            )
            report = evaluation.evaluate_pack(pack)
        self.assertEqual(report["status"], "complete")
        self.assertEqual(report["metrics"]["gold_coverage"]["value"], 1.0)
        self.assertEqual(report["metrics"]["hierarchy_edge_f1"], 1.0)
        self.assertTrue(report["metrics"]["hallucination_gate_passed"])


if __name__ == "__main__":
    unittest.main()
