#!/usr/bin/env python3
"""Evaluation metrics for automatic table-of-contents compilation.

The metrics below are computed against a human-gold corpus with an explicit
denominator for every number, so a partially annotated corpus can never be
reported as a high score. When the corpus is absent or incomplete the entry
point reports ``not_run``/``pending`` rather than a value: an unrun
evaluation is never presented as a passing one.

Layout of an evaluation pack::

    <pack>/gold/<document_id>.json        one gold record per document
    <pack>/runs/<document_id>/candidates.json
    <pack>/runs/<document_id>/generation-report.json

A gold record is::

    {"document_id": "...",
     "expected_behavior": "bookmarks_required" | "needs_review" | "safe_refusal",
     "annotated": true,
     "bookmarks": [{"title": "...", "level": 0, "target_physical_page_index": 3}]}

Run ``--self-test`` to check the metric formulas against synthetic data;
that mode needs no corpus and is what ordinary CI exercises.
"""

from __future__ import annotations

import argparse
import json
import sys
import unicodedata
from pathlib import Path

WRITTEN_STATUSES = {"auto_confirmed", "confirmed"}
BEHAVIORS = {"bookmarks_required", "needs_review", "safe_refusal"}


class EvalError(Exception):
    """A malformed pack. Never raised for a merely absent corpus."""


def normalize_title(title: str) -> str:
    """The comparison key: NFKC, case-folded, whitespace-collapsed.

    Accents are deliberately *not* folded here — a polytonic Greek title
    that lost its breathing marks is a wrong title, not a match.
    """
    folded = unicodedata.normalize("NFKC", title).casefold()
    return " ".join(folded.split())


def written_bookmarks(candidates: dict) -> list[dict]:
    """The entries that actually reach a PDF outline: the deterministic
    ``auto_confirmed`` ones and the human ``confirmed`` ones."""
    return [
        candidate
        for candidate in candidates.get("candidates", [])
        if candidate.get("status") in WRITTEN_STATUSES
    ]


def depth_of(candidate: dict, by_id: dict[str, dict]) -> int:
    depth = 0
    parent = candidate.get("effective_parent_id")
    seen = set()
    while parent and parent in by_id and parent not in seen:
        seen.add(parent)
        depth += 1
        parent = by_id[parent].get("effective_parent_id")
    return depth


def edges(entries: list[tuple[str, int, int]]) -> set[tuple[str, str]]:
    """Parent/child edges implied by a depth-ordered list of
    ``(title, depth, page)`` entries, as ``(parent_title, child_title)``."""
    stack: list[tuple[str, int]] = []
    result: set[tuple[str, str]] = set()
    for title, depth, _ in entries:
        while stack and stack[-1][1] >= depth:
            stack.pop()
        if stack:
            result.add((stack[-1][0], title))
        stack.append((title, depth))
    return result


def evaluate_document(gold: dict, candidates: dict | None, report: dict | None) -> dict:
    """Per-document counts. Every count names what it is counted out of."""
    behavior = gold.get("expected_behavior")
    if behavior not in BEHAVIORS:
        raise EvalError(f"unknown expected behavior: {behavior!r}")
    gold_entries = [
        (
            normalize_title(entry["title"]),
            int(entry.get("level", 0)),
            int(entry["target_physical_page_index"]),
        )
        for entry in gold.get("bookmarks", [])
    ]
    written: list[tuple[str, int, int]] = []
    if candidates is not None:
        by_id = {
            candidate["candidate_id"]: candidate
            for candidate in candidates.get("candidates", [])
        }
        for candidate in written_bookmarks(candidates):
            written.append(
                (
                    normalize_title(candidate["effective_title"]),
                    depth_of(candidate, by_id),
                    int(candidate["physical_page_index"]),
                )
            )
    gold_titles = {title for title, _, _ in gold_entries}
    gold_by_title = {title: (depth, page) for title, depth, page in gold_entries}

    title_matches = [entry for entry in written if entry[0] in gold_titles]
    hallucinations = [entry for entry in written if entry[0] not in gold_titles]
    page_correct = [
        entry for entry in title_matches if gold_by_title[entry[0]][1] == entry[2]
    ]
    covered = {entry[0] for entry in page_correct}

    gold_edges = edges(gold_entries)
    written_edges = edges(sorted(written, key=lambda entry: (entry[2], entry[1])))
    shared_edges = gold_edges & written_edges

    if behavior == "safe_refusal":
        refusal_correct = len(written) == 0
    else:
        refusal_correct = None
    zero_edit = (
        behavior == "bookmarks_required"
        and not hallucinations
        and len(covered) == len(gold_titles)
        and gold_edges == written_edges
    )
    return {
        "document_id": gold.get("document_id"),
        "expected_behavior": behavior,
        "gold_bookmarks": len(gold_entries),
        "written_bookmarks": len(written),
        "title_matches": len(title_matches),
        "page_correct": len(page_correct),
        "hallucinations": len(hallucinations),
        "gold_edges": len(gold_edges),
        "written_edges": len(written_edges),
        "shared_edges": len(shared_edges),
        "covered_gold": len(covered),
        "auto_confirmed": 0
        if report is None
        else int(report.get("auto_confirmed", 0)),
        "safe_refusal_correct": refusal_correct,
        "zero_edit_success": zero_edit,
        "mode": None if report is None else report.get("mode"),
    }


