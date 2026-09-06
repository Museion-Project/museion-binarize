from pathlib import Path

import pytest
from PIL import Image

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
        regions=(
            cgpg.Region(
                "r", "text", (0, 0, 100, 200), (cgpg.Line("λόγος", (1, 2, 80, 20)),)
            ),
        ),
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


def test_first_party_image_starts_as_valid_empty_draft(tmp_path: Path):
    image_path = tmp_path / "mixed-page.png"
    Image.new("RGB", (120, 240), "white").save(image_path)
    record = gold.draft_from_image(image_path)
    assert record["schema_version"] == "1.2"
    assert record["coverage"]["all_structure_exhaustively_reviewed"] is False
    assert record["coverage"]["all_typography_exhaustively_reviewed"] is False
    assert record["annotation_seed"]["kind"] == "blank_human_annotation"
    assert record["lines"] == []
    gold.validate_record(record, require_complete=False, image_path=image_path)
    with pytest.raises(gold.GoldError, match="must contain at least one"):
        gold.validate_record(record, require_complete=True, image_path=image_path)


def test_geometry_and_transcription_are_separate_review_modules(tmp_path: Path):
    record = gold.draft_from_cgpg(page(tmp_path))
    line = record["lines"][0]
    line["content_class"] = "main_text"

    gold.mark_line_geometry_verified(line)
    assert line["geometry_status"] == "human_verified"
    assert line["transcription_status"] == "candidate_unverified"
    assert line["source"] == "human_draft"

    gold.mark_line_transcription_verified(line)
    assert line["geometry_status"] == line["transcription_status"] == "human_verified"
    assert line["source"] == "human"

    gold.invalidate_line_geometry(line)
    assert line["geometry_status"] == "candidate_unverified"
    assert line["transcription_status"] == "human_verified"
    assert line["source"] == "human_draft"

    gold.mark_line_geometry_verified(line)
    gold.invalidate_line_geometry(line, invalidate_transcription=True)
    assert line["geometry_status"] == "candidate_unverified"
    assert line["transcription_status"] == "candidate_unverified"
    assert line["source"] == "human_draft"


def test_transcription_module_requires_verified_geometry_and_labels(tmp_path: Path):
    line = gold.draft_from_cgpg(page(tmp_path))["lines"][0]
    with pytest.raises(gold.GoldError, match="geometry"):
        gold.mark_line_transcription_verified(line)
    gold.mark_line_geometry_verified(line)
    with pytest.raises(gold.GoldError, match="content class"):
        gold.mark_line_transcription_verified(line)


def first_party_line_record(tmp_path: Path) -> dict:
    image_path = tmp_path / "mixed-page.png"
    Image.new("RGB", (120, 240), "white").save(image_path)
    record = gold.draft_from_image(image_path)
    record["lines"].append(
        {
            "line_id": "mixed-page-human-0000",
            "reading_order": 0,
            "bbox": [1, 2, 100, 20],
            "text": "texte1",
            "language": "fra",
            "content_class": "main_text",
            "geometry_status": "human_verified",
            "transcription_status": "human_verified",
            "structure_status": "candidate_unverified",
            "typography_status": "candidate_unverified",
            "paragraph_role": "start",
            "leaf_id": "printed-1",
            "column_id": "main",
            "hierarchy_level": None,
            "note_id": None,
            "canonical_reference": None,
            "inline_spans": [],
            "source": "human_draft",
        }
    )
    return record


