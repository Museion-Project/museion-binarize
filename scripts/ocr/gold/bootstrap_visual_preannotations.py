#!/usr/bin/env python3
"""Bootstrap the 2026-08-30 mixed-scholarly pages after visual inspection.

This is deliberately candidate-only.  It encodes page-specific layout decisions
made from the rendered page images, but never marks model-assisted evidence as
human verified.
"""

from __future__ import annotations

import argparse
import copy
import re
import sys
import unicodedata
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import closed_world_gold as gold  # noqa: E402


GREEK_RE = re.compile(r"[\u0370-\u03ff\u1f00-\u1fff]")
NUMBERED_NOTE_RE = re.compile(r"^\(?\s*(\d+)\s*\)?[.)]?")


def normalize_line(line: dict) -> None:
    line.update(
        geometry_status="candidate_unverified",
        structure_status="candidate_unverified",
        transcription_status="candidate_unverified",
        typography_status="candidate_unverified",
        source="human_draft",
    )
    line.setdefault("hierarchy_level", None)
    line.setdefault("canonical_reference", None)
    line.setdefault("inline_spans", [])
    line.setdefault("note_id", None)


def reset_page(record: dict, message: str) -> None:
    coverage = record["coverage"]
    coverage.update(
        status="draft",
        all_visible_lines_exhaustively_reviewed=False,
        all_structure_exhaustively_reviewed=False,
        all_typography_exhaustively_reviewed=False,
        reviewer="",
        verified_at=None,
        unresolved_notes=message,
    )
    for line in record["lines"]:
        normalize_line(line)


def language_of(text: str, default: str) -> str:
    greek = len(GREEK_RE.findall(text))
    latin = sum(character.isalpha() and not GREEK_RE.match(character) for character in text)
    if greek and latin:
        return "mixed"
    if greek:
        return "grc"
    english_score = len(re.findall(r"\b(the|and|with|from|of)\b", text, re.I))
    french_score = len(
        re.findall(r"\b(les|des|dans|avec|études|philosophie)\b", text, re.I)
    )
    if english_score >= 2:
        return "eng"
    if french_score >= 2:
        return "fra"
    return default


def renumber(record: dict) -> None:
    for order, line in enumerate(record["lines"]):
        line["reading_order"] = order


def add_span(
    line: dict,
    phrase: str,
    *,
    styles: tuple[str, ...] = (),
    role: str = "none",
    target: str | None = None,
) -> bool:
    start = line["text"].find(phrase)
    if start < 0:
        return False
    end = start + len(phrase)
    if any(
        start < span["end_char"] and end > span["start_char"]
        for span in line["inline_spans"]
    ):
        return False
    used = {span["span_id"] for span in line["inline_spans"]}
    counter = 0
    while f"{line['line_id']}-visual-{counter:04d}" in used:
        counter += 1
    line["inline_spans"].append(
        {
            "span_id": f"{line['line_id']}-visual-{counter:04d}",
            "start_char": start,
            "end_char": end,
            "text": phrase,
            "styles": list(styles),
            "semantic_role": role,
            "target_id": target,
            "bbox": None,
        }
    )
    line["inline_spans"].sort(
        key=lambda span: (span["start_char"], span["end_char"], span["span_id"])
    )
    return True


def refresh_existing_spans(line: dict) -> None:
    rebuilt = []
    cursor = 0
    for span in line.get("inline_spans", []):
        start = line["text"].find(span["text"], cursor)
        if start < 0:
            start = line["text"].find(span["text"])
        if start < 0:
            continue
        copy = dict(span)
        copy["start_char"] = start
        copy["end_char"] = start + len(copy["text"])
        rebuilt.append(copy)
        cursor = copy["end_char"]
    line["inline_spans"] = sorted(
        rebuilt, key=lambda span: (span["start_char"], span["end_char"], span["span_id"])
    )


