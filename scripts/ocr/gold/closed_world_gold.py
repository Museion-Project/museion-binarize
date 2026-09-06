#!/usr/bin/env python3
"""Closed-world OCR gold records and fail-closed validation.

PAGE XML is accepted only as annotation scaffolding.  A record becomes gold
only after a human has verified every visible line, explicitly asserted that
the page was exhaustively reviewed, and removed every unresolved note.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import tempfile
import unicodedata
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Mapping

import cgpg


SCHEMA = "mpdf-closed-world-ocr-gold-page"
SCHEMA_VERSION = "1.2"
FIRST_PARTY_LEGACY_SCHEMA_VERSION = "1.1"
CGPG_SCHEMA_VERSION = "1.0"
SUPPORTED_SCHEMA_VERSIONS = {
    CGPG_SCHEMA_VERSION,
    FIRST_PARTY_LEGACY_SCHEMA_VERSION,
    SCHEMA_VERSION,
}
MANIFEST_SCHEMA = "mpdf-closed-world-ocr-gold-manifest"
MANIFEST_VERSION = "1.0"
LANGUAGES = ("grc", "lat", "deu", "fra", "eng", "mixed", "zxx", "und")
LEGACY_CONTENT_CLASSES = (
    "main_text",
    "apparatus",
    "running_head",
    "page_number",
    "footnote",
    "caption",
    "other",
    "unclassified",
)
CONTENT_CLASSES = (
    "document_title",
    "table_of_contents_title",
    "table_of_contents_entry",
    "section_heading",
    "main_text",
    "apparatus",
    "running_head",
    "footer",
    "page_number",
    "footnote",
    "bibliography",
    "marginal_page_label",
    "marginal_line_number",
    "marginalia",
    "caption",
    "table",
    "equation",
    "other",
    "unclassified",
)
PARAGRAPH_ROLES = (
    "start",
    "continuation",
    "standalone",
    "not_applicable",
    "unclassified",
)
INLINE_STYLES = (
    "italic",
    "bold",
    "superscript",
    "subscript",
    "underline",
    "small_caps",
)
INLINE_SEMANTIC_ROLES = (
    "none",
    "footnote_marker",
    "canonical_reference",
    "emphasis",
    "title",
    "citation",
    "other",
)
REFERENCE_KINDS = (
    "marginal_page_label",
    "marginal_line_number",
    "canonical_reference",
    "toc_destination",
)
HOLDOUT_PAGE_IDS = (
    "grc_grna_or_399",
    "grc_grna_or_409",
    "grc_grna_or_423",
    "grc_grna_or_429",
    "grc_grna_or_443",
    "grc_grna_or_447",
    "grc_grna_or_461",
    "grc_grna_or_475",
    "grc_grna_or_487",
    "grc_grna_or_489",
    "grc_grna_or_503",
    "grc_grna_or_513",
)
FORENSIC_PAGE_IDS = (
    "grc_grna_or_399",
    "grc_grna_or_443",
    "grc_grna_or_489",
    "grc_grna_or_503",
)


class GoldError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json(value: object) -> bytes:
    return (
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        + "\n"
    ).encode("utf-8")


def atomic_write_json(path: Path, value: object) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(canonical_json(value))
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def infer_language(text: str) -> str:
    label = cgpg.script_of_text(text)
    return {"greek": "grc", "latin": "lat"}.get(label, "mixed")


def _reconcile_line_source(line: dict) -> None:
    """Keep provenance honest while the two modules are reviewed separately."""
    statuses = [line["geometry_status"], line["transcription_status"]]
    if "structure_status" in line:
        statuses.extend((line["structure_status"], line["typography_status"]))
    line["source"] = (
        "human" if all(status == "human_verified" for status in statuses) else "human_draft"
    )


def invalidate_line_geometry(
    line: dict, *, invalidate_transcription: bool = False
) -> None:
    """Invalidate geometry, and optionally text when the box changes identity."""
    line["geometry_status"] = "candidate_unverified"
    if invalidate_transcription:
        line["transcription_status"] = "candidate_unverified"
        if "typography_status" in line:
            line["typography_status"] = "candidate_unverified"
    _reconcile_line_source(line)


def invalidate_line_structure(line: dict) -> None:
    if "structure_status" in line:
        line["structure_status"] = "candidate_unverified"
    else:
        line["geometry_status"] = "candidate_unverified"
    _reconcile_line_source(line)


def invalidate_line_transcription(line: dict) -> None:
    """Invalidate the character module without discarding reviewed geometry."""
    line["transcription_status"] = "candidate_unverified"
    if "typography_status" in line:
        line["typography_status"] = "candidate_unverified"
    _reconcile_line_source(line)


def invalidate_line_typography(line: dict) -> None:
    if "typography_status" in line:
        line["typography_status"] = "candidate_unverified"
    else:
        line["transcription_status"] = "candidate_unverified"
    _reconcile_line_source(line)


def mark_line_geometry_verified(line: dict) -> None:
    """Accept the line box, order, and v1.2 structural metadata."""
    if "structure_status" in line:
        if line.get("paragraph_role") not in PARAGRAPH_ROLES or line["paragraph_role"] == "unclassified":
            raise GoldError("line paragraph role must be classified")
        reference = line.get("canonical_reference")
        _validate_canonical_reference(reference)
        if line.get("content_class") in {
            "marginal_page_label",
            "marginal_line_number",
        } and not reference:
            raise GoldError("marginal reference metadata is required")
        if line.get("content_class") == "table_of_contents_entry" and (
            not _valid_hierarchy_level(line.get("hierarchy_level"))
            or line.get("hierarchy_level") is None
            or not reference
            or reference.get("kind") != "toc_destination"
        ):
            raise GoldError("contents entries require hierarchy and destination metadata")
        line["structure_status"] = "human_verified"
    line["geometry_status"] = "human_verified"
    _reconcile_line_source(line)


def mark_line_transcription_verified(line: dict) -> None:
    """Accept exact text, labels, and v1.2 inline typography after geometry."""
    required_geometry = [line.get("geometry_status")]
    if "structure_status" in line:
        required_geometry.append(line.get("structure_status"))
    if any(status != "human_verified" for status in required_geometry):
        raise GoldError("line geometry module must be human verified before characters")
    text = line.get("text")
    if not isinstance(text, str) or not text.strip():
        raise GoldError("transcription cannot be empty")
    if unicodedata.normalize("NFC", text) != text:
        raise GoldError("transcription must be NFC-normalized")
    if line.get("language") not in LANGUAGES:
        raise GoldError("line language is invalid")
    if (
        line.get("content_class") not in CONTENT_CLASSES
        or line["content_class"] == "unclassified"
    ):
        raise GoldError("line content class must be classified")
    line["transcription_status"] = "human_verified"
    if "typography_status" in line:
        _validate_inline_spans(line, 0, 0, check_bbox=False)
        if line["content_class"] == "footnote" and not line.get("note_id"):
            raise GoldError("footnote lines require a note_id")
        line["typography_status"] = "human_verified"
    _reconcile_line_source(line)


def upgrade_record_to_v1_2(record: dict) -> dict:
    """Upgrade a first-party v1.1 page in place without relabeling new evidence."""
    if record.get("schema_version") == SCHEMA_VERSION:
        return record
    if record.get("schema_version") != FIRST_PARTY_LEGACY_SCHEMA_VERSION:
        return record
    record["schema_version"] = SCHEMA_VERSION
    coverage = record["coverage"]
    coverage["all_structure_exhaustively_reviewed"] = False
    coverage["all_typography_exhaustively_reviewed"] = False
    coverage["footnote_marker_exceptions"] = []
    coverage["external_footnote_targets"] = []
    coverage["status"] = "draft"
    coverage["verified_at"] = None
    for line in record["lines"]:
        line.update(
            structure_status="candidate_unverified",
            typography_status="candidate_unverified",
            paragraph_role="unclassified",
            leaf_id=None,
            column_id=None,
            hierarchy_level=None,
            note_id=None,
            canonical_reference=None,
            inline_spans=[],
        )
        _reconcile_line_source(line)
    return record


def draft_from_cgpg(page: cgpg.Page) -> dict:
    """Creates an explicitly unverified draft from CGPG PAGE XML."""
    lines = []
    for region in page.regions:
        for candidate in region.lines:
            left, top, right, bottom = candidate.bbox
            ordinal = len(lines)
            lines.append(
                {
                    "line_id": f"{page.name}-l{ordinal:04d}",
                    "reading_order": ordinal,
                    "bbox": [left, top, right, bottom],
                    "text": candidate.text,
                    "language": infer_language(candidate.text),
                    "content_class": "unclassified",
                    "geometry_status": "candidate_unverified",
                    "transcription_status": "candidate_unverified",
                    "source": "cgpg_page_xml_candidate",
                }
            )
    return {
        "schema": SCHEMA,
        "schema_version": CGPG_SCHEMA_VERSION,
        "page_id": page.name,
        "image": {
            "sha256": sha256_file(page.image_path),
            "width": page.width,
            "height": page.height,
            "mime_type": "image/jpeg",
        },
        "coverage": {
            "status": "draft",
            "closed_world": True,
            "all_visible_lines_exhaustively_reviewed": False,
            "reviewer": "",
            "verified_at": None,
            "unresolved_notes": "",
        },
        "candidate_provenance": {
            "kind": "cgpg_page_xml_scaffolding_only",
            "xml_sha256": sha256_file(page.xml_path),
            "warning": "Not gold until every line and page coverage are human_verified.",
        },
        "lines": lines,
    }


def draft_from_image(image_path: Path, page_id: str | None = None) -> dict:
    """Creates a blank v1.2 draft for first-party closed-world annotation."""
    from PIL import Image

    image_path = Path(image_path)
    with Image.open(image_path) as image:
        width, height = image.size
        image_format = (image.format or "").upper()
    mime_type = {
        "JPEG": "image/jpeg",
        "PNG": "image/png",
        "TIFF": "image/tiff",
    }.get(image_format)
    if mime_type is None:
        raise GoldError(
            f"unsupported annotation image format: {image_format or 'unknown'}"
        )
    identity = page_id or image_path.stem
    if not identity:
        raise GoldError("page_id is required")
    return {
        "schema": SCHEMA,
        "schema_version": SCHEMA_VERSION,
        "page_id": identity,
        "image": {
            "sha256": sha256_file(image_path),
            "width": width,
            "height": height,
            "mime_type": mime_type,
        },
        "coverage": {
            "status": "draft",
            "closed_world": True,
            "all_visible_lines_exhaustively_reviewed": False,
            "all_structure_exhaustively_reviewed": False,
            "all_typography_exhaustively_reviewed": False,
            "footnote_marker_exceptions": [],
            "external_footnote_targets": [],
            "reviewer": "",
            "verified_at": None,
            "unresolved_notes": "",
        },
        "annotation_seed": {
            "kind": "blank_human_annotation",
            "warning": "Not gold until every visible line and page coverage are human_verified.",
        },
        "lines": [],
    }


def load_record(path: Path) -> dict:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise GoldError(f"cannot read gold page {path}: {error}") from error
    if not isinstance(value, dict):
        raise GoldError("gold page must be a JSON object")
    return value


def _valid_sha256(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def _valid_bbox(value: object, width: int, height: int) -> bool:
    if not isinstance(value, list) or len(value) != 4:
        return False
    if any(
        isinstance(item, bool) or not isinstance(item, (int, float)) for item in value
    ):
        return False
    left, top, right, bottom = map(float, value)
    return (
        all(math.isfinite(item) for item in (left, top, right, bottom))
        and 0 <= left < right <= width
        and 0 <= top < bottom <= height
    )


def _valid_optional_id(value: object) -> bool:
    return value is None or (isinstance(value, str) and bool(value.strip()))


def _valid_hierarchy_level(value: object) -> bool:
    return value is None or (
        not isinstance(value, bool) and isinstance(value, int) and 1 <= value <= 32
    )


def _validate_canonical_reference(reference: object) -> None:
    if reference is None:
        return
    if (
        not isinstance(reference, dict)
        or set(reference) != {"kind", "system", "label", "anchor_line_id"}
        or reference.get("kind") not in REFERENCE_KINDS
        or not isinstance(reference.get("system"), str)
        or not reference["system"].strip()
        or not isinstance(reference.get("label"), str)
        or not reference["label"].strip()
        or not _valid_optional_id(reference.get("anchor_line_id"))
    ):
        raise GoldError("canonical reference metadata is malformed")


def _validate_note_exceptions(
    value: object, *, field_name: str, id_field: str
) -> dict[str, str]:
    """Return explicitly justified note exceptions, rejecting vague overrides."""
    if value is None:
        return {}
    if not isinstance(value, list):
        raise GoldError(f"{field_name} must be a list")
    exceptions: dict[str, str] = {}
    for index, exception in enumerate(value):
        if (
            not isinstance(exception, dict)
            or set(exception) != {id_field, "reason"}
            or not isinstance(exception.get(id_field), str)
            or not exception[id_field].strip()
            or len(exception[id_field]) > 256
            or not isinstance(exception.get("reason"), str)
            or not exception["reason"].strip()
            or len(exception["reason"]) > 1024
        ):
            raise GoldError(f"{field_name} entry {index} is malformed")
        identifier = exception[id_field].strip()
        if identifier in exceptions:
            raise GoldError(f"duplicate {field_name} entry for {identifier}")
        exceptions[identifier] = exception["reason"].strip()
    return exceptions


def _validate_inline_spans(
    line: Mapping[str, object], width: int, height: int, *, check_bbox: bool = True
) -> None:
    spans = line.get("inline_spans")
    text = line.get("text")
    if not isinstance(spans, list) or not isinstance(text, str):
        raise GoldError("inline spans are malformed")
    ordered = sorted(
        spans,
        key=lambda span: (
            span.get("start_char", -1) if isinstance(span, dict) else -1,
            span.get("end_char", -1) if isinstance(span, dict) else -1,
            span.get("span_id", "") if isinstance(span, dict) else "",
        ),
    )
    if spans != ordered:
        raise GoldError("inline spans must be in character order")
    seen_ids: set[str] = set()
    previous_end = 0
    fields = {
        "span_id",
        "start_char",
        "end_char",
        "text",
        "styles",
        "semantic_role",
        "target_id",
        "bbox",
    }
    for span in spans:
        if not isinstance(span, dict) or set(span) != fields:
            raise GoldError("inline span has missing or unknown fields")
        start, end = span["start_char"], span["end_char"]
        styles = span["styles"]
        role = span["semantic_role"]
        span_id = span["span_id"]
        if (
            not isinstance(span_id, str)
            or not span_id
            or span_id in seen_ids
            or isinstance(start, bool)
            or not isinstance(start, int)
            or isinstance(end, bool)
            or not isinstance(end, int)
            or not 0 <= start < end <= len(text)
            or start < previous_end
            or span["text"] != text[start:end]
            or unicodedata.normalize("NFC", span["text"]) != span["text"]
            or not isinstance(styles, list)
            or len(styles) != len(set(styles))
            or any(style not in INLINE_STYLES for style in styles)
            or role not in INLINE_SEMANTIC_ROLES
            or (not styles and role == "none")
            or not _valid_optional_id(span["target_id"])
            or (role == "footnote_marker" and not span["target_id"])
            or (
                span["bbox"] is not None
                and check_bbox
                and not _valid_bbox(span["bbox"], width, height)
            )
        ):
            raise GoldError(f"inline span {span_id or '<empty>'} is invalid")
        seen_ids.add(span_id)
        previous_end = end


def validate_record(
    record: Mapping[str, object],
    *,
    require_complete: bool = True,
    image_path: Path | None = None,
) -> None:
    version = record.get("schema_version")
    provenance_field = (
        "candidate_provenance" if version == CGPG_SCHEMA_VERSION else "annotation_seed"
    )
    required = {
        "schema",
        "schema_version",
        "page_id",
        "image",
        "coverage",
        provenance_field,
        "lines",
    }
    if set(record) != required:
        raise GoldError("page record has missing or unknown top-level fields")
    if record["schema"] != SCHEMA or version not in SUPPORTED_SCHEMA_VERSIONS:
        raise GoldError("unsupported closed-world gold schema")
    if not isinstance(record["page_id"], str) or not record["page_id"]:
        raise GoldError("page_id is required")
    image = record["image"]
    if not isinstance(image, dict) or set(image) != {
        "sha256",
        "width",
        "height",
        "mime_type",
    }:
        raise GoldError("image identity is malformed")
    width, height = image.get("width"), image.get("height")
    if (
        isinstance(width, bool)
        or not isinstance(width, int)
        or isinstance(height, bool)
        or not isinstance(height, int)
        or width <= 0
        or height <= 0
        or not _valid_sha256(image.get("sha256"))
        or image.get("mime_type") not in {"image/jpeg", "image/png", "image/tiff"}
    ):
        raise GoldError("image dimensions, digest, or type are invalid")
    if image_path is not None and sha256_file(image_path) != image["sha256"]:
        raise GoldError("page image digest differs from the annotated image")

    coverage = record["coverage"]
    required_coverage_fields = {
        "status",
        "closed_world",
        "all_visible_lines_exhaustively_reviewed",
        "reviewer",
        "verified_at",
        "unresolved_notes",
    }
    if version == SCHEMA_VERSION:
        required_coverage_fields.update(
            {
                "all_structure_exhaustively_reviewed",
                "all_typography_exhaustively_reviewed",
            }
        )
    optional_coverage_fields = (
        {"footnote_marker_exceptions", "external_footnote_targets"}
        if version == SCHEMA_VERSION
        else set()
    )
    if (
        not isinstance(coverage, dict)
        or not required_coverage_fields.issubset(coverage)
        or set(coverage) - required_coverage_fields - optional_coverage_fields
    ):
        raise GoldError("coverage record is malformed")
    if coverage["closed_world"] is not True:
        raise GoldError("closed_world must be explicitly true")
    if coverage["status"] not in {"draft", "verified"}:
        raise GoldError("coverage status is invalid")
    if not isinstance(coverage["unresolved_notes"], str):
        raise GoldError("unresolved_notes must be text")
    marker_exceptions = _validate_note_exceptions(
        coverage.get("footnote_marker_exceptions")
        if version == SCHEMA_VERSION
        else None,
        field_name="footnote marker exceptions",
        id_field="note_id",
    )
    external_footnote_targets = _validate_note_exceptions(
        coverage.get("external_footnote_targets")
        if version == SCHEMA_VERSION
        else None,
        field_name="external footnote targets",
        id_field="target_id",
    )

    if version == CGPG_SCHEMA_VERSION:
        provenance = record["candidate_provenance"]
        if (
            not isinstance(provenance, dict)
            or set(provenance) != {"kind", "xml_sha256", "warning"}
            or provenance.get("kind") != "cgpg_page_xml_scaffolding_only"
            or not _valid_sha256(provenance.get("xml_sha256"))
            or not isinstance(provenance.get("warning"), str)
            or not provenance["warning"]
        ):
            raise GoldError("CGPG candidate provenance is malformed")
    else:
        seed = record["annotation_seed"]
        if (
            not isinstance(seed, dict)
            or set(seed) != {"kind", "warning"}
            or seed.get("kind") != "blank_human_annotation"
            or not isinstance(seed.get("warning"), str)
            or not seed["warning"]
        ):
            raise GoldError("annotation seed is malformed")

    lines = record["lines"]
    if not isinstance(lines, list) or (require_complete and not lines):
        raise GoldError(
            "a complete closed-world page must contain at least one visible line"
        )
    legacy_line_fields = {
        "line_id",
        "reading_order",
        "bbox",
        "text",
        "language",
        "content_class",
        "geometry_status",
        "transcription_status",
        "source",
    }
    extended_line_fields = legacy_line_fields | {
        "structure_status",
        "typography_status",
        "paragraph_role",
        "leaf_id",
        "column_id",
        "hierarchy_level",
        "note_id",
        "canonical_reference",
        "inline_spans",
    }
    line_fields = extended_line_fields if version == SCHEMA_VERSION else legacy_line_fields
    ids = set()
    references: list[tuple[int, str]] = []
    footnote_targets: list[tuple[int, str, str | None]] = []
    note_ids: set[str] = set()
    note_lines: dict[str, list[tuple[int, Mapping[str, object]]]] = {}
    for order, line in enumerate(lines):
        if not isinstance(line, dict) or set(line) != line_fields:
            raise GoldError(f"line {order} has missing or unknown fields")
        text = line["text"]
        if (
            not isinstance(line["line_id"], str)
            or not line["line_id"]
            or line["line_id"] in ids
            or line["reading_order"] != order
            or not _valid_bbox(line["bbox"], width, height)
            or not isinstance(text, str)
            or unicodedata.normalize("NFC", text) != text
            or line["language"] not in LANGUAGES
            or line["content_class"]
            not in (
                CONTENT_CLASSES if version == SCHEMA_VERSION else LEGACY_CONTENT_CLASSES
            )
            or line["geometry_status"] not in {"candidate_unverified", "human_verified"}
            or line["transcription_status"]
            not in {"candidate_unverified", "human_verified"}
            or line["source"]
            not in (
                {"cgpg_page_xml_candidate", "human_draft", "human"}
                if version == CGPG_SCHEMA_VERSION
                else {"human_draft", "human"}
            )
        ):
            raise GoldError(
                f"line {order} has invalid identity, order, box, text, or labels"
            )
        ids.add(line["line_id"])
        if version == SCHEMA_VERSION:
            if (
                line["structure_status"]
                not in {"candidate_unverified", "human_verified"}
                or line["typography_status"]
                not in {"candidate_unverified", "human_verified"}
                or line["paragraph_role"] not in PARAGRAPH_ROLES
                or not _valid_optional_id(line["leaf_id"])
                or not _valid_optional_id(line["column_id"])
                or not _valid_hierarchy_level(line["hierarchy_level"])
                or not _valid_optional_id(line["note_id"])
            ):
                raise GoldError(f"line {order} has invalid structure or typography status")
            _validate_canonical_reference(line["canonical_reference"])
            _validate_inline_spans(line, width, height)
            if line["note_id"]:
                note_ids.add(line["note_id"])
                note_lines.setdefault(line["note_id"], []).append((order, line))
            reference = line["canonical_reference"]
            if reference and reference["anchor_line_id"]:
                references.append((order, reference["anchor_line_id"]))
            for span in line["inline_spans"]:
                if (
                    require_complete
                    and "superscript" in span["styles"]
                    and span["text"].strip().isdigit()
                    and span["semantic_role"] == "other"
                ):
                    raise GoldError(
                        f"line {order} numeric superscript cannot remain "
                        "semantic_role other"
                    )
                if span["semantic_role"] == "footnote_marker":
                    footnote_targets.append(
                        (order, span["target_id"], line["note_id"])
                    )
        if require_complete and (
            not text.strip()
            or line["geometry_status"] != "human_verified"
            or line["transcription_status"] != "human_verified"
            or (
                version == SCHEMA_VERSION
                and (
                    line["structure_status"] != "human_verified"
                    or line["typography_status"] != "human_verified"
                    or line["paragraph_role"] == "unclassified"
                    or (
                        line["content_class"] == "table_of_contents_entry"
                        and (
                            line["hierarchy_level"] is None
                            or line["canonical_reference"] is None
                            or line["canonical_reference"]["kind"]
                            != "toc_destination"
                        )
                    )
                    or (
                        line["content_class"] == "footnote" and not line["note_id"]
                    )
                    or (
                        line["content_class"]
                        in {"marginal_page_label", "marginal_line_number"}
                        and line["canonical_reference"] is None
                    )
                )
            )
            or line["source"] != "human"
            or line["content_class"] == "unclassified"
        ):
            raise GoldError(f"line {order} is still candidate or unclassified")

    for order, anchor_line_id in references:
        if anchor_line_id not in ids:
            raise GoldError(f"line {order} references an unknown anchor line")
    for order, target_id, source_note_id in footnote_targets:
        if target_id not in note_ids and target_id not in external_footnote_targets:
            raise GoldError(f"line {order} references an unknown note_id")
        if require_complete and source_note_id == target_id:
            raise GoldError(
                f"line {order} footnote definition label cannot self-link "
                "as a footnote_marker"
            )

    if require_complete and version == SCHEMA_VERSION:
        new_note_ids = set()
        for note_id, occurrences in note_lines.items():
            footnote_lines = [
                (order, line)
                for order, line in occurrences
                if line["content_class"] == "footnote"
            ]
            first_footnote_line = (
                min(footnote_lines, key=lambda item: item[0])[1]
                if footnote_lines
                else None
            )
            if first_footnote_line and first_footnote_line["paragraph_role"] == "start":
                new_note_ids.add(note_id)
        body_marker_targets = {
            target_id
            for _order, target_id, source_note_id in footnote_targets
            if source_note_id is None
        }
        actual_external_targets = {
            target_id
            for _order, target_id, _source_note_id in footnote_targets
            if target_id not in note_ids
        }
        unused_external_targets = (
            set(external_footnote_targets) - actual_external_targets
        )
        if unused_external_targets:
            raise GoldError(
                "external footnote target is not used by a marker on this page: "
                f"{', '.join(sorted(unused_external_targets))}"
            )
        exception_ids = set(marker_exceptions)
        invalid_exception_ids = exception_ids - new_note_ids
        if invalid_exception_ids:
            raise GoldError(
                "footnote marker exception does not name a note starting on this "
                f"page: {', '.join(sorted(invalid_exception_ids))}"
            )
        stale_exception_ids = exception_ids & body_marker_targets
        if stale_exception_ids:
            raise GoldError(
                "footnote marker exception is stale because a body marker exists: "
                f"{', '.join(sorted(stale_exception_ids))}"
            )
        missing_marker_ids = new_note_ids - body_marker_targets - exception_ids
        if missing_marker_ids:
            raise GoldError(
                "new footnote has no body footnote_marker or explicit exception: "
                f"{', '.join(sorted(missing_marker_ids))}"
            )

    if require_complete:
        if (
            coverage["status"] != "verified"
            or coverage["all_visible_lines_exhaustively_reviewed"] is not True
            or (
                version == SCHEMA_VERSION
                and (
                    coverage["all_structure_exhaustively_reviewed"] is not True
                    or coverage["all_typography_exhaustively_reviewed"] is not True
                )
            )
            or not isinstance(coverage["reviewer"], str)
            or not coverage["reviewer"].strip()
            or not isinstance(coverage["verified_at"], str)
            or not coverage["verified_at"]
            or coverage["unresolved_notes"].strip()
        ):
            raise GoldError("page coverage is not exhaustively human verified")
        try:
            datetime.fromisoformat(coverage["verified_at"].replace("Z", "+00:00"))
        except ValueError as error:
            raise GoldError("verified_at is not an ISO-8601 timestamp") from error


def mark_verified(record: dict, reviewer: str) -> None:
    validate_record(record, require_complete=False)
    if not reviewer.strip():
        raise GoldError("reviewer is required")
    coverage = record["coverage"]
    if coverage["all_visible_lines_exhaustively_reviewed"] is not True:
        raise GoldError("visible-line coverage is not exhaustively reviewed")
    if record.get("schema_version") == SCHEMA_VERSION:
        if coverage["all_structure_exhaustively_reviewed"] is not True:
            raise GoldError("page structure coverage is not exhaustively reviewed")
        if coverage["all_typography_exhaustively_reviewed"] is not True:
            raise GoldError("page typography coverage is not exhaustively reviewed")
    coverage["reviewer"] = reviewer.strip()
    coverage["status"] = "verified"
    coverage["closed_world"] = True
    coverage["verified_at"] = (
        datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    )
    validate_record(record, require_complete=True)


def freeze_manifest(
    page_paths: Iterable[Path], expected_page_ids: Iterable[str]
) -> dict:
    expected = tuple(expected_page_ids)
    records = []
    seen = set()
    for path in sorted(map(Path, page_paths)):
        record = load_record(path)
        validate_record(record, require_complete=True)
        page_id = record["page_id"]
        if page_id in seen:
            raise GoldError(f"duplicate page_id {page_id}")
        seen.add(page_id)
        records.append(
            {
                "page_id": page_id,
                "record_sha256": sha256_file(path),
                "image_sha256": record["image"]["sha256"],
                "line_count": len(record["lines"]),
            }
        )
    if seen != set(expected):
        raise GoldError(
            f"manifest coverage differs: missing={sorted(set(expected) - seen)}, "
            f"unexpected={sorted(seen - set(expected))}"
        )
    records.sort(key=lambda item: expected.index(item["page_id"]))
    return {
        "schema": MANIFEST_SCHEMA,
        "schema_version": MANIFEST_VERSION,
        "coverage": "closed_world_complete",
        "expected_page_ids": list(expected),
        "pages": records,
    }


def _main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pages", nargs="+", type=Path)
    parser.add_argument("--allow-draft", action="store_true")
    args = parser.parse_args()
    try:
        for path in args.pages:
            validate_record(load_record(path), require_complete=not args.allow_draft)
    except GoldError as error:
        parser.error(str(error))
    print(f"valid: {len(args.pages)} page(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
