#!/usr/bin/env python3
"""Tests for the M5 single-authoritative-annotator acceptance package."""

from __future__ import annotations

import csv
import json
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))
import human_acceptance as acceptance  # noqa: E402


class HumanAcceptanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.pack = Path(self.temporary.name) / "pack"
        self.pack.mkdir()
        documents = [
            {
                "id": "digital",
                "evidence_class": "digital_toc",
                "path": "digital.pdf",
                "sha256": "0" * 64,
                "page_count": 5,
                "toc_hint_page_indices": [0],
            },
            {
                "id": "scanned",
                "evidence_class": "scanned_toc",
                "path": "scanned.pdf",
                "sha256": "1" * 64,
                "page_count": 6,
                "toc_hint_page_indices": [1],
            },
            {
                "id": "refusal",
                "evidence_class": "safe_refusal",
                "path": "refusal.pdf",
                "sha256": "2" * 64,
                "page_count": 2,
                "toc_hint_page_indices": [],
            },
        ]
        (self.pack / "manifest.json").write_text(
            json.dumps(
                {
                    "schema": "mpdf-m5-human-gold-corpus",
                    "schema_version": "1.0",
                    "documents": documents,
                }
            ),
            encoding="utf-8",
        )
        self.decisions = [
            self.decision("digital", "bookmarks_required", "0"),
            self.decision("scanned", "needs_review", "1"),
            self.decision("refusal", "safe_refusal", ""),
        ]
        self.bookmarks = [
            self.bookmark("digital-root", "digital", 0, "", 2),
            self.bookmark("digital-child", "digital", 1, "digital-root", 3),
        ]
        self.readers = [self.reader(reader) for reader in sorted(acceptance.EXPECTED_READERS)]
        self.write_tables()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def decision(document_id: str, behavior: str, toc_pages: str) -> dict[str, str]:
        return {
            "document_id": document_id,
            "annotator": "pei-haoran",
            "review_status": "complete",
            "expected_behavior": behavior,
            "toc_physical_page_indices": toc_pages,
            "printed_label_notes": "checked",
            "document_notes": "checked",
        }

    @staticmethod
    def bookmark(
        bookmark_id: str,
        document_id: str,
        level: int,
        parent: str,
        page: int,
    ) -> dict[str, str]:
        return {
            "bookmark_id": bookmark_id,
            "document_id": document_id,
            "decision": "include",
            "title": "Ἀρχὴ Πολιτείας",
            "level": str(level),
            "parent_bookmark_id": parent,
            "target_physical_page_index": str(page),
            "target_y_fraction_from_top": "0.20",
            "printed_page_label": str(page + 1),
            "evidence_kind": "digital_toc",
            "evidence_physical_page_index": "0",
            "evidence_bbox_x_fraction": "0.10",
            "evidence_bbox_y_fraction": "0.20",
            "evidence_bbox_width_fraction": "0.70",
            "evidence_bbox_height_fraction": "0.05",
            "confidence": "high",
            "notes": "verified",
        }

    @staticmethod
    def reader(reader: str) -> dict[str, str]:
        row = {column: "pass" for column in acceptance.READER_CHECKS}
        row.update(
            {
                "reader": reader,
                "reader_version": "test-version",
                "platform": "test-platform",
                "overall_status": "pass",
                "evidence_notes": "manually checked",
            }
        )
        return row

    @staticmethod
    def write_csv(path: Path, columns: list[str], rows: list[dict[str, str]]) -> None:
        with path.open("w", encoding="utf-8", newline="") as destination:
            writer = csv.DictWriter(destination, fieldnames=columns)
            writer.writeheader()
            writer.writerows(rows)

    def write_tables(self) -> None:
        self.write_csv(
            self.pack / "document-decisions.csv",
            acceptance.DOCUMENT_COLUMNS,
            self.decisions,
        )
        self.write_csv(
            self.pack / "bookmarks.csv",
            acceptance.BOOKMARK_COLUMNS,
            self.bookmarks,
        )
        self.write_csv(
            self.pack / "reader-results.csv",
            acceptance.READER_COLUMNS,
            self.readers,
        )

    def test_complete_single_annotator_pack_passes(self) -> None:
        report, errors = acceptance.validate(self.pack)

        self.assertEqual(errors, [])
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["authoritative_annotator"], "pei-haoran")
        self.assertEqual(report["included_bookmarks"], 2)
        self.assertEqual(report["readers_passed"], 3)

    def test_second_annotator_is_rejected(self) -> None:
        self.decisions[1]["annotator"] = "someone-else"
        self.write_tables()

        _, errors = acceptance.validate(self.pack)

        self.assertIn(
            "document decisions must use one authoritative annotator", errors
        )

    def test_safe_refusal_cannot_include_a_bookmark(self) -> None:
        self.bookmarks.append(self.bookmark("unsafe", "refusal", 0, "", 0))
        self.write_tables()

        _, errors = acceptance.validate(self.pack)

        self.assertIn("refusal: safe_refusal has included bookmarks", errors)

    def test_evidence_bbox_cannot_escape_page(self) -> None:
        self.bookmarks[0]["evidence_bbox_x_fraction"] = "0.50"
        self.bookmarks[0]["evidence_bbox_width_fraction"] = "0.75"
        self.write_tables()

        _, errors = acceptance.validate(self.pack)

        self.assertTrue(
            any("evidence bbox is empty or exceeds the page" in error for error in errors)
        )

    def test_incomplete_reader_result_fails_closed(self) -> None:
        self.readers[0]["search_polytonic_greek"] = "not_run"
        self.write_tables()

        _, errors = acceptance.validate(self.pack)

        self.assertTrue(
            any("search_polytonic_greek is not pass" in error for error in errors)
        )

    def test_symlinked_manifest_is_rejected(self) -> None:
        real_manifest = self.pack / "real-manifest.json"
        (self.pack / "manifest.json").replace(real_manifest)
        (self.pack / "manifest.json").symlink_to(real_manifest.name)

        report, errors = acceptance.validate(self.pack)

        self.assertEqual(report["status"], "invalid")
        self.assertEqual(errors, ["missing or unsafe human-gold manifest"])

    def test_malformed_manifest_entry_fails_closed(self) -> None:
        manifest = json.loads((self.pack / "manifest.json").read_text(encoding="utf-8"))
        manifest["documents"][0] = "not-an-object"
        (self.pack / "manifest.json").write_text(
            json.dumps(manifest), encoding="utf-8"
        )

        report, errors = acceptance.validate(self.pack)

        self.assertEqual(report["status"], "invalid")
        self.assertEqual(errors, ["manifest document entry is invalid"])


if __name__ == "__main__":
    unittest.main()