def split_spread_headers(record: dict) -> None:
    output = []
    for line in record["lines"]:
        text = line["text"].strip()
        left, top, right, bottom = line["bbox"]
        leading = re.match(r"^(\d{2,4})\s+(.+)$", text)
        trailing = re.match(r"^(.+?)\s+(\d{2,4})$", text)
        if top < 220 and leading:
            page_text, header_text = leading.groups()
            page_line = dict(line)
            page_line.update(
                line_id=f"{record['page_id']}-visual-page-left",
                bbox=[left, top, min(right, left + 150), bottom],
                text=page_text,
                language="zxx",
                content_class="page_number",
                paragraph_role="not_applicable",
                column_id="furniture",
                inline_spans=[],
            )
            header_line = dict(line)
            header_line.update(
                line_id=f"{record['page_id']}-visual-head-left",
                bbox=[min(right - 1, left + 210), top, right, bottom],
                text=header_text,
                language=language_of(header_text, "fra"),
                content_class="running_head",
                paragraph_role="not_applicable",
                column_id="furniture",
                inline_spans=[],
            )
            add_span(header_line, header_text, styles=("bold", "small_caps"))
            output.extend((page_line, header_line))
        elif top < 220 and trailing:
            header_text, page_text = trailing.groups()
            header_line = dict(line)
            header_line.update(
                line_id=f"{record['page_id']}-visual-head-right",
                bbox=[left, top, max(left + 1, right - 150), bottom],
                text=header_text,
                language=language_of(header_text, "fra"),
                content_class="running_head",
                paragraph_role="not_applicable",
                column_id="furniture",
                inline_spans=[],
            )
            add_span(header_line, header_text, styles=("bold", "small_caps"))
            page_line = dict(line)
            page_line.update(
                line_id=f"{record['page_id']}-visual-page-right",
                bbox=[max(left, right - 100), top, right, bottom],
                text=page_text,
                language="zxx",
                content_class="page_number",
                paragraph_role="not_applicable",
                column_id="furniture",
                inline_spans=[],
            )
            output.extend((header_line, page_line))
        else:
            output.append(line)
    record["lines"] = output
    renumber(record)


def annotate_toc(record: dict) -> None:
    targets = {
        1: "30", 2: "31", 3: "35", 4: "35", 5: "36", 6: "36",
        7: "38", 8: "41", 9: "44", 10: "47", 11: "47", 12: "48",
        13: "48", 14: "50", 15: "50", 16: "50", 17: "52", 19: "55",
        20: "55", 21: "58", 22: "64", 23: "71", 24: "71", 25: "72",
        26: "72", 27: "73", 28: "76", 29: "76", 30: "81", 31: "84",
        32: "84", 33: "85", 34: "86", 35: "98",
    }
    explicit_numbers = {
        1: 36, 2: 37, 4: 38, 5: 39, 7: 40, 8: 41, 9: 42, 10: 43,
        11: 44, 12: 45, 13: 46, 26: 47, 27: 48, 28: 49, 30: 50,
        32: 51, 33: 52, 34: 53, 35: 54,
    }
    lines = record["lines"]
    by_order = {line["reading_order"]: line for line in lines}
    merged = []
    for order in range(36):
        if order == 18:  # gutter artefact
            continue
        line = by_order[order]
        line.update(
            leaf_id="contents-1",
            column_id="toc",
            note_id=None,
            inline_spans=[],
            language=language_of(line["text"], "fra"),
            paragraph_role="standalone",
        )
        if order == 0:
            line.update(
                content_class="table_of_contents_title",
                hierarchy_level=1,
                canonical_reference=None,
            )
            add_span(line, line["text"], styles=("bold",), role="title")
        else:
            match = re.match(r"^(\d+(?:\.\d+)*\.?|[A-Z]\.|\d+\.|[a-z]\))", line["text"])
            token = match.group(1) if match else ""
            if token and token[0].isdigit():
                level = token.count(".") + (1 if token.endswith(".") else 2)
            elif token and token[0].isupper():
                level = 4
            elif token:
                level = 5
            else:
                level = 3 if order in {4, 15, 26, 32} else 4
                line["paragraph_role"] = "continuation"
            destination = targets[order]
            line.update(
                content_class="table_of_contents_entry",
                hierarchy_level=min(6, level),
                canonical_reference={
                    "kind": "toc_destination",
                    "system": "printed_page",
                    "label": destination,
                    "anchor_line_id": None,
                },
            )
            number_order = explicit_numbers.get(order)
            if number_order is not None:
                number_line = by_order[number_order]
                line["bbox"] = [
                    min(line["bbox"][0], number_line["bbox"][0]),
                    min(line["bbox"][1], number_line["bbox"][1]),
                    max(line["bbox"][2], number_line["bbox"][2]),
                    max(line["bbox"][3], number_line["bbox"][3]),
                ]
            if not re.search(rf"\b{re.escape(destination)}$", line["text"]):
                line["text"] = f"{line['text'].rstrip()} {destination}"
            add_span(
                line,
                destination,
                role="canonical_reference",
                target=f"printed-page-{destination}",
            )
        merged.append(line)
    record["lines"] = merged
    renumber(record)


BRISSON_CONFIG = {
    "brisson-le-meme-pdf110": {"left_notes": 2300, "right_notes": 2230, "headings": {23, 42, 58}},
    "brisson-le-meme-pdf170": {"left_notes": 1960, "right_notes": 2100, "headings": set()},
}


