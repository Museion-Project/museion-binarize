from pathlib import Path

import pytest

import closed_world_gold as gold
import cgpg


def page(tmp_path: Path) -> cgpg.Page:
    image = tmp_path / "p.jpg"
    image.write_bytes(b"image")
    xml = tmp_path / "p.xml"
    xml.write_bytes(b"xml")
    return cgpg.Page(
        name="p",
        image_path=image,
        xml_path=xml,
        width=100,
        height=200,
        regions=(cgpg.Region("r", "text", (0, 0, 100, 200), (cgpg.Line("λόγος", (1, 2, 80, 20)),)),),
    )


def verified_record(tmp_path: Path) -> dict:
    record = gold.draft_from_cgpg(page(tmp_path))
    line = record["lines"][0]
    line.update(
        content_class="main_text",
        geometry_status="human_verified",
        transcription_status="human_verified",
        source="human",
    )
    record["coverage"]["all_visible_lines_exhaustively_reviewed"] = True
    gold.mark_verified(record, "reviewer")
    return record


def test_candidate_xml_is_never_complete_gold(tmp_path: Path):
    record = gold.draft_from_cgpg(page(tmp_path))
    gold.validate_record(record, require_complete=False)
    with pytest.raises(gold.GoldError, match="still candidate"):
        gold.validate_record(record, require_complete=True)


def test_verified_closed_world_page_passes(tmp_path: Path):
    gold.validate_record(verified_record(tmp_path), require_complete=True)


@pytest.mark.parametrize("mutation", ["missing_line", "bad_order", "not_nfc", "unresolved"])
def test_coverage_failures_are_rejected(tmp_path: Path, mutation: str):
    record = verified_record(tmp_path)
    if mutation == "missing_line":
        record["lines"] = []
    elif mutation == "bad_order":
        record["lines"][0]["reading_order"] = 3
    elif mutation == "not_nfc":
        record["lines"][0]["text"] = "λο\u0301γος"
    else:
        record["coverage"]["unresolved_notes"] = "missing marginal line"
    with pytest.raises(gold.GoldError):
        gold.validate_record(record, require_complete=True)


def test_manifest_requires_exact_closed_world_page_set(tmp_path: Path):
    path = tmp_path / "p.json"
    gold.atomic_write_json(path, verified_record(tmp_path))
    manifest = gold.freeze_manifest([path], ["p"])
    assert manifest["coverage"] == "closed_world_complete"
    with pytest.raises(gold.GoldError, match="coverage differs"):
        gold.freeze_manifest([path], ["p", "missing"])
