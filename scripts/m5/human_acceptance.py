#!/usr/bin/env python3
"""Prepare and validate the M5 single-annotator acceptance package."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_DIR = REPO_ROOT / "test-data/benchmark/m5-human-gold-v1"
MANIFEST_PATH = CORPUS_DIR / "manifest.json"
DOCUMENT_COLUMNS = [
    "document_id",
    "annotator",
    "review_status",
    "expected_behavior",
    "toc_physical_page_indices",
    "printed_label_notes",
    "document_notes",
]
BOOKMARK_COLUMNS = [
    "bookmark_id",
    "document_id",
    "decision",
    "title",
    "level",
    "parent_bookmark_id",
    "target_physical_page_index",
    "target_y_fraction_from_top",
    "printed_page_label",
    "evidence_kind",
    "evidence_physical_page_index",
    "evidence_bbox_x_fraction",
    "evidence_bbox_y_fraction",
    "evidence_bbox_width_fraction",
    "evidence_bbox_height_fraction",
    "confidence",
    "notes",
]
READER_COLUMNS = [
    "reader",
    "reader_version",
    "platform",
    "open_pdf",
    "page_count_4",
    "visible_rendering_unchanged",
    "search_polytonic_greek",
    "search_unicode_greek",
    "outline_titles",
    "outline_hierarchy",
    "child_target_page_2",
    "appendix_target_page_4",
    "rotated_destinations",
    "overall_status",
    "evidence_notes",
]
READER_CHECKS = READER_COLUMNS[3:13]
EXPECTED_READERS = {"Adobe Acrobat", "Preview", "iOS PDF reader"}
EVIDENCE_CLASSES = {"digital_toc", "scanned_toc", "safe_refusal"}
BEHAVIORS = {"bookmarks_required", "needs_review", "safe_refusal"}
EVIDENCE_KINDS = {
    "digital_toc",
    "scanned_toc",
    "heading_region",
    "typography",
    "numbering",
    "other",
}


class AcceptanceError(Exception):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_manifest(path: Path = MANIFEST_PATH) -> dict:
    if path.is_symlink() or not path.is_file():
        raise AcceptanceError("missing or unsafe human-gold manifest")
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != "mpdf-m5-human-gold-corpus" or data.get(
        "schema_version"
    ) != "1.0":
        raise AcceptanceError("unsupported human-gold manifest")
    documents = data.get("documents")
    if not isinstance(documents, list) or not documents:
        raise AcceptanceError("human-gold manifest has no documents")
    if any(not isinstance(item, dict) for item in documents):
        raise AcceptanceError("manifest document entry is invalid")
    ids = [item.get("id") for item in documents]
    if any(not isinstance(item, str) or not item for item in ids):
        raise AcceptanceError("manifest document ID is invalid")
    if len(ids) != len(set(ids)):
        raise AcceptanceError("manifest document IDs are not unique")
    classes = [item.get("evidence_class") for item in documents]
    if any(item not in EVIDENCE_CLASSES for item in classes):
        raise AcceptanceError("manifest evidence class is invalid")
    for item in documents:
        path = item.get("path")
        digest = item.get("sha256")
        if not isinstance(item.get("page_count"), int) or item["page_count"] < 1:
            raise AcceptanceError("manifest page count is invalid")
        if not isinstance(path, str) or not path:
            raise AcceptanceError("manifest source path is invalid")
        if (
            not isinstance(digest, str)
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
        ):
            raise AcceptanceError("manifest source digest is invalid")
        hints = item.get("toc_hint_page_indices")
        if not isinstance(hints, list) or any(
            not isinstance(page, int) or not 0 <= page < item["page_count"]
            for page in hints
        ):
            raise AcceptanceError("manifest TOC hint is invalid")
    return data


def safe_source(corpus_root: Path, relative: str) -> Path:
    relative_path = Path(relative)
    if relative_path.is_absolute() or ".." in relative_path.parts:
        raise AcceptanceError(f"unsafe source path: {relative}")
    candidate = corpus_root / relative_path
    if candidate.is_symlink():
        raise AcceptanceError(f"source PDF must not be a symlink: {relative}")
    try:
        resolved = candidate.resolve(strict=True)
    except FileNotFoundError as error:
        raise AcceptanceError(f"source PDF is missing: {relative}") from error
    resolved_root = corpus_root.resolve(strict=True)
    if not resolved.is_relative_to(resolved_root):
        raise AcceptanceError(f"source PDF escapes corpus root: {relative}")
    if not resolved.is_file():
        raise AcceptanceError(f"source PDF is not a file: {relative}")
    return resolved


def qpdf_page_count(path: Path) -> int:
    result = subprocess.run(
        ["qpdf", "--show-npages", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode not in (0, 3):
        raise AcceptanceError(f"qpdf could not inspect {path.name}")
    try:
        return int(result.stdout.strip())
    except ValueError as error:
        raise AcceptanceError(f"qpdf returned an invalid page count for {path.name}") from error


def read_csv(path: Path, expected_columns: list[str]) -> list[dict[str, str]]:
    if path.is_symlink() or not path.is_file():
        raise AcceptanceError(f"missing or unsafe CSV: {path.name}")
    with path.open("r", encoding="utf-8-sig", newline="") as source:
        reader = csv.DictReader(source)
        if reader.fieldnames != expected_columns:
            raise AcceptanceError(f"{path.name} columns do not match the frozen template")
        return [
            {key: (value or "").strip() for key, value in row.items()}
            for row in reader
            if any((value or "").strip() for value in row.values())
        ]


def write_csv(path: Path, columns: list[str], rows: list[dict[str, object]]) -> None:
    with path.open("x", encoding="utf-8", newline="") as destination:
        writer = csv.DictWriter(destination, fieldnames=columns)
        writer.writeheader()
        writer.writerows(rows)


def prepare(
    corpus_root: Path,
    output: Path,
    annotator: str,
    include_reader_fixture: bool = True,
) -> None:
    manifest = load_manifest()
    if output.exists() or output.is_symlink():
        raise AcceptanceError(f"refusing to overwrite output path: {output}")
    if not annotator.strip():
        raise AcceptanceError("annotator must not be empty")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)
    )
    try:
        locations = []
        for document in manifest["documents"]:
            source = safe_source(corpus_root, document["path"])
            if sha256_file(source) != document["sha256"]:
                raise AcceptanceError(f"source SHA-256 mismatch: {document['id']}")
            if qpdf_page_count(source) != document["page_count"]:
                raise AcceptanceError(f"source page count mismatch: {document['id']}")
            locations.append(
                {
                    "document_id": document["id"],
                    "evidence_class": document["evidence_class"],
                    "page_count": document["page_count"],
                    "sha256": document["sha256"],
                    "absolute_source_path": str(source),
                }
            )

        shutil.copy2(MANIFEST_PATH, staging / "manifest.json")
        shutil.copy2(CORPUS_DIR / "README.zh-CN.md", staging / "INSTRUCTIONS.zh-CN.md")
        with (CORPUS_DIR / "document-decisions.template.csv").open(
            "r", encoding="utf-8", newline=""
        ) as source:
            rows = list(csv.DictReader(source))
        for row in rows:
            row["annotator"] = annotator.strip()
        write_csv(staging / "document-decisions.csv", DOCUMENT_COLUMNS, rows)
        shutil.copy2(CORPUS_DIR / "bookmarks.template.csv", staging / "bookmarks.csv")
        shutil.copy2(
            CORPUS_DIR / "reader-results.template.csv", staging / "reader-results.csv"
        )
        write_csv(
            staging / "source-files.csv",
            [
                "document_id",
                "evidence_class",
                "page_count",
                "sha256",
                "absolute_source_path",
            ],
            locations,
        )

        if include_reader_fixture:
            if not os.environ.get("MPDF_PDFIUM_LIBRARY"):
                raise AcceptanceError(
                    "MPDF_PDFIUM_LIBRARY is required to build the reader fixture"
                )
            subprocess.run(
                [
                    str(REPO_ROOT / "scripts/m5/check_reader_matrix.sh"),
                    str(staging.resolve() / "reader-fixture"),
                ],
                cwd=REPO_ROOT,
                check=True,
            )
        staging.rename(output)
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def parse_int(value: str, label: str, errors: list[str]) -> int | None:
    try:
        return int(value)
    except ValueError:
        errors.append(f"{label} must be an integer")
        return None


def parse_fraction(value: str, label: str, errors: list[str]) -> float | None:
    try:
        parsed = float(value)
    except ValueError:
        errors.append(f"{label} must be a number between 0 and 1")
        return None
    if not math.isfinite(parsed) or not 0.0 <= parsed <= 1.0:
        errors.append(f"{label} must be between 0 and 1")
        return None
    return parsed


def validate(pack: Path) -> tuple[dict, list[str]]:
    errors: list[str] = []
    if pack.is_symlink() or not pack.is_dir():
        report = {"schema": "mpdf-m5-human-acceptance-validation", "status": "invalid"}
        return report, ["acceptance pack must be a real directory, not a symlink"]
    try:
        manifest = load_manifest(pack / "manifest.json")
        decision_rows = read_csv(pack / "document-decisions.csv", DOCUMENT_COLUMNS)
        bookmark_rows = read_csv(pack / "bookmarks.csv", BOOKMARK_COLUMNS)
        reader_rows = read_csv(pack / "reader-results.csv", READER_COLUMNS)
    except (AcceptanceError, OSError, json.JSONDecodeError) as error:
        report = {"schema": "mpdf-m5-human-acceptance-validation", "status": "invalid"}
        return report, [str(error)]

    documents = {item["id"]: item for item in manifest["documents"]}
    decisions: dict[str, dict[str, str]] = {}
    for row in decision_rows:
        document_id = row["document_id"]
        if document_id in decisions:
            errors.append(f"duplicate document decision: {document_id}")
            continue
        if document_id not in documents:
            errors.append(f"unknown document decision: {document_id}")
            continue
        decisions[document_id] = row
        if not row["annotator"]:
            errors.append(f"{document_id}: annotator is empty")
        if row["review_status"] != "complete":
            errors.append(f"{document_id}: review_status is not complete")
        if row["expected_behavior"] not in BEHAVIORS:
            errors.append(f"{document_id}: expected_behavior is invalid")
        for raw in filter(None, row["toc_physical_page_indices"].split(";")):
            page = parse_int(raw, f"{document_id}: TOC page", errors)
            if page is not None and not 0 <= page < documents[document_id]["page_count"]:
                errors.append(f"{document_id}: TOC page is out of range")
    missing_decisions = set(documents) - set(decisions)
    for document_id in sorted(missing_decisions):
        errors.append(f"missing document decision: {document_id}")
    annotators = {row["annotator"] for row in decisions.values() if row["annotator"]}
    if len(annotators) > 1:
        errors.append("document decisions must use one authoritative annotator")

    bookmarks: dict[str, dict[str, object]] = {}
    included_by_document = {document_id: 0 for document_id in documents}
    excluded_count = 0
    for row_number, row in enumerate(bookmark_rows, start=2):
        prefix = f"bookmarks.csv row {row_number}"
        bookmark_id = row["bookmark_id"]
        document_id = row["document_id"]
        if not bookmark_id:
            errors.append(f"{prefix}: bookmark_id is empty")
            continue
        if bookmark_id in bookmarks:
            errors.append(f"{prefix}: duplicate bookmark_id {bookmark_id}")
            continue
        if document_id not in documents:
            errors.append(f"{prefix}: unknown document_id {document_id}")
            continue
        if row["decision"] not in {"include", "exclude"}:
            errors.append(f"{prefix}: decision must be include or exclude")
            continue
        if not row["title"]:
            errors.append(f"{prefix}: title is empty")
        level = parse_int(row["level"], f"{prefix}: level", errors)
        if level is not None and not 0 <= level <= 64:
            errors.append(f"{prefix}: level is out of range")
        if row["confidence"] not in {"high", "medium", "low"}:
            errors.append(f"{prefix}: confidence is invalid")

        target_page = None
        evidence_page = None
        if row["decision"] == "include":
            included_by_document[document_id] += 1
            target_page = parse_int(
                row["target_physical_page_index"], f"{prefix}: target page", errors
            )
            if target_page is not None and not 0 <= target_page < documents[
                document_id
            ]["page_count"]:
                errors.append(f"{prefix}: target page is out of range")
            if row["target_y_fraction_from_top"]:
                parse_fraction(
                    row["target_y_fraction_from_top"], f"{prefix}: target y", errors
                )
            else:
                errors.append(f"{prefix}: target y is required for an included bookmark")
            if row["evidence_kind"] not in EVIDENCE_KINDS:
                errors.append(f"{prefix}: evidence_kind is invalid")
            evidence_page = parse_int(
                row["evidence_physical_page_index"],
                f"{prefix}: evidence page",
                errors,
            )
            if evidence_page is not None and not 0 <= evidence_page < documents[
                document_id
            ]["page_count"]:
                errors.append(f"{prefix}: evidence page is out of range")
            bbox_keys = [
                "evidence_bbox_x_fraction",
                "evidence_bbox_y_fraction",
                "evidence_bbox_width_fraction",
                "evidence_bbox_height_fraction",
            ]
            if not all(row[key] for key in bbox_keys):
                errors.append(f"{prefix}: complete evidence bbox is required")
            else:
                bbox = [
                    parse_fraction(row[key], f"{prefix}: {key}", errors)
                    for key in bbox_keys
                ]
                if all(value is not None for value in bbox):
                    x, y, width, height = bbox
                    if width == 0 or height == 0 or x + width > 1 or y + height > 1:
                        errors.append(f"{prefix}: evidence bbox is empty or exceeds the page")
        else:
            excluded_count += 1
            if not row["notes"]:
                errors.append(f"{prefix}: excluded bookmark requires notes")

        bookmarks[bookmark_id] = {
            "document_id": document_id,
            "level": level,
            "parent": row["parent_bookmark_id"],
            "decision": row["decision"],
            "target_page": target_page,
            "evidence_page": evidence_page,
        }

    for bookmark_id, bookmark in bookmarks.items():
        parent_id = bookmark["parent"]
        level = bookmark["level"]
        if not parent_id:
            if level not in (0, None):
                errors.append(f"{bookmark_id}: a non-root bookmark has no parent")
            continue
        parent = bookmarks.get(str(parent_id))
        if parent is None:
            errors.append(f"{bookmark_id}: parent bookmark is missing")
        elif parent["document_id"] != bookmark["document_id"]:
            errors.append(f"{bookmark_id}: parent belongs to another document")
        elif parent["decision"] != "include" or bookmark["decision"] != "include":
            errors.append(f"{bookmark_id}: excluded bookmarks cannot participate in hierarchy")
        elif level is not None and parent["level"] is not None and level != parent["level"] + 1:
            errors.append(f"{bookmark_id}: child level must equal parent level plus one")

    for document_id, decision in decisions.items():
        behavior = decision["expected_behavior"]
        count = included_by_document[document_id]
        if behavior == "bookmarks_required" and count == 0:
            errors.append(f"{document_id}: bookmarks_required has no included bookmarks")
        if behavior == "safe_refusal" and count != 0:
            errors.append(f"{document_id}: safe_refusal has included bookmarks")

    readers: dict[str, dict[str, str]] = {}
    for row in reader_rows:
        reader = row["reader"]
        if reader in readers:
            errors.append(f"duplicate reader result: {reader}")
            continue
        if reader not in EXPECTED_READERS:
            errors.append(f"unknown reader result: {reader}")
            continue
        readers[reader] = row
        if not row["reader_version"]:
            errors.append(f"{reader}: reader_version is empty")
        if not row["platform"]:
            errors.append(f"{reader}: platform is empty")
        if row["overall_status"] != "pass":
            errors.append(f"{reader}: overall_status is not pass")
        for check in READER_CHECKS:
            if row[check] != "pass":
                errors.append(f"{reader}: {check} is not pass")
        if not row["evidence_notes"]:
            errors.append(f"{reader}: evidence_notes is empty")
    for reader in sorted(EXPECTED_READERS - set(readers)):
        errors.append(f"missing reader result: {reader}")

    class_counts = {
        evidence_class: sum(
            1
            for document in documents.values()
            if document["evidence_class"] == evidence_class
        )
        for evidence_class in sorted(EVIDENCE_CLASSES)
    }
    report = {
        "schema": "mpdf-m5-human-acceptance-validation",
        "schema_version": "1.0",
        "status": "pass" if not errors else "incomplete",
        "manifest_sha256": sha256_file(pack / "manifest.json"),
        "documents": len(documents),
        "complete_document_decisions": sum(
            1 for row in decisions.values() if row["review_status"] == "complete"
        ),
        "authoritative_annotator": next(iter(annotators), None)
        if len(annotators) == 1
        else None,
        "evidence_class_counts": class_counts,
        "included_bookmarks": sum(included_by_document.values()),
        "excluded_bookmarks": excluded_count,
        "safe_refusal_documents": sum(
            1
            for row in decisions.values()
            if row["expected_behavior"] == "safe_refusal"
        ),
        "readers_passed": sum(
            1 for row in readers.values() if row["overall_status"] == "pass"
        ),
        "annotation_sha256": {
            "document_decisions_csv": sha256_file(pack / "document-decisions.csv"),
            "bookmarks_csv": sha256_file(pack / "bookmarks.csv"),
            "reader_results_csv": sha256_file(pack / "reader-results.csv"),
        },
        "errors": errors,
    }
    return report, errors


def write_report(pack: Path, report: dict) -> None:
    destination = pack / "validation-report.json"
    if destination.is_symlink():
        raise AcceptanceError("validation report target must not be a symlink")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".validation-report-", suffix=".tmp", dir=pack
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(report, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(destination)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser("prepare")
    prepare_parser.add_argument("corpus_root", type=Path)
    prepare_parser.add_argument("output", type=Path)
    prepare_parser.add_argument("--annotator", default="pei-haoran")
    prepare_parser.add_argument("--without-reader-fixture", action="store_true")
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("pack", type=Path)
    args = parser.parse_args()

    try:
        if args.command == "prepare":
            prepare(
                args.corpus_root,
                args.output,
                args.annotator,
                include_reader_fixture=not args.without_reader_fixture,
            )
            print(f"prepared M5 human acceptance package: {args.output}")
            return 0
        report, errors = validate(args.pack)
        write_report(args.pack, report)
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 1 if errors else 0
    except (AcceptanceError, OSError, subprocess.CalledProcessError) as error:
        print(f"error: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