def annotate_brisson(record: dict) -> None:
    split_spread_headers(record)
    config = BRISSON_CONFIG[record["page_id"]]
    half = record["image"]["width"] / 2
    note_number = {"left": None, "right": None}
    previous_main = {"left": None, "right": None}
    for line in record["lines"]:
        normalize_line(line)
        center = (line["bbox"][0] + line["bbox"][2]) / 2
        leaf = "left" if center < half else "right"
        line["leaf_id"] = f"scan-{leaf}"
        line["hierarchy_level"] = None
        line["canonical_reference"] = None
        line["note_id"] = None
        if line["content_class"] in {"page_number", "running_head"}:
            line["paragraph_role"] = "not_applicable"
            continue
        original_order = int(re.search(r"(\d{4})$", line["line_id"]).group(1)) if re.search(r"(\d{4})$", line["line_id"]) else -1
        note_cutoff = config[f"{leaf}_notes"]
        if line["bbox"][1] >= note_cutoff:
            line.update(content_class="footnote", column_id="notes")
            match = NUMBERED_NOTE_RE.match(line["text"].replace(")", ") ", 1))
            if match:
                note_number[leaf] = match.group(1)
                line["paragraph_role"] = "start"
            else:
                line["paragraph_role"] = "continuation"
            note = note_number[leaf] or "unresolved"
            line["note_id"] = f"{record['page_id']}-{leaf}-note-{note}"
        elif original_order in config["headings"]:
            line.update(
                content_class="section_heading",
                column_id="main",
                paragraph_role="standalone",
            )
            add_span(line, line["text"], styles=("bold",), role="title")
        else:
            line.update(content_class="main_text", column_id="main")
            base = 205 if leaf == "left" else 1890
            start = line["bbox"][0] >= base + 35 or previous_main[leaf] is None
            line["paragraph_role"] = "start" if start else "continuation"
            previous_main[leaf] = line
        line["language"] = language_of(line["text"], "fra")
        for phrase in ("Timée", "République", "Phys.", "Rep.", "Id.", "In Tim.", "Pl. cosm.", "Soph."):
            add_span(line, phrase, styles=("italic",), role="citation")

    notes = {line["note_id"] for line in record["lines"] if line.get("note_id")}
    for line in record["lines"]:
        if line["content_class"] != "main_text":
            continue
        leaf = "left" if line["leaf_id"].endswith("left") else "right"
        for match in reversed(list(re.finditer(r"(?<=[A-Za-zÀ-ÿ»)])([1-9])(?=[.,;:]?$)", line["text"]))):
            target = f"{record['page_id']}-{leaf}-note-{match.group(1)}"
            if target in notes:
                add_span(
                    line,
                    match.group(1),
                    styles=("superscript",),
                    role="footnote_marker",
                    target=target,
                )


def annotate_existing_brisson_first(record: dict) -> None:
    corrections = {
        17: "est toujours en acte. »2 Il semble donc, d'après le passage cité,",
        19: "principes ψυχή — νοῦς ἐν δυνάμει — νοῦς κατ᾽ ἐνέργειαν — πρῶτος θεός3.",
        35: "contre Ueberweg-Praechter (Die Philos. des Altertums, I, p. 542), Witt (Albinus, pp. 128-",
        39: "nous in Albinus is always a function of the world-soul. » (« Albinus' metaphysics », Mn.,",
        90: "p. 88 Leemans) témoignent que Numénius était « pythagoricien ». Alors que Jamblique",
    }
    singleton_notes = {32, 33, 47, 82, 83, 84}
    for line in record["lines"]:
        order = line["reading_order"]
        if order in corrections:
            line["text"] = unicodedata.normalize("NFC", corrections[order])
        if order == 2:
            line["paragraph_role"] = "continuation"
        if order in singleton_notes:
            line["paragraph_role"] = "standalone"
        refresh_existing_spans(line)
        normalize_line(line)
        if line["content_class"] == "running_head":
            add_span(line, line["text"], styles=("bold", "small_caps"))
        if order == 83:
            add_span(line, "Id.", styles=("italic",), role="citation")


