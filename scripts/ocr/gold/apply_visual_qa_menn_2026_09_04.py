#!/usr/bin/env python3
"""Apply the 2026-09-04 visual-AI candidate QA pass to six Menn pages.

This script deliberately leaves every line and page gate unverified.  It only
repairs visually observed candidate transcription/structure issues so a human
reviewer can work from a better draft.
"""

from __future__ import annotations

import copy
import re
import unicodedata
from pathlib import Path

import closed_world_gold as gold


ROOT = Path(__file__).resolve().parents[3]
PAGES = ROOT / "gold-data" / "mixed-scholarly-v1" / "pages"
PAGE_IDS = (
    "menn-metaphysics-pdf288",
    "menn-metaphysics-pdf337",
    "menn-metaphysics-pdf414",
    "menn-metaphysics-pdf522",
    "menn-metaphysics-pdf577",
    "menn-metaphysics-pdf925",
)
GREEK_RE = re.compile(r"[\u0370-\u03ff\u1f00-\u1fff]")


def line(record: dict, order: int) -> dict:
    candidate = record["lines"][order]
    if candidate["reading_order"] != order:
        raise RuntimeError(f"unexpected reading order at {record['page_id']}:{order}")
    return candidate


def replace_at(record: dict, order: int, old: str, new: str) -> None:
    candidate = line(record, order)
    if old not in candidate["text"]:
        raise RuntimeError(
            f"expected text missing at {record['page_id']}:{order}: {old!r}"
        )
    candidate["text"] = candidate["text"].replace(old, new)


def replace_page(record: dict, replacements: tuple[tuple[str, str], ...]) -> None:
    for candidate in record["lines"]:
        text = candidate["text"]
        for old, new in replacements:
            text = text.replace(old, new)
        candidate["text"] = text


def set_note_range(record: dict, start: int, end: int, note_id: str) -> None:
    for order in range(start, end + 1):
        candidate = line(record, order)
        candidate["content_class"] = "footnote"
        candidate["column_id"] = "notes"
        candidate["note_id"] = note_id
        candidate["paragraph_role"] = "start" if order == start else "continuation"


def set_marker_spans(
    candidate: dict, markers: tuple[tuple[str, str | None], ...]
) -> None:
    spans = []
    cursor = 0
    for index, (marker, target_id) in enumerate(markers):
        start = candidate["text"].find(marker, cursor)
        if start < 0:
            raise RuntimeError(
                f"marker {marker!r} missing from {candidate['line_id']}: "
                f"{candidate['text']!r}"
            )
        end = start + len(marker)
        spans.append(
            {
                "span_id": f"{candidate['line_id']}-visual-qa-marker-{index:02d}",
                "start_char": start,
                "end_char": end,
                "text": marker,
                "styles": ["superscript"],
                "semantic_role": "footnote_marker" if target_id else "other",
                "target_id": target_id,
                "bbox": None,
            }
        )
        cursor = end
    candidate["inline_spans"] = spans


def finalize_candidate(record: dict, unresolved: str) -> None:
    for order, candidate in enumerate(record["lines"]):
        candidate["reading_order"] = order
        candidate["text"] = unicodedata.normalize("NFC", candidate["text"])
        candidate["source"] = "human_draft"
        candidate["geometry_status"] = "candidate_unverified"
        candidate["structure_status"] = "candidate_unverified"
        candidate["transcription_status"] = "candidate_unverified"
        candidate["typography_status"] = "candidate_unverified"
        if candidate["language"] == "eng" and GREEK_RE.search(candidate["text"]):
            candidate["language"] = "mixed"

    record["coverage"] = {
        "status": "draft",
        "closed_world": True,
        "all_visible_lines_exhaustively_reviewed": False,
        "all_structure_exhaustively_reviewed": False,
        "all_typography_exhaustively_reviewed": False,
        "reviewer": "",
        "verified_at": None,
        "unresolved_notes": (
            "VISUAL AI QA CANDIDATE PASS (Codex image review, 2026-09-04). "
            "Geometry/text/footnote structure were corrected where visually clear; "
            "nothing is human verified. "
            + unresolved
        ),
    }


