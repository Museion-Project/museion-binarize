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
SCHEMA_VERSION = "1.1"
CGPG_SCHEMA_VERSION = "1.0"
SUPPORTED_SCHEMA_VERSIONS = {CGPG_SCHEMA_VERSION, SCHEMA_VERSION}
MANIFEST_SCHEMA = "mpdf-closed-world-ocr-gold-manifest"
MANIFEST_VERSION = "1.0"
LANGUAGES = ("grc", "lat", "deu", "fra", "eng", "mixed", "zxx", "und")
CONTENT_CLASSES = (
    "main_text",
    "apparatus",
    "running_head",
    "page_number",
    "footnote",
    "caption",
    "other",
    "unclassified",
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
    """Creates a blank v1.1 draft for first-party closed-world annotation."""
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
        raise GoldError(f"unsupported annotation image format: {image_format or 'unknown'}")
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
    if any(isinstance(item, bool) or not isinstance(item, (int, float)) for item in value):
        return False
    left, top, right, bottom = map(float, value)
    return (
        all(math.isfinite(item) for item in (left, top, right, bottom))
        and 0 <= left < right <= width
        and 0 <= top < bottom <= height
    )


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
    if not isinstance(image, dict) or set(image) != {"sha256", "width", "height", "mime_type"}:
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
    coverage_fields = {
        "status",
        "closed_world",
        "all_visible_lines_exhaustively_reviewed",
        "reviewer",
        "verified_at",
        "unresolved_notes",
    }
    if not isinstance(coverage, dict) or set(coverage) != coverage_fields:
        raise GoldError("coverage record is malformed")
    if coverage["closed_world"] is not True:
        raise GoldError("closed_world must be explicitly true")
    if coverage["status"] not in {"draft", "verified"}:
        raise GoldError("coverage status is invalid")
    if not isinstance(coverage["unresolved_notes"], str):
        raise GoldError("unresolved_notes must be text")

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
        raise GoldError("a complete closed-world page must contain at least one visible line")
    line_fields = {
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
    ids = set()
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
            or line["content_class"] not in CONTENT_CLASSES
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
            raise GoldError(f"line {order} has invalid identity, order, box, text, or labels")
        ids.add(line["line_id"])
        if require_complete and (
            not text.strip()
            or line["geometry_status"] != "human_verified"
            or line["transcription_status"] != "human_verified"
            or line["source"] != "human"
            or line["content_class"] == "unclassified"
        ):
            raise GoldError(f"line {order} is still candidate or unclassified")

    if require_complete:
        if (
            coverage["status"] != "verified"
            or coverage["all_visible_lines_exhaustively_reviewed"] is not True
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
    coverage["reviewer"] = reviewer.strip()
    coverage["status"] = "verified"
    coverage["closed_world"] = True
    coverage["all_visible_lines_exhaustively_reviewed"] = True
    coverage["verified_at"] = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    validate_record(record, require_complete=True)


def freeze_manifest(page_paths: Iterable[Path], expected_page_ids: Iterable[str]) -> dict:
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