def annotate_kramer(record: dict) -> None:
    page_id = record["page_id"]
    width = record["image"]["width"]
    if page_id == "ueberweg-kraemer-pdf100":
        first = record["lines"][0]
        first["text"] = "100 § 7. Weitere Akademiker der ersten Generation (Bibl. 156-160)"
        split_spread_headers(record)
        for line in record["lines"]:
            normalize_line(line)
            line.update(leaf_id="printed-100", column_id="main", hierarchy_level=None, note_id=None, canonical_reference=None)
            if line["content_class"] in {"page_number", "running_head"}:
                line["paragraph_role"] = "not_applicable"
            elif line["bbox"][1] in range(560, 750):
                line.update(content_class="section_heading", paragraph_role="standalone")
                add_span(line, line["text"], styles=("italic", "bold"), role="title")
            elif 1900 <= line["bbox"][1] <= 2200:
                line.update(content_class="equation", paragraph_role="standalone", language="zxx")
            else:
                line.update(
                    content_class="main_text",
                    paragraph_role="start" if line["bbox"][0] >= 240 else "continuation",
                    language=language_of(line["text"], "deu"),
                )
        equations = [line for line in record["lines"] if line["content_class"] == "equation"]
        if len(equations) >= 4:
            equations[0]["text"] = "∜2"
            equations[1]["text"] = "(1 + √2) / 2"
            equations[1]["bbox"] = [
                min(equations[1]["bbox"][0], equations[2]["bbox"][0]),
                min(equations[1]["bbox"][1], equations[2]["bbox"][1]),
                max(equations[1]["bbox"][2], equations[2]["bbox"][2]),
                max(equations[1]["bbox"][3], equations[2]["bbox"][3]),
            ]
            equations[3]["text"] = "2 (2 − √2);"
            record["lines"].remove(equations[2])
        if not any(line["text"].strip() == "führt." for line in record["lines"]):
            template = copy.deepcopy(record["lines"][-1])
            template.update(
                line_id=f"{page_id}-visual-missed-final-line",
                bbox=[208, 2390, 355, 2435],
                text="führt.",
                language="deu",
                content_class="main_text",
                paragraph_role="continuation",
                inline_spans=[],
            )
            record["lines"].append(template)
        renumber(record)
        return

    # Bibliography pages: preserve the visually correct left-column then right-column order.
    if page_id == "ueberweg-kraemer-pdf145":
        first = record["lines"].pop(0)
        header = copy.deepcopy(first)
        header.update(line_id=f"{page_id}-visual-head", bbox=[760, 140, 980, 190], text="Xenokrates")
        page = copy.deepcopy(first)
        page.update(line_id=f"{page_id}-visual-page", bbox=[1480, 140, 1580, 190], text="145")
        record["lines"] = [header, page, *record["lines"]]
    header_lines = [line for line in record["lines"] if line["bbox"][1] < 210]
    body = [line for line in record["lines"] if line["bbox"][1] >= 210]
    left = sorted((line for line in body if line["bbox"][0] < width / 2), key=lambda line: (line["bbox"][1], line["bbox"][0]))
    right = sorted((line for line in body if line["bbox"][0] >= width / 2), key=lambda line: (line["bbox"][1], line["bbox"][0]))
    record["lines"] = [*header_lines, *left, *right]
    heading_re = re.compile(
        r"^(Aetios|Albinos|Alexander|Ps\.-Galenos|Synesios|Vita |Zur Chronologie|Zum Leben|Theologie |Die kosmologische|und das Problem)"
    )
    for line in record["lines"]:
        normalize_line(line)
        line.update(
            leaf_id=f"printed-{page_id[-3:]}",
            hierarchy_level=None,
            note_id=None,
            canonical_reference=None,
        )
        if line["bbox"][1] < 210:
            if line["text"].strip().isdigit():
                line.update(content_class="page_number", language="zxx")
            else:
                line.update(content_class="running_head", language="deu")
                add_span(line, line["text"], styles=("small_caps",))
            line.update(column_id="furniture", paragraph_role="not_applicable")
            continue
        line["column_id"] = "left" if line["bbox"][0] < width / 2 else "right"
        line["language"] = language_of(line["text"], "deu")
        if heading_re.match(line["text"]):
            line.update(content_class="section_heading", paragraph_role="standalone")
            add_span(line, line["text"], styles=("italic",), role="title")
        else:
            line.update(
                content_class="bibliography",
                paragraph_role="start" if re.match(r"^\d{3}\s", line["text"]) else "continuation",
            )
    renumber(record)