def ratio(numerator: int, denominator: int) -> dict:
    """A metric always carries its denominator; an empty denominator is
    reported as ``null``, never as 1.0."""
    return {
        "numerator": numerator,
        "denominator": denominator,
        "value": (numerator / denominator) if denominator else None,
    }


def aggregate(documents: list[dict]) -> dict:
    total = lambda key: sum(int(document[key]) for document in documents)  # noqa: E731
    refusal_documents = [
        document
        for document in documents
        if document["expected_behavior"] == "safe_refusal"
    ]
    required_documents = [
        document
        for document in documents
        if document["expected_behavior"] == "bookmarks_required"
    ]
    precision_denominator = total("written_bookmarks")
    edge_precision = ratio(total("shared_edges"), total("written_edges"))
    edge_recall = ratio(total("shared_edges"), total("gold_edges"))
    f1 = None
    if edge_precision["value"] and edge_recall["value"]:
        f1 = (
            2
            * edge_precision["value"]
            * edge_recall["value"]
            / (edge_precision["value"] + edge_recall["value"])
        )
    return {
        "documents": len(documents),
        "target_page_accuracy": ratio(total("page_correct"), total("title_matches")),
        "title_precision": ratio(total("title_matches"), precision_denominator),
        "gold_coverage": ratio(total("covered_gold"), total("gold_bookmarks")),
        "automatically_confirmed_coverage": ratio(
            total("auto_confirmed"), total("gold_bookmarks")
        ),
        "hierarchy_edge_precision": edge_precision,
        "hierarchy_edge_recall": edge_recall,
        "hierarchy_edge_f1": f1,
        "safe_refusal_document_accuracy": ratio(
            sum(1 for document in refusal_documents if document["safe_refusal_correct"]),
            len(refusal_documents),
        ),
        "zero_edit_document_success": ratio(
            sum(1 for document in required_documents if document["zero_edit_success"]),
            len(required_documents),
        ),
        # The release gate: a written bookmark whose title is not in the gold
        # set is a hallucination, and the gate is exactly zero.
        "hallucination_count": total("hallucinations"),
        "hallucination_gate_passed": total("hallucinations") == 0,
    }


def read_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvalError(f"cannot read {path}: {error}") from error


def evaluate_pack(pack: Path) -> dict:
    gold_directory = pack / "gold"
    if not gold_directory.is_dir():
        return {
            "schema": "mpdf-auto-bookmark-eval",
            "schema_version": "0.1",
            "status": "not_run",
            "reason": f"no gold corpus at {gold_directory}",
        }
    documents = []
    pending = []
    for gold_path in sorted(gold_directory.glob("*.json")):
        gold = read_json(gold_path)
        if not gold.get("annotated", False):
            pending.append(gold.get("document_id", gold_path.stem))
            continue
        run = pack / "runs" / str(gold.get("document_id", gold_path.stem))
        candidates_path = run / "candidates.json"
        report_path = run / "generation-report.json"
        if not candidates_path.is_file() or not report_path.is_file():
            pending.append(gold.get("document_id", gold_path.stem))
            continue
        documents.append(
            evaluate_document(
                gold, read_json(candidates_path), read_json(report_path)
            )
        )
    if not documents:
        return {
            "schema": "mpdf-auto-bookmark-eval",
            "schema_version": "0.1",
            "status": "pending",
            "reason": "no annotated document has both gold data and a generated run",
            "pending_documents": pending,
        }
    return {
        "schema": "mpdf-auto-bookmark-eval",
        "schema_version": "0.1",
        "status": "complete" if not pending else "partial",
        "pending_documents": pending,
        "metrics": aggregate(documents),
        "documents": documents,
    }


