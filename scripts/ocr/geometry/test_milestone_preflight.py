import copy
import json

import pytest

import milestone_preflight as preflight


def record():
    return {
        "page_id": "dev",
        "image": {"width": 100, "height": 100},
        "coverage": {"closed_world": True,
                     "all_visible_lines_exhaustively_reviewed": True,
                     "reviewer": "synthetic-test", "unresolved_notes": ""},
        "lines": [{"line_id": "line-1", "reading_order": 0,
                   "bbox": [0, 0, 90, 20], "text": "λόγος Test ¹",
                   "geometry_status": "human_verified",
                   "transcription_status": "human_verified"}],
    }


def environment(tmp_path):
    pages, images = tmp_path / "pages", tmp_path / "images"
    pages.mkdir()
    images.mkdir()
    # Correctly hashed but deliberately unparseable: holdout text must not
    # enter this diagnostic tool even when its bytes are integrity checked.
    (pages / "frozen.json").write_bytes(b"DO NOT PARSE HOLDOUT")
    (images / "frozen.png").write_bytes(b"frozen-image")
    (images / "dev.png").write_bytes(b"different-dev-image")
    dev = record()
    dev["image"]["sha256"] = preflight.digest(images / "dev.png")
    (pages / "dev.json").write_text(json.dumps(dev))
    manifest = {
        "schema": "mpdf-closed-world-ocr-gold-manifest", "schema_version": "1.0",
        "coverage": "closed_world_complete", "expected_page_ids": ["frozen"],
        "pages": [{"page_id": "frozen", "record_sha256": preflight.digest(pages / "frozen.json"),
                   "image_sha256": preflight.digest(images / "frozen.png"), "line_count": 1}],
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    selection = {"sources": [{"source_id": "source", "sha256": "a" * 64}],
                 "pages": [{"page_id": page_id, "source_id": "source", "pdf_page": number,
                            "image_sha256": preflight.digest(images / (page_id + ".png"))}
                           for number, page_id in enumerate(("frozen", "dev"), 1)]}
    selection_path = tmp_path / "selection.json"
    selection_path.write_text(json.dumps(selection))
    return manifest_path, pages, images, pages, images, [selection_path]


def test_d_semantic_fields_are_not_required_for_ocr_reference():
    assert preflight.reference_gaps(record()) == []


def test_partial_review_cannot_establish_exhaustive_reference():
    draft = record()
    draft["coverage"]["all_visible_lines_exhaustively_reviewed"] = False
    second = copy.deepcopy(draft["lines"][0])
    second.update(line_id="line-2", reading_order=1, transcription_status="candidate_unverified")
    draft["lines"].append(second)
    assert set(preflight.reference_gaps(draft)) == {"visible_line_coverage_unreviewed", "transcription_unreviewed"}


def test_holdout_is_hashed_but_not_parsed(tmp_path):
    report = preflight.inspect(*environment(tmp_path))
    assert report["excluded"] == {"frozen_page_id": 1}
    assert report["reference_ready_pages"] == 1
    assert report["frozen_evaluation_executed"] is False


def test_holdout_tampering_stops_before_diagnostics(tmp_path):
    args = environment(tmp_path)
    (args[1] / "frozen.json").write_bytes(b"tampered")
    with pytest.raises(ValueError, match="frozen_integrity_failure"):
        preflight.inspect(*args)


def test_renamed_image_clone_is_excluded(tmp_path):
    args = environment(tmp_path)
    (args[2] / "dev.png").write_bytes((args[2] / "frozen.png").read_bytes())
    report = preflight.inspect(*args)
    assert report["excluded"]["frozen_image_digest"] == 1
    assert report["reference_ready_pages"] == 0


def test_rerendered_same_source_page_is_excluded(tmp_path):
    args = environment(tmp_path)
    selection = json.loads(args[5][0].read_text())
    selection["pages"][1]["pdf_page"] = 1
    args[5][0].write_text(json.dumps(selection))
    report = preflight.inspect(*args)
    assert report["excluded"]["frozen_source_page"] == 1
    assert report["reference_ready_pages"] == 0


def test_missing_provenance_is_not_reference_ready(tmp_path):
    args = environment(tmp_path)
    selection = json.loads(args[5][0].read_text())
    selection["pages"].pop()
    args[5][0].write_text(json.dumps(selection))
    report = preflight.inspect(*args)
    assert report["records"][0]["reference_gaps"] == ["no_source_page_binding"]


@pytest.mark.parametrize("key,value,gap", [
    ("bbox", [0, 0, float("nan"), 20], "invalid_reference_box"),
    ("reading_order", True, "invalid_reference_order"),
    ("text", "α\u0301", "invalid_reference_text"),
])
def test_invalid_reference_evidence_cannot_be_ready(key, value, gap):
    dev = record()
    dev["lines"][0][key] = value
    assert gap in preflight.reference_gaps(dev)