BURNET_CONFIG = {
    "burnet-platonis-opera-pdf0050": {
        "apparatus_y": 1740, "title": "ΠΛΑΤΩΝΟΣ", "page": "155e",
        "title_box": [590, 95, 875, 150], "page_box": [120, 100, 220, 150],
    },
    "burnet-platonis-opera-pdf0300": {
        "apparatus_y": 1770, "title": "ΠΛΑΤΩΝΟΣ", "page": "276e",
        "title_box": [555, 115, 850, 170], "page_box": [75, 115, 205, 170],
    },
    "burnet-platonis-opera-pdf1100": {
        "apparatus_y": 1670, "title": "ΜΙΝΩΣ", "page": "320a",
        "title_box": [505, 95, 700, 150], "page_box": [1090, 100, 1220, 155],
    },
    "burnet-platonis-opera-pdf1400": {
        "apparatus_y": 1670, "title": "ΝΟΜΩΝ Θ΄ ΙΧ", "page": "869c",
        "title_box": [450, 75, 1050, 135], "page_box": [1090, 75, 1220, 140],
    },
}


def annotate_burnet(record: dict) -> None:
    config = BURNET_CONFIG[record["page_id"]]
    body = []
    for line in record["lines"][1:]:
        top = line["bbox"][1]
        if top < config["apparatus_y"] and len(GREEK_RE.findall(line["text"])) < 4:
            continue
        body.append(line)
    template = record["lines"][0]
    page_line = dict(template)
    page_line.update(
        line_id=f"{record['page_id']}-visual-stephanus",
        bbox=config["page_box"],
        text=config["page"],
        language="zxx",
        content_class="marginal_page_label",
        paragraph_role="not_applicable",
        leaf_id="single",
        column_id="margin",
        hierarchy_level=None,
        note_id=None,
        canonical_reference={
            "kind": "marginal_page_label",
            "system": "stephanus",
            "label": config["page"],
            "anchor_line_id": None,
        },
        inline_spans=[],
    )
    title_line = dict(template)
    title_line.update(
        line_id=f"{record['page_id']}-visual-running-head",
        bbox=config["title_box"],
        text=config["title"],
        language="grc",
        content_class="running_head",
        paragraph_role="not_applicable",
        leaf_id="single",
        column_id="furniture",
        hierarchy_level=None,
        note_id=None,
        canonical_reference=None,
        inline_spans=[],
    )
    add_span(title_line, title_line["text"], styles=("bold", "small_caps"))
    record["lines"] = [page_line, title_line, *body]
    for line in record["lines"]:
        normalize_line(line)
        if line in (page_line, title_line):
            continue
        line.update(
            leaf_id="single",
            column_id="apparatus" if line["bbox"][1] >= config["apparatus_y"] else "main",
            hierarchy_level=None,
            note_id=None,
            canonical_reference=None,
            inline_spans=[],
        )
        if line["bbox"][1] >= config["apparatus_y"]:
            line.update(content_class="apparatus", language="mixed")
            line["paragraph_role"] = "start" if line is next((item for item in record["lines"] if item["bbox"][1] >= config["apparatus_y"]), None) else "continuation"
        else:
            line.update(content_class="main_text", language="grc")
            line["paragraph_role"] = (
                "start"
                if re.match(r"^(ΣΩ|ΦΑΙ|ΚΛ|ΑΘ|EQ|QAI|SAI)\.", line["text"])
                else "continuation"
            )
    renumber(record)


def run(root: Path, selected_pages: set[str] | None = None) -> None:
    pages = root / "pages"
    images = root / "images"
    message = (
        "VISUAL-MODEL PREANNOTATION ONLY (Codex interactive vision, 2026-08-31). "
        "Structure, labels, note groups, references, and obvious typography were prefilled; "
        "OCR text and every visible line still require exhaustive human review."
    )
    for path in sorted(pages.glob("*.json")):
        record = gold.load_record(path)
        if selected_pages is not None and record["page_id"] not in selected_pages:
            continue
        reset_page(record, message)
        page_id = record["page_id"]
        if page_id == "brisson-le-meme-pdf013":
            annotate_toc(record)
        elif page_id == "brisson-le-meme-pdf030":
            annotate_existing_brisson_first(record)
        elif page_id.startswith("brisson-"):
            annotate_brisson(record)
        elif page_id.startswith("ueberweg-"):
            annotate_kramer(record)
        elif page_id.startswith("burnet-"):
            annotate_burnet(record)
        else:
            raise gold.GoldError(f"unsupported visual-preannotation page {page_id}")
        for line in record["lines"]:
            line["text"] = unicodedata.normalize("NFC", line["text"])
            normalize_line(line)
        renumber(record)
        image_path = images / f"{page_id}.png"
        gold.validate_record(record, require_complete=False, image_path=image_path)
        gold.atomic_write_json(path, record)
        print(f"{page_id}: {len(record['lines'])} lines")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path("gold-data/mixed-scholarly-v1"),
    )
    parser.add_argument("--pages", nargs="+", help="limit a one-time run to page ids")
    args = parser.parse_args()
    run(args.root, set(args.pages) if args.pages else None)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