def first_party_note_record(
    tmp_path: Path,
    *,
    body_marker: bool,
    note_role: str = "start",
    self_link_definition: bool = False,
) -> dict:
    record = first_party_line_record(tmp_path)
    body = record["lines"][0]
    if body_marker:
        body["inline_spans"] = [
            {
                "span_id": "body-marker-1",
                "start_char": 5,
                "end_char": 6,
                "text": "1",
                "styles": ["superscript"],
                "semantic_role": "footnote_marker",
                "target_id": "note-1",
                "bbox": None,
            }
        ]
    definition_role = "footnote_marker" if self_link_definition else "none"
    record["lines"].append(
        {
            "line_id": "mixed-page-human-0001",
            "reading_order": 1,
            "bbox": [1, 30, 100, 48],
            "text": "1 Note text",
            "language": "eng",
            "content_class": "footnote",
            "geometry_status": "candidate_unverified",
            "transcription_status": "candidate_unverified",
            "structure_status": "candidate_unverified",
            "typography_status": "candidate_unverified",
            "paragraph_role": note_role,
            "leaf_id": "printed-1",
            "column_id": "notes",
            "hierarchy_level": None,
            "note_id": "note-1",
            "canonical_reference": None,
            "inline_spans": [
                {
                    "span_id": "definition-number-1",
                    "start_char": 0,
                    "end_char": 1,
                    "text": "1",
                    "styles": ["superscript"],
                    "semantic_role": definition_role,
                    "target_id": "note-1" if self_link_definition else None,
                    "bbox": None,
                }
            ],
            "source": "human_draft",
        }
    )
    for line in record["lines"]:
        gold.mark_line_geometry_verified(line)
        gold.mark_line_transcription_verified(line)
    coverage = record["coverage"]
    coverage["all_visible_lines_exhaustively_reviewed"] = True
    coverage["all_structure_exhaustively_reviewed"] = True
    coverage["all_typography_exhaustively_reviewed"] = True
    return record