def self_test() -> dict:
    """Checks the formulas against synthetic gold and synthetic runs, so
    ordinary CI verifies the metric definitions without any corpus."""
    gold = {
        "document_id": "synthetic",
        "expected_behavior": "bookmarks_required",
        "annotated": True,
        "bookmarks": [
            {"title": "Introduction", "level": 0, "target_physical_page_index": 3},
            {"title": "Ἀρχή", "level": 1, "target_physical_page_index": 5},
        ],
    }
    candidates = {
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
                "effective_title": "Ἀρχή",
                "effective_parent_id": "a",
                "physical_page_index": 5,
                "status": "auto_confirmed",
            },
            {
                "candidate_id": "c",
                "effective_title": "Not In The Gold Set",
                "effective_parent_id": None,
                "physical_page_index": 9,
                "status": "needs_review",
            },
        ]
    }
    report = {"auto_confirmed": 2, "mode": "toc_aligned"}
    perfect = aggregate([evaluate_document(gold, candidates, report)])
    assert perfect["target_page_accuracy"]["value"] == 1.0, perfect
    assert perfect["title_precision"]["value"] == 1.0, perfect
    assert perfect["gold_coverage"]["value"] == 1.0, perfect
    assert perfect["hierarchy_edge_f1"] == 1.0, perfect
    assert perfect["hallucination_count"] == 0, perfect
    assert perfect["zero_edit_document_success"]["value"] == 1.0, perfect

    # A written title that is not in the gold set is a hallucination, and a
    # wrong target page is not counted as accurate.
    hallucinated = json.loads(json.dumps(candidates))
    hallucinated["candidates"][2]["status"] = "auto_confirmed"
    hallucinated["candidates"][1]["physical_page_index"] = 6
    degraded = aggregate([evaluate_document(gold, hallucinated, report)])
    assert degraded["hallucination_count"] == 1, degraded
    assert degraded["hallucination_gate_passed"] is False, degraded
    assert degraded["target_page_accuracy"] == {
        "numerator": 1,
        "denominator": 2,
        "value": 0.5,
    }, degraded
    assert degraded["title_precision"]["denominator"] == 3, degraded
    assert degraded["zero_edit_document_success"]["value"] == 0.0, degraded

    # A safe-refusal document is correct exactly when nothing was written.
    refusal_gold = {
        "document_id": "refusal",
        "expected_behavior": "safe_refusal",
        "annotated": True,
        "bookmarks": [],
    }
    refused = aggregate(
        [evaluate_document(refusal_gold, {"candidates": []}, {"auto_confirmed": 0})]
    )
    assert refused["safe_refusal_document_accuracy"]["value"] == 1.0, refused
    wrote_anyway = aggregate(
        [evaluate_document(refusal_gold, candidates, {"auto_confirmed": 2})]
    )
    assert wrote_anyway["safe_refusal_document_accuracy"]["value"] == 0.0, wrote_anyway
    assert wrote_anyway["hallucination_count"] == 2, wrote_anyway

    # An empty denominator is reported as null, never as a perfect score.
    empty = aggregate(
        [
            evaluate_document(
                {
                    "document_id": "empty",
                    "expected_behavior": "needs_review",
                    "annotated": True,
                    "bookmarks": [],
                },
                {"candidates": []},
                {"auto_confirmed": 0},
            )
        ]
    )
    assert empty["target_page_accuracy"]["value"] is None, empty
    assert empty["gold_coverage"]["value"] is None, empty
    return {"status": "self_test_passed"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pack", type=Path, help="evaluation pack directory")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check the metric formulas against synthetic data and exit",
    )
    parser.add_argument("--output", type=Path, help="write the JSON report here")
    arguments = parser.parse_args()
    if arguments.self_test:
        report = self_test()
    elif arguments.pack:
        try:
            report = evaluate_pack(arguments.pack)
        except EvalError as error:
            print(f"error: {error}", file=sys.stderr)
            return 1
    else:
        parser.error("pass --pack or --self-test")
        return 2
    text = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True)
    if arguments.output:
        arguments.output.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