def qa_288(record: dict) -> None:
    replace_page(
        record,
        (
            ("µ", "μ"),
            ("ἀλλ ᾿", "ἀλλ᾿"),
            ("καθ ᾿", "καθ᾿"),
            ("1025b341026a3", "1025b34-1026a3"),
        ),
    )
    line(record, 7)["paragraph_role"] = "start"
    replace_at(record, 16, "there seems", "19 there seems")
    line(record, 16)["bbox"] = [268, 1423, 2077, 1470]
    del record["lines"][17]
    for order, candidate in enumerate(record["lines"]):
        candidate["reading_order"] = order
    set_note_range(record, 16, 22, "menn-metaphysics-pdf288-note-19")
    set_note_range(record, 23, 43, "menn-metaphysics-pdf288-note-20")
    set_marker_spans(
        line(record, 1), (("19", "menn-metaphysics-pdf288-note-19"),)
    )
    set_marker_spans(
        line(record, 3), (("20", "menn-metaphysics-pdf288-note-20"),)
    )
    set_marker_spans(
        line(record, 16), (("19", "menn-metaphysics-pdf288-note-19"),)
    )
    set_marker_spans(
        line(record, 23), (("20", "menn-metaphysics-pdf288-note-20"),)
    )
    finalize_candidate(
        record,
        "The opening block is a cross-page continuation and footnote 18 also begins "
        "on the preceding page; exact punctuation, Greek accents, and typography need "
        "human comparison.",
    )


def qa_337(record: dict) -> None:
    replace_page(record, (("", "∃"),))
    line(record, 12)["text"] = (
        'to explain away "the odd lines 1017a27-30 in Metaphysics V 7," '
        "LSD p.269 n14).43, 44, 45"
    )
    replace_at(record, 13, "43Owen's", "43 Owen's")
    replace_at(record, 43, "44some", "44 some")
    set_note_range(record, 13, 42, "menn-metaphysics-pdf337-note-43")
    set_note_range(record, 43, 52, "menn-metaphysics-pdf337-note-44")
    set_marker_spans(
        line(record, 12),
        (
            ("43", "menn-metaphysics-pdf337-note-43"),
            ("44", "menn-metaphysics-pdf337-note-44"),
            ("45", None),
        ),
    )
    set_marker_spans(
        line(record, 13), (("43", "menn-metaphysics-pdf337-note-43"),)
    )
    set_marker_spans(
        line(record, 43), (("44", "menn-metaphysics-pdf337-note-44"),)
    )
    finalize_candidate(
        record,
        "Footnote 45 continues outside this page and is therefore not linked here. "
        "Exact punctuation, underlining, and footnote wording require human review.",
    )


def qa_414(record: dict) -> None:
    replacements = {
        0: (("L10", "Λ10"),),
        1: (("in L", "in Λ"),),
        3: (("that L", "that Λ"),),
        4: (("of L", "of Λ"),),
        5: (("in L", "in Λ"),),
        6: (("that L", "that Λ"),),
        7: ((". L", ". Λ"),),
        9: (("ajrcaiv", "ἀρχαί"),),
        10: (("L merely", "Λ merely"),),
        14: (("the A-M and a-b differences", "the Λ-M and α-β differences"),),
        15: (("treating A9 in Ib1", "treating Λ9 in Iβ1"),),
        19: (
            ("Ia4 and IIIg3", "Iα4 and IIIγ3"),
            ("the L10 parallel", "the Λ10 parallel"),
        ),
        21: (("marks L,", "marks Λ,"),),
        31: (("passage L chapter 10", "passage Λ chapter 10"),),
        34: (("see Ia4", "see Iα4"),),
        35: (("theme of N and L", "theme of N and Λ"),),
        41: (
            ("from the L parallel", "from the Λ parallel"),
            ("confined to L", "confined to Λ"),
        ),
        46: (("an ajrchv", "an ἀρχή"),),
        49: (
            ("the ajrchv is a stoicei'on", "the ἀρχή is a στοιχεῖον"),
            ("is an ajrchv", "is an ἀρχή"),
        ),
    }
    for order, pairs in replacements.items():
        for old, new in pairs:
            replace_at(record, order, old, new)
    line(record, 14)["paragraph_role"] = "start"
    set_note_range(record, 50, 51, "menn-metaphysics-pdf414-note-26")
    set_marker_spans(
        line(record, 2), (("26", "menn-metaphysics-pdf414-note-26"),)
    )
    set_marker_spans(
        line(record, 50), (("26", "menn-metaphysics-pdf414-note-26"),)
    )
    finalize_candidate(
        record,
        "The long unnumbered braced authorial note below the rule remains a provisional "
        "footnote-region classification. Exact Greek accents, punctuation, and typography "
        "need human comparison.",
    )


