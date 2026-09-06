#!/usr/bin/env python3
"""Prepare the second mixed-scholarly visual-annotation batch.

The generated records are model-assisted annotation drafts, never gold.  Native
PDF text is used where available because it preserves useful font and baseline
evidence; historical scans are seeded with Tesseract line geometry.  Every
field remains ``candidate_unverified`` until the human workbench review.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import re
import subprocess
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path

import fitz
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))

import closed_world_gold as gold  # noqa: E402


ROOT = Path(__file__).resolve().parents[3]
CORPUS = ROOT / "gold-data" / "mixed-scholarly-v1"
DPI = 300
SCALE = DPI / 72
PREANNOTATION_NOTE = (
    "VISUAL-MODEL PREANNOTATION ONLY (Codex interactive vision, 2026-08-31). "
    "Line geometry, reading order, page furniture, notes/apparatus, language, "
    "paragraph structure, and available native typography were prefilled. "
    "Every visible line and exact character still requires exhaustive human review."
)
GREEK_RE = re.compile(r"[\u0370-\u03ff\u1f00-\u1fff]")
WORD_RE = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+")


@dataclass(frozen=True)
class Source:
    source_id: str
    path: Path
    pages: tuple[int, ...]
    mode: str
    default_language: str
    features: tuple[str, ...]


SOURCES = (
    Source(
        "bude-sophiste-cuf",
        Path("/Volumes/Haoran/Plato/1.7 Sophist/01_primary_texts/critical_editions/(1925) Auguste Diès - Platon_ Le Sophiste [CUF Budé].pdf"),
        (24, 25, 26, 27),
        "scan_spread",
        "fra",
        ("fra", "grc", "critical-edition", "apparatus", "two-page-spread", "gutter"),
    ),
    Source(
        "warren-osap36",
        Path("/Volumes/Haoran/speusipp/06_伦理学_快乐论_价值/(2009) James Warren - Aristotle on Speusippus on Eudoxus on Pleasure [OSAP 36].pdf"),
        (14, 15, 16, 17, 18),
        "native_journal",
        "eng",
        ("eng", "grc", "journal", "footnotes", "section-headings"),
    ),
    Source(
        "dillon-phronesis29",
        Path("/Volumes/Haoran/speusipp/03_本原论_形而上学/(1984) John Dillon - Speusippus in Iamblichus [Phronesis 29.3].pdf"),
        (2, 3, 4, 5, 6),
        "native_journal",
        "eng",
        ("eng", "grc", "journal", "footnotes", "article-title"),
    ),
    Source(
        "ueberweg-kraemer-aeltere-akademie",
        Path("/Volumes/Haoran/speusipp/02_专著_通论_书评/(2004) Hans Krämer - Die Ältere Akademie [Ueberweg].pdf"),
        (3, 4),
        "scan_columns",
        "deu",
        ("deu", "grc", "two-column", "small-type", "historical-scan"),
    ),
    Source(
        "primavesi-zwei-elementen",
        Path("/Volumes/Haoran/speusipp/03_本原论_形而上学/(2024) Oliver Primavesi - Aristoteles und Speusipp über die Platonische Zwei-Elementen-Lehre [De Gruyter].pdf"),
        (10, 11, 12, 13, 14, 15, 16, 79),
        "native_critical_study",
        "deu",
        ("deu", "grc", "modern-scholarship", "apparatus", "footnotes", "critical-signs"),
    ),
    Source(
        "menn-metaphysics",
        Path("/Volumes/Haoran/学者全集/Menn_Stephen/The Aim and the Argument of Aristotle's Metaphysics/The Aim and the Argument of Aristotle's Metaphysics.pdf"),
        (288, 337, 414, 522, 577, 925),
        "native_dense",
        "eng",
        ("eng", "grc", "dense-prose", "footnotes", "long-lines", "scholarly-commentary"),
    ),
)


def page_id(source: Source, page_number: int) -> str:
    digits = 4 if page_number >= 1000 else 3
    return f"{source.source_id}-pdf{page_number:0{digits}d}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def render_page(document: fitz.Document, pdf_page: int, destination: Path) -> None:
    pixmap = document[pdf_page - 1].get_pixmap(dpi=DPI, alpha=False)
    destination.parent.mkdir(parents=True, exist_ok=True)
    pixmap.save(destination)


def infer_language(text: str, default: str) -> str:
    greek = len(GREEK_RE.findall(text))
    latin = len(WORD_RE.findall(text))
    if greek and latin:
        return "mixed"
    if greek:
        return "grc"
    return default


def _bbox_to_pixels(bbox: tuple[float, float, float, float]) -> list[int]:
    return [max(0, round(value * SCALE)) for value in bbox]


def _native_lines(page: fitz.Page, identity: str) -> list[dict]:
    extracted: list[dict] = []
    block_ordinal = 0
    for block in page.get_text("dict", flags=fitz.TEXTFLAGS_DICT)["blocks"]:
        if block.get("type") != 0:
            continue
        for line_ordinal, source_line in enumerate(block.get("lines", [])):
            source_spans = source_line.get("spans", [])
            groups = [source_spans]
            # Several publishers encode a running head and its page number as
            # adjacent spans on one PDF text line.  They are distinct visible
            # objects and must therefore become distinct annotation boxes.
            if source_line["bbox"][1] < 70 and len(source_spans) > 1:
                first = source_spans[0].get("text", "").strip()
                last = source_spans[-1].get("text", "").strip()
                if _is_numeric_page(first):
                    groups = [source_spans[:1], source_spans[1:]]
                elif _is_numeric_page(last):
                    groups = [source_spans[:-1], source_spans[-1:]]
            for group_ordinal, group in enumerate(groups):
                raw = "".join(span.get("text", "") for span in group)
                leading = len(raw) - len(raw.lstrip())
                text = unicodedata.normalize("NFC", raw.strip())
                if not text:
                    continue
                span_records = []
                cursor = 0
                for span in group:
                    span_text = unicodedata.normalize("NFC", span.get("text", ""))
                    start = cursor - leading
                    end = start + len(span_text)
                    cursor += len(span_text)
                    clipped_start = max(0, start)
                    clipped_end = min(len(text), end)
                    if clipped_start >= clipped_end:
                        continue
                    flags = int(span.get("flags", 0))
                    font = str(span.get("font", ""))
                    styles = []
                    if flags & 2 or "italic" in font.lower() or "oblique" in font.lower():
                        styles.append("italic")
                    if flags & 16 or "bold" in font.lower():
                        styles.append("bold")
                    if flags & 1:
                        styles.append("superscript")
                    if "smallcap" in font.lower():
                        styles.append("small_caps")
                    if styles:
                        span_records.append(
                            {
                                "start": clipped_start,
                                "end": clipped_end,
                                "text": text[clipped_start:clipped_end],
                                "styles": styles,
                            }
                        )
                sized_text = [
                    (float(span.get("size", 0)), len(span.get("text", "").strip()))
                    for span in group
                    if span.get("text", "").strip()
                ]
                bbox = _bbox_to_pixels(
                    (
                        min(span["bbox"][0] for span in group),
                        min(span["bbox"][1] for span in group),
                        max(span["bbox"][2] for span in group),
                        max(span["bbox"][3] for span in group),
                    )
                )
                extracted.append(
                    {
                        "line_id": f"{identity}-native-{len(extracted):04d}",
                        "bbox": bbox,
                        "text": text,
                        "_size": (
                            sum(size * length for size, length in sized_text)
                            / sum(length for _size, length in sized_text)
                            if sized_text
                            else 0
                        ),
                        "_block": block_ordinal,
                        "_block_first": line_ordinal == 0 and group_ordinal == 0,
                        "_native_spans": span_records,
                    }
                )
        block_ordinal += 1
    return extracted


def _tesseract_lines(image_path: Path, identity: str, languages: str) -> list[dict]:
    result = subprocess.run(
        ["tesseract", str(image_path), "stdout", "--oem", "1", "--psm", "3", "-l", languages, "tsv"],
        check=True,
        capture_output=True,
        text=True,
    )
    groups: dict[tuple[int, int, int, int], list[dict]] = collections.defaultdict(list)
    for row in result.stdout.splitlines()[1:]:
        fields = row.split("\t", 11)
        if len(fields) != 12:
            continue
        try:
            level, page_num, block_num, par_num, line_num, _word_num = map(int, fields[:6])
            left, top, width, height = map(int, fields[6:10])
            confidence = float(fields[10])
        except ValueError:
            continue
        text = unicodedata.normalize("NFC", fields[11].strip())
        if level != 5 or not text:
            continue
        groups[(page_num, block_num, par_num, line_num)].append(
            {"text": text, "bbox": [left, top, left + width, top + height], "confidence": confidence}
        )
    lines = []
    for words in groups.values():
        words.sort(key=lambda word: word["bbox"][0])
        lines.append(
            {
                "line_id": f"{identity}-tesseract-{len(lines):04d}",
                "bbox": [
                    min(word["bbox"][0] for word in words),
                    min(word["bbox"][1] for word in words),
                    max(word["bbox"][2] for word in words),
                    max(word["bbox"][3] for word in words),
                ],
                "text": " ".join(word["text"] for word in words),
                "_confidence": sum(word["confidence"] for word in words) / len(words),
                "_block_first": False,
                "_native_spans": [],
            }
        )
    return lines


def _body_size(lines: list[dict], height: int) -> float:
    weighted = collections.Counter()
    for line in lines:
        if 0.08 * height < line["bbox"][1] < 0.9 * height and line.get("_size", 0) > 0:
            weighted[round(line["_size"], 1)] += max(1, len(line["text"]))
    if not weighted:
        return 10.0
    total = sum(weighted.values())
    prevalent = [size for size, weight in weighted.items() if weight >= total * 0.08]
    return float(max(prevalent)) if prevalent else float(weighted.most_common(1)[0][0])


def _base_line(line: dict, identity: str, default_language: str) -> dict:
    spans = []
    for ordinal, span in enumerate(line.get("_native_spans", [])):
        spans.append(
            {
                "span_id": f"{line['line_id']}-native-span-{ordinal:03d}",
                "start_char": span["start"],
                "end_char": span["end"],
                "text": span["text"],
                "styles": span["styles"],
                "semantic_role": "citation" if "italic" in span["styles"] else "none",
                "target_id": None,
                "bbox": None,
            }
        )
    return {
        "line_id": line["line_id"],
        "reading_order": 0,
        "bbox": line["bbox"],
        "text": line["text"],
        "language": infer_language(line["text"], default_language),
        "content_class": "main_text",
        "geometry_status": "candidate_unverified",
        "structure_status": "candidate_unverified",
        "transcription_status": "candidate_unverified",
        "typography_status": "candidate_unverified",
        "paragraph_role": "start" if line.get("_block_first") else "continuation",
        "leaf_id": "single",
        "column_id": "main",
        "hierarchy_level": None,
        "note_id": None,
        "canonical_reference": None,
        "inline_spans": spans,
        "source": "human_draft",
    }


def _add_span(
    line: dict,
    start: int,
    end: int,
    *,
    styles: tuple[str, ...],
    semantic_role: str = "none",
) -> None:
    if start < 0 or end <= start or end > len(line["text"]):
        return
    if any(start < span["end_char"] and end > span["start_char"] for span in line["inline_spans"]):
        return
    line["inline_spans"].append(
        {
            "span_id": f"{line['line_id']}-visual-span-{len(line['inline_spans']):03d}",
            "start_char": start,
            "end_char": end,
            "text": line["text"][start:end],
            "styles": list(styles),
            "semantic_role": semantic_role,
            "target_id": None,
            "bbox": None,
        }
    )
    line["inline_spans"].sort(key=lambda span: (span["start_char"], span["end_char"]))


def _is_numeric_page(text: str) -> bool:
    return bool(re.fullmatch(r"[ivxlcdmIVXLCDM]+|\d{1,4}", text.strip()))


def annotate_native(
    raw_lines: list[dict], identity: str, source: Source, width: int, height: int
) -> list[dict]:
    body_size = _body_size(raw_lines, height)
    raw_lines.sort(key=lambda line: (line["bbox"][1], line["bbox"][0]))
    output = []
    current_note = None
    note_counter = 0
    for raw in raw_lines:
        line = _base_line(raw, identity, source.default_language)
        left, top, right, bottom = line["bbox"]
        size = raw.get("_size", body_size)
        centered = abs(((left + right) / 2) - width / 2) < width * 0.12
        top_zone = top < height * 0.075
        bottom_zone = bottom > height * 0.94
        small = size <= body_size - 0.55
        heading_pattern = bool(
            re.match(
                r"^(?:\d+\.|\d+\.\d+(?:\.\d+)*\.?|[IVX]+\.|[A-Z]\.|\([A-Z]\))\s+\S",
                line["text"],
            )
        )

        if (top_zone or bottom_zone) and _is_numeric_page(line["text"]):
            line.update(
                content_class="page_number", language="zxx", paragraph_role="not_applicable",
                column_id="furniture"
            )
        elif top_zone and (centered or len(line["text"]) < 90):
            line.update(
                content_class="running_head", paragraph_role="not_applicable", column_id="furniture"
            )
        elif bottom_zone and len(line["text"]) < 140:
            line.update(content_class="footer", paragraph_role="not_applicable", column_id="furniture")
        elif small and (source.mode == "native_critical_study" or top > height * 0.35):
            critical = source.mode == "native_critical_study" and (
                top < height * 0.38
                or bool(
                re.search(
                    r"(?:\bom\.|\bcod\.|varia lectio|Lesart|Konjektur|\bOA\b|\bArn\b|γραφ|\]\s*:)",
                    line["text"], re.I,
                )
                )
            )
            if critical:
                line.update(
                    content_class="apparatus", paragraph_role="standalone", column_id="apparatus"
                )
            else:
                marker = re.match(r"^\s*(\d{1,3})\s+", line["text"])
                if marker:
                    current_note = marker.group(1)
                    role = "start"
                else:
                    role = "continuation"
                if current_note is None:
                    note_counter += 1
                    current_note = f"unresolved-{note_counter}"
                line.update(
                    content_class="footnote",
                    paragraph_role=role,
                    column_id="notes",
                    note_id=f"{identity}-note-{current_note}",
                )
        elif (
            source.source_id == "dillon-phronesis29"
            and size >= body_size + 1.2
            and centered
            and top < height * 0.35
        ):
            line.update(
                content_class="document_title" if top < height * 0.22 else "section_heading",
                paragraph_role="standalone",
            )
            for span in line["inline_spans"]:
                span["semantic_role"] = "title"
        elif (
            source.mode != "native_dense"
            and (
                heading_pattern
                or (centered and bool(re.fullmatch(r"[IVX]{1,5}", line["text"])))
            )
        ):
            line.update(content_class="section_heading", paragraph_role="standalone")
            for span in line["inline_spans"]:
                span["semantic_role"] = "title"
        output.append(line)

    note_targets = {
        re.search(r"-note-(.+)$", line["note_id"]).group(1): line["note_id"]
        for line in output
        if line.get("note_id") and re.search(r"-note-(.+)$", line["note_id"])
    }
    for line in output:
        if line["content_class"] not in {"main_text", "section_heading"}:
            continue
        for span in line["inline_spans"]:
            if "superscript" not in span["styles"]:
                continue
            marker = span["text"].strip()
            if marker in note_targets:
                span.update(semantic_role="footnote_marker", target_id=note_targets[marker])
    for order, line in enumerate(output):
        line["reading_order"] = order
    return output


def annotate_scan_spread(
    raw_lines: list[dict], identity: str, source: Source, width: int, height: int
) -> list[dict]:
    half = width / 2
    header_labels = {
        "bude-sophiste-cuf-pdf024": ("304", "218a"),
        "bude-sophiste-cuf-pdf025": ("305", "218e"),
        "bude-sophiste-cuf-pdf026": ("306", "219c"),
        "bude-sophiste-cuf-pdf027": ("307", "219e"),
    }
    printed_page, stephanus_page = header_labels[identity]
    raw_lines = [
        line
        for line in raw_lines
        if line["bbox"][1] >= height * 0.07
        and not (
            line["bbox"][3] - line["bbox"][1]
            > 4 * max(1, line["bbox"][2] - line["bbox"][0])
            and len(line["text"]) < 20
        )
    ]
    header_specs = (
        ("left-stephanus", [75, 95, 190, 150], stephanus_page),
        ("left-head", [540, 95, 930, 155], "LE SOPHISTE"),
        ("left-page", [1150, 95, 1260, 155], printed_page),
        ("right-page", [1530, 95, 1640, 155], printed_page),
        ("right-head", [1870, 95, 2240, 155], "ΣΟΦΙΣΤΗΣ"),
        ("right-stephanus", [2510, 95, 2650, 155], stephanus_page),
    )
    for suffix, bbox, text in header_specs:
        raw_lines.append(
            {
                "line_id": f"{identity}-visual-{suffix}",
                "bbox": bbox,
                "text": text,
                "_block_first": False,
                "_native_spans": [],
            }
        )
    raw_lines.sort(
        key=lambda line: (
            0 if (line["bbox"][0] + line["bbox"][2]) / 2 < half else 1,
            line["bbox"][1],
            line["bbox"][0],
        )
    )
    output = []
    for raw in raw_lines:
        line = _base_line(raw, identity, source.default_language)
        left, top, right, bottom = line["bbox"]
        center = (left + right) / 2
        leaf = "left" if center < half else "right"
        leaf_left = 0 if leaf == "left" else half
        line["leaf_id"] = f"scan-{leaf}"
        stephanus = re.fullmatch(r"(\d{2,3})\s*([a-e])", line["text"].strip(), re.I)
        margin_letter = re.fullmatch(r"[a-e]", line["text"].strip(), re.I)
        if stephanus:
            label = f"{stephanus.group(1)}{stephanus.group(2).lower()}"
            line.update(
                content_class="marginal_page_label",
                language="zxx",
                paragraph_role="not_applicable",
                column_id="margin",
                canonical_reference={
                    "kind": "marginal_page_label",
                    "system": "stephanus",
                    "label": label,
                    "anchor_line_id": None,
                },
            )
        elif margin_letter and top >= height * 0.085:
            label = margin_letter.group(0).lower()
            line.update(
                content_class="marginal_line_number",
                language="zxx",
                paragraph_role="not_applicable",
                column_id="margin",
                canonical_reference={
                    "kind": "marginal_line_number",
                    "system": "stephanus-section",
                    "label": label,
                    "anchor_line_id": None,
                },
            )
        elif top < height * 0.085 and _is_numeric_page(line["text"]):
            line.update(
                content_class="page_number", language="zxx", paragraph_role="not_applicable",
                column_id="furniture"
            )
        elif top < height * 0.07:
            line.update(
                content_class="running_head", paragraph_role="not_applicable", column_id="furniture"
            )
            _add_span(line, 0, len(line["text"]), styles=("small_caps",))
        elif "Seminar für Byzantinistik" in line["text"]:
            line.update(
                content_class="marginalia", paragraph_role="standalone", column_id="margin"
            )
        elif top > height * 0.78:
            line.update(content_class="apparatus", paragraph_role="standalone", column_id="apparatus")
        else:
            inset = left - leaf_left
            line.update(
                content_class="main_text",
                paragraph_role="start" if inset > half * 0.13 else "continuation",
                column_id="main",
            )
            speaker = re.match(
                r"^(?:L['’]ÉTRANGER|L['’]ETRANGER|THÉÉTÈTE|THEETETE|SOCRATE|ΞΕ\.|ΘΕΑΙ\.|ΣΩ\.)",
                line["text"], re.I,
            )
            if speaker:
                _add_span(line, speaker.start(), speaker.end(), styles=("small_caps",))
        output.append(line)
    for order, line in enumerate(output):
        line["reading_order"] = order
    return output


def annotate_scan_columns(
    raw_lines: list[dict], identity: str, source: Source, width: int, height: int
) -> list[dict]:
    half = width / 2
    expected_headers = {
        "ueberweg-kraemer-aeltere-akademie-pdf003": (
            "3", "§ 1. Die Ältere Akademie im Allgemeinen"
        ),
        "ueberweg-kraemer-aeltere-akademie-pdf004": (
            "4", "§ 1. Die Ältere Akademie im Allgemeinen (Bibl. 130–138)"
        ),
    }
    page_text, header_text = expected_headers[identity]
    raw_lines = [line for line in raw_lines if line["bbox"][1] >= height * 0.09]
    raw_lines.extend(
        (
            {
                "line_id": f"{identity}-visual-page",
                "bbox": [320, 175, 385, 235],
                "text": page_text,
                "_block_first": False,
                "_native_spans": [],
            },
            {
                "line_id": f"{identity}-visual-head",
                "bbox": [470, 175, 1510, 240],
                "text": header_text,
                "_block_first": False,
                "_native_spans": [],
            },
        )
    )
    furniture = [line for line in raw_lines if line["bbox"][1] < height * 0.17]
    body = [line for line in raw_lines if line not in furniture]
    left = [line for line in body if (line["bbox"][0] + line["bbox"][2]) / 2 < half]
    right = [line for line in body if line not in left]
    ordered = sorted(furniture, key=lambda line: (line["bbox"][1], line["bbox"][0]))
    ordered += sorted(left, key=lambda line: (line["bbox"][1], line["bbox"][0]))
    ordered += sorted(right, key=lambda line: (line["bbox"][1], line["bbox"][0]))
    output = []
    for raw in ordered:
        line = _base_line(raw, identity, source.default_language)
        if identity.endswith("pdf003") and line["text"].startswith("und Überlieferung"):
            line["text"] = "1. Zeugnisse " + line["text"]
        left_x, top, right_x, bottom = line["bbox"]
        center = (left_x + right_x) / 2
        if top < height * 0.09 and _is_numeric_page(line["text"]):
            line.update(
                content_class="page_number", language="zxx", paragraph_role="not_applicable",
                column_id="furniture"
            )
        elif top < height * 0.09:
            line.update(
                content_class="running_head", paragraph_role="not_applicable", column_id="furniture"
            )
            _add_span(line, 0, len(line["text"]), styles=("small_caps",))
        elif top < height * 0.23 and len(line["text"]) < 130:
            line.update(content_class="section_heading", paragraph_role="standalone", column_id="main")
            _add_span(line, 0, len(line["text"]), styles=("bold",), semantic_role="title")
        elif top > height * 0.91 and (bottom - top) < height * 0.025:
            line.update(content_class="footnote", paragraph_role="standalone", column_id="notes")
            line["note_id"] = f"{identity}-note-unresolved"
        else:
            column = "left" if center < half else "right"
            column_edge = width * (0.08 if column == "left" else 0.55)
            line.update(
                content_class="main_text",
                paragraph_role="start" if left_x > column_edge + width * 0.018 else "continuation",
                column_id=column,
            )
        line["leaf_id"] = "single"
        output.append(line)
    for order, line in enumerate(output):
        line["reading_order"] = order
    return output


def candidate_record(identity: str, image_digest: str, raw_lines: list[dict], source: Source) -> dict:
    provider = (
        {"id": "tesseract", "version": "5.5.3", "oem": 1, "psm": 3}
        if source.mode.startswith("scan_")
        else {"id": "native_pdf_text", "library": "PyMuPDF", "candidate_only": True}
    )
    return {
        "schema": "mpdf-geometry-annotation-candidate/1",
        "page_id": identity,
        "status": "candidate_unverified_not_gold",
        "image_sha256": image_digest,
        "provider": provider,
        "lines": [
            {
                "reading_order": order,
                "bbox": line["bbox"],
                "candidate_text": line["text"],
            }
            for order, line in enumerate(raw_lines)
        ],
    }


def prepare(force: bool) -> None:
    image_dir = CORPUS / "images"
    candidate_dir = CORPUS / "candidates"
    page_dir = CORPUS / "pages"
    manifest_pages = []
    manifest_sources = []
    candidate_total = 0
    for source in SOURCES:
        if not source.path.is_file():
            raise SystemExit(f"missing source: {source.path}")
        source_digest = sha256(source.path)
        document = fitz.open(source.path)
        manifest_sources.append(
            {
                "source_id": source.source_id,
                "path": str(source.path),
                "sha256": source_digest,
                "pages": list(source.pages),
            }
        )
        for pdf_page in source.pages:
            identity = page_id(source, pdf_page)
            image_path = image_dir / f"{identity}.png"
            candidate_path = candidate_dir / f"{identity}.json"
            record_path = page_dir / f"{identity}.json"
            if not force and any(path.exists() for path in (image_path, candidate_path, record_path)):
                raise SystemExit(f"refusing to overwrite existing batch page: {identity}")
            render_page(document, pdf_page, image_path)
            with Image.open(image_path) as image:
                width, height = image.size
            image_digest = sha256(image_path)
            if source.mode.startswith("scan_"):
                languages = "fra+grc+lat" if source.default_language == "fra" else "deu+grc+lat"
                raw_lines = _tesseract_lines(image_path, identity, languages)
            else:
                raw_lines = _native_lines(document[pdf_page - 1], identity)
            gold.atomic_write_json(
                candidate_path, candidate_record(identity, image_digest, raw_lines, source)
            )
            if source.mode == "scan_spread":
                lines = annotate_scan_spread(raw_lines, identity, source, width, height)
            elif source.mode == "scan_columns":
                lines = annotate_scan_columns(raw_lines, identity, source, width, height)
            else:
                lines = annotate_native(raw_lines, identity, source, width, height)
            record = gold.draft_from_image(image_path, identity)
            record["coverage"]["unresolved_notes"] = PREANNOTATION_NOTE
            record["lines"] = lines
            gold.validate_record(record, require_complete=False, image_path=image_path)
            gold.atomic_write_json(record_path, record)
            candidate_total += len(lines)
            manifest_pages.append(
                {
                    "page_id": identity,
                    "source_id": source.source_id,
                    "pdf_page": pdf_page,
                    "image_file": f"images/{identity}.png",
                    "image_sha256": image_digest,
                    "width": width,
                    "height": height,
                    "features": list(source.features),
                    "candidate_line_count": len(lines),
                }
            )
            print(f"{identity}: {len(lines)} candidate lines")
        document.close()
    manifest = {
        "schema": "mpdf-mixed-scholarly-visual-selection/1",
        "created_at": "2026-08-31",
        "status": "visual_model_preannotated_candidate_drafts_not_gold",
        "render": {"dpi": DPI, "format": "png", "renderer": "PyMuPDF", "color_policy": "original"},
        "selection_policy": {
            "page_count": len(manifest_pages),
            "purpose": "second mixed-layout geometry, structure, transcription, and typography holdout batch",
            "allocation": {
                "Bude critical edition": 4,
                "OSAP": 5,
                "Phronesis": 5,
                "Kraemer Ueberweg": 2,
                "Primavesi": 8,
                "Menn": 6,
            },
        },
        "workbench_review": {
            "candidate_status": "schema_1_2_candidate_unverified_not_gold",
            "candidate_line_count": candidate_total,
            "visual_model_preannotation": {
                "completed": True,
                "provider": "Codex interactive vision session plus native PDF/Tesseract scaffolding",
                "status": "candidate_unverified_not_gold",
            },
        },
        "sources": manifest_sources,
        "pages": manifest_pages,
    }
    gold.atomic_write_json(CORPUS / "selection-2026-08-31.json", manifest)
    print(f"TOTAL: {len(manifest_pages)} pages, {candidate_total} candidate lines")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    prepare(args.force)


if __name__ == "__main__":
    main()