def test_v1_2_modules_require_structure_and_typography_review(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    gold.mark_line_geometry_verified(line)
    assert line["structure_status"] == "human_verified"
    assert line["source"] == "human_draft"
    gold.mark_line_transcription_verified(line)
    assert line["typography_status"] == "human_verified"
    assert line["source"] == "human"


def test_v1_2_footnote_marker_must_link_to_note(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    line["inline_spans"] = [
        {
            "span_id": "s1",
            "start_char": 5,
            "end_char": 6,
            "text": "1",
            "styles": ["superscript"],
            "semantic_role": "footnote_marker",
            "target_id": "note-1",
            "bbox": None,
        }
    ]
    with pytest.raises(gold.GoldError, match="unknown note_id"):
        gold.validate_record(record, require_complete=False)


def test_new_footnote_requires_a_body_marker(tmp_path: Path):
    record = first_party_note_record(tmp_path, body_marker=False)
    with pytest.raises(gold.GoldError, match="new footnote has no body"):
        gold.mark_verified(record, "reviewer")


def test_markerless_footnote_requires_an_explicit_reason(tmp_path: Path):
    record = first_party_note_record(tmp_path, body_marker=False)
    record["coverage"]["footnote_marker_exceptions"] = [
        {"note_id": "note-1", "reason": "Unnumbered editorial note in the source"}
    ]
    gold.mark_verified(record, "reviewer")
    gold.validate_record(record, require_complete=True)


def test_cross_page_footnote_continuation_needs_no_repeated_marker(tmp_path: Path):
    record = first_party_note_record(
        tmp_path, body_marker=False, note_role="continuation"
    )
    gold.mark_verified(record, "reviewer")
    gold.validate_record(record, require_complete=True)


def test_footnote_definition_number_cannot_self_link(tmp_path: Path):
    record = first_party_note_record(
        tmp_path, body_marker=True, self_link_definition=True
    )
    with pytest.raises(gold.GoldError, match="definition label cannot self-link"):
        gold.mark_verified(record, "reviewer")


def test_footnote_marker_exception_cannot_be_stale(tmp_path: Path):
    record = first_party_note_record(tmp_path, body_marker=True)
    record["coverage"]["footnote_marker_exceptions"] = [
        {"note_id": "note-1", "reason": "This must not hide an existing marker"}
    ]
    with pytest.raises(gold.GoldError, match="exception is stale"):
        gold.mark_verified(record, "reviewer")


def test_footnote_marker_exception_must_name_a_note_starting_here(tmp_path: Path):
    record = first_party_note_record(tmp_path, body_marker=False)
    record["coverage"]["footnote_marker_exceptions"] = [
        {"note_id": "unknown-note", "reason": "This identifier is not on the page"}
    ]
    with pytest.raises(gold.GoldError, match="does not name a note starting"):
        gold.mark_verified(record, "reviewer")


def test_external_footnote_target_requires_an_explicit_reason(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    line["inline_spans"] = [
        {
            "span_id": "external-marker-2",
            "start_char": 5,
            "end_char": 6,
            "text": "1",
            "styles": ["superscript"],
            "semantic_role": "footnote_marker",
            "target_id": "next-page-note-1",
            "bbox": None,
        }
    ]
    with pytest.raises(gold.GoldError, match="unknown note_id"):
        gold.validate_record(record, require_complete=False)
    record["coverage"]["external_footnote_targets"] = [
        {
            "target_id": "next-page-note-1",
            "reason": "Definition is outside this page image",
        }
    ]
    gold.mark_line_geometry_verified(line)
    gold.mark_line_transcription_verified(line)
    record["coverage"]["all_visible_lines_exhaustively_reviewed"] = True
    record["coverage"]["all_structure_exhaustively_reviewed"] = True
    record["coverage"]["all_typography_exhaustively_reviewed"] = True
    gold.mark_verified(record, "reviewer")
    gold.validate_record(record, require_complete=True)


def test_external_footnote_target_declaration_cannot_be_unused(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    gold.mark_line_geometry_verified(line)
    gold.mark_line_transcription_verified(line)
    record["coverage"]["all_visible_lines_exhaustively_reviewed"] = True
    record["coverage"]["all_structure_exhaustively_reviewed"] = True
    record["coverage"]["all_typography_exhaustively_reviewed"] = True
    record["coverage"]["external_footnote_targets"] = [
        {
            "target_id": "unused-note",
            "reason": "This declaration has no corresponding marker",
        }
    ]
    with pytest.raises(gold.GoldError, match="not used by a marker"):
        gold.mark_verified(record, "reviewer")


def test_numeric_superscript_cannot_remain_semantically_other(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    line["inline_spans"] = [
        {
            "span_id": "unclassified-superscript-1",
            "start_char": 5,
            "end_char": 6,
            "text": "1",
            "styles": ["superscript"],
            "semantic_role": "other",
            "target_id": None,
            "bbox": None,
        }
    ]
    gold.mark_line_geometry_verified(line)
    gold.mark_line_transcription_verified(line)
    record["coverage"]["all_visible_lines_exhaustively_reviewed"] = True
    record["coverage"]["all_structure_exhaustively_reviewed"] = True
    record["coverage"]["all_typography_exhaustively_reviewed"] = True
    with pytest.raises(gold.GoldError, match="cannot remain semantic_role other"):
        gold.mark_verified(record, "reviewer")


def test_contents_entry_requires_hierarchy_and_destination(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    line["content_class"] = "table_of_contents_entry"
    with pytest.raises(gold.GoldError, match="contents entries"):
        gold.mark_line_geometry_verified(line)
    line["hierarchy_level"] = 2
    line["canonical_reference"] = {
        "kind": "toc_destination",
        "system": "printed_page",
        "label": "30",
        "anchor_line_id": None,
    }
    gold.mark_line_geometry_verified(line)
    assert line["structure_status"] == "human_verified"


def test_v1_1_upgrade_reopens_new_review_gates(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    for field in (
        "structure_status",
        "typography_status",
        "paragraph_role",
        "leaf_id",
        "column_id",
        "hierarchy_level",
        "note_id",
        "canonical_reference",
        "inline_spans",
    ):
        del line[field]
    record["schema_version"] = "1.1"
    del record["coverage"]["all_structure_exhaustively_reviewed"]
    del record["coverage"]["all_typography_exhaustively_reviewed"]
    line["source"] = "human"
    gold.upgrade_record_to_v1_2(record)
    assert record["schema_version"] == "1.2"
    assert line["structure_status"] == "candidate_unverified"
    assert line["typography_status"] == "candidate_unverified"
    assert line["source"] == "human_draft"
    gold.validate_record(record, require_complete=False)


def test_page_verification_requires_explicit_exhaustive_gates(tmp_path: Path):
    record = first_party_line_record(tmp_path)
    line = record["lines"][0]
    gold.mark_line_geometry_verified(line)
    gold.mark_line_transcription_verified(line)
    with pytest.raises(gold.GoldError, match="visible-line"):
        gold.mark_verified(record, "reviewer")
    record["coverage"]["all_visible_lines_exhaustively_reviewed"] = True
    with pytest.raises(gold.GoldError, match="structure coverage"):
        gold.mark_verified(record, "reviewer")
    record["coverage"]["all_structure_exhaustively_reviewed"] = True
    with pytest.raises(gold.GoldError, match="typography coverage"):
        gold.mark_verified(record, "reviewer")


def test_verified_closed_world_page_passes(tmp_path: Path):
    gold.validate_record(verified_record(tmp_path), require_complete=True)


@pytest.mark.parametrize(
    "mutation", ["missing_line", "bad_order", "not_nfc", "unresolved"]
)
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