def qa_522(record: dict) -> None:
    replace_page(
        record,
        (
            ("ajporouvmenai oujsivai", "ἀπορούμεναι οὐσίαι"),
            ("oJmologouvmenai oujsivai", "ὁμολογούμεναι οὐσίαι"),
            ("oujsivai", "οὐσίαι"),
            ("aujtou'", "αὐτοῦ"),
            ("kaiv", "καί"),
            ("aJplw'\"", "ἁπλῶς"),
            ("ejn tai'\" pravxesi", "ἐν ταῖς πράξεσι"),
        ),
    )
    replace_at(record, 15, "be dev or", "be δέ or")
    replace_at(record, 10, "32I have", "32 I have")
    replace_at(record, 18, "33The point", "33 The point")
    line(record, 4)["paragraph_role"] = "start"
    set_note_range(record, 10, 17, "menn-metaphysics-pdf522-note-32")
    set_note_range(record, 18, 51, "menn-metaphysics-pdf522-note-33")
    set_marker_spans(
        line(record, 3), (("32", "menn-metaphysics-pdf522-note-32"),)
    )
    set_marker_spans(
        line(record, 9), (("33", "menn-metaphysics-pdf522-note-33"),)
    )
    set_marker_spans(
        line(record, 10), (("32", "menn-metaphysics-pdf522-note-32"),)
    )
    set_marker_spans(
        line(record, 18), (("33", "menn-metaphysics-pdf522-note-33"),)
    )
    finalize_candidate(
        record,
        "The page contains two very long notes; exact Greek accents, line-end spacing, "
        "underlining, and note typography still require human comparison.",
    )


def qa_577(record: dict) -> None:
    replace_page(
        record,
        (
            ("oJ leuko;\" a[nqrwpo\"", "ὁ λευκὸς ἄνθρωπος"),
            ("leuko;\" a[nqrwpo\"", "λευκὸς ἄνθρωπος"),
            ("oJ divkaio\"", "ὁ δίκαιος"),
            ("oJ leukov\"", "ὁ λευκός"),
            ("to; leukovn", "τὸ λευκόν"),
            ("o{per leukovn", "ὅπερ λευκόν"),
            ("o{per tovde ti", "ὅπερ τόδε τι"),
            ("a[llo kat j a[llou", "ἄλλο καθ᾿ ἄλλου"),
            ("kaq j auJtov", "καθ᾿ αὑτό"),
            ("dikaiosuvnh", "δικαιοσύνη"),
            ("toiovnde", "τοιόνδε"),
            ("oujsivai", "οὐσίαι"),
            ("oujsiva", "οὐσία"),
            ("e[kqesi\"", "ἔκθεσις"),
            ("lovgo\"", "λόγος"),
            ("pavqo\"", "πάθος"),
            ("o{per", "ὅπερ"),
            ("divkaio\"", "δίκαιος"),
            ("tovde", "τόδε"),
            ("sw'ma", "σῶμα"),
            ("leukovn", "λευκόν"),
            ("i[dia", "ἴδια"),
            ("Ia2", "Iα2"),
        ),
    )
    replace_at(record, 38, "16contrast", "16 contrast")
    replace_at(record, 47, "17this", "17 this")
    replace_at(record, 50, "18cp.", "18 cp.")
    set_note_range(record, 38, 46, "menn-metaphysics-pdf577-note-16")
    set_note_range(record, 47, 49, "menn-metaphysics-pdf577-note-17")
    set_note_range(record, 50, 50, "menn-metaphysics-pdf577-note-18")
    set_marker_spans(
        line(record, 12), (("17", "menn-metaphysics-pdf577-note-17"),)
    )
    set_marker_spans(
        line(record, 15), (("18", "menn-metaphysics-pdf577-note-18"),)
    )
    set_marker_spans(
        line(record, 38), (("16", "menn-metaphysics-pdf577-note-16"),)
    )
    set_marker_spans(
        line(record, 47), (("17", "menn-metaphysics-pdf577-note-17"),)
    )
    set_marker_spans(
        line(record, 50), (("18", "menn-metaphysics-pdf577-note-18"),)
    )
    finalize_candidate(
        record,
        "The upper note block continues footnote 15 from the preceding page. This page is "
        "Greek-dense; every accent, breathing, quote mark, and italic/underline span still "
        "requires human comparison.",
    )


def qa_925(record: dict) -> None:
    replace_page(
        record,
        (
            ("ejnergei'n", "ἐνεργεῖν"),
            ("ejnevrgeia", "ἐνέργεια"),
            ("noei'n", "νοεῖν"),
            ("fronei'", "φρονεῖ"),
            ("noei'", "νοεῖ"),
            ("nou'\"", "νοῦς"),
            ("duvnami\"", "δύναμις"),
            ("oujsiva", "οὐσία"),
            ("novhsi\"", "νόησις"),
            ("semnovn", "σεμνόν"),
            ("kuvrion", "κύριον"),
            ("tivmion", "τίμιον"),
            ("ajrchv", "ἀρχή"),
            ("L9", "Λ9"),
            ("arues", "argues"),
            ("queston", "question"),
        ),
    )
    replace_at(record, 17, "31cp.", "31 cp.")
    replace_at(record, 35, "32Brunschwig", "32 Brunschwig")
    replace_at(record, 44, "33in case", "33 in case")
    line(record, 3)["paragraph_role"] = "start"
    set_note_range(record, 17, 34, "menn-metaphysics-pdf925-note-31")
    set_note_range(record, 35, 43, "menn-metaphysics-pdf925-note-32")
    set_note_range(record, 44, 50, "menn-metaphysics-pdf925-note-33")
    set_marker_spans(
        line(record, 0), (("31", "menn-metaphysics-pdf925-note-31"),)
    )
    set_marker_spans(
        line(record, 16),
        (
            ("32", "menn-metaphysics-pdf925-note-32"),
            ("33", "menn-metaphysics-pdf925-note-33"),
        ),
    )
    set_marker_spans(
        line(record, 17), (("31", "menn-metaphysics-pdf925-note-31"),)
    )
    set_marker_spans(
        line(record, 35), (("32", "menn-metaphysics-pdf925-note-32"),)
    )
    set_marker_spans(
        line(record, 44), (("33", "menn-metaphysics-pdf925-note-33"),)
    )
    finalize_candidate(
        record,
        "Footnote 33 continues onto the next page. Exact Greek accents, punctuation, "
        "underlining, and note text remain for exhaustive human review.",
    )


QA = {
    "menn-metaphysics-pdf288": qa_288,
    "menn-metaphysics-pdf337": qa_337,
    "menn-metaphysics-pdf414": qa_414,
    "menn-metaphysics-pdf522": qa_522,
    "menn-metaphysics-pdf577": qa_577,
    "menn-metaphysics-pdf925": qa_925,
}


def main() -> int:
    originals = {page_id: gold.load_record(PAGES / f"{page_id}.json") for page_id in PAGE_IDS}
    updated = copy.deepcopy(originals)
    for page_id in PAGE_IDS:
        QA[page_id](updated[page_id])
        gold.validate_record(updated[page_id], require_complete=False)

    for page_id in PAGE_IDS:
        gold.atomic_write_json(PAGES / f"{page_id}.json", updated[page_id])
        print(
            f"{page_id}: {len(originals[page_id]['lines'])} -> "
            f"{len(updated[page_id]['lines'])} candidate lines"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
