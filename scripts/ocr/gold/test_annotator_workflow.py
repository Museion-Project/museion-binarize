from copy import deepcopy

import pytest
from PIL import Image

import closed_world_gold as gold
import annotator_workflow as workflow


@pytest.fixture
def draft(tmp_path):
    image = tmp_path / 'page.png'
    Image.new('RGB', (100, 100), 'white').save(image)
    record = gold.draft_from_image(image)
    record['lines'] = [dict(line_id='line-a', reading_order=0, bbox=[1, 1, 90, 20],
        text='Visible text', language='mixed', content_class='main_text',
        geometry_status='candidate_unverified', transcription_status='candidate_unverified',
        source='human_draft', structure_status='candidate_unverified',
        typography_status='candidate_unverified', paragraph_role='unclassified',
        leaf_id=None, column_id=None, hierarchy_level=None, note_id=None,
        canonical_reference=None, inline_spans=[])]
    record['coverage']['all_visible_lines_exhaustively_reviewed'] = True
    return record, image


def test_full_completion_failure_is_transactional(draft):
    record, image = draft
    record['coverage'].update(all_structure_exhaustively_reviewed=True,
                              all_typography_exhaustively_reviewed=True)
    before = deepcopy(record)
    with pytest.raises(gold.GoldError):
        workflow.complete(record, 'reviewer', 'full-gold', image)
    assert record == before
    assert record['coverage']['status'] == 'draft'


def test_full_line_failure_does_not_half_verify(draft):
    record, image = draft
    record['lines'][0].update(geometry_status='human_verified', structure_status='human_verified',
                             content_class='footnote', paragraph_role='single')
    before = deepcopy(record)
    with pytest.raises(gold.GoldError, match='note_id'):
        workflow.mark_line(record, 0, 'transcription', 'full-gold', image)
    assert record == before


def dev_ready(record, image):
    for stage in ('geometry', 'transcription'):
        record = workflow.mark_line(record, 0, stage, 'ocr-geometry', image)
    return workflow.complete(record, 'reviewer', 'ocr-geometry', image)


def test_dev_receipt_independent_of_deferred_semantics_and_bound_to_digests(draft, tmp_path):
    record, image = draft
    record = dev_ready(record, image)
    assert record['coverage']['status'] == 'draft'
    assert record['lines'][0]['structure_status'] == 'candidate_unverified'
    assert record['lines'][0]['typography_status'] == 'candidate_unverified'
    path = tmp_path / 'pages' / 'page.json'
    saved = workflow.save(path, record, None, image, dev_complete=True)
    assert workflow.receipt_valid(path, record, image)
    changed = deepcopy(record)
    changed['lines'][0]['text'] += '!'
    assert not workflow.receipt_valid(path, changed, image)
    workflow.save(path, changed, saved, image)
    assert not workflow.receipt_valid(path, changed, image)


def test_image_edit_invalidates_receipt(draft, tmp_path):
    record, image = draft
    record = dev_ready(record, image)
    path = tmp_path / 'pages' / 'page.json'
    workflow.save(path, record, None, image, dev_complete=True)
    Image.new('RGB', (100, 100), 'black').save(image)
    assert not workflow.receipt_valid(path, record, image)


@pytest.mark.parametrize('change', ['empty_page', 'empty_text', 'unreviewed', 'coverage', 'notes'])
def test_dev_does_not_accept_incomplete_ocr(draft, change):
    record, image = draft
    record = dev_ready(record, image)
    if change == 'empty_page': record['lines'] = []
    elif change == 'empty_text': record['lines'][0]['text'] = ''
    elif change == 'unreviewed': record['lines'][0]['transcription_status'] = 'candidate_unverified'
    elif change == 'coverage': record['coverage']['all_visible_lines_exhaustively_reviewed'] = False
    else: record['coverage']['unresolved_notes'] = 'missing line'
    with pytest.raises(gold.GoldError):
        workflow.complete(record, 'reviewer', 'ocr-geometry', image)


def test_external_edit_refuses_overwrite(draft, tmp_path):
    record, image = draft
    path = tmp_path / 'pages' / 'page.json'
    saved = workflow.save(path, record, None, image)
    path.write_text('external newer work')
    with pytest.raises(gold.GoldError, match='changed on disk'):
        workflow.save(path, record, saved, image)
    assert path.read_text() == 'external newer work'


def test_dev_uses_existing_draft_validator(draft):
    record, image = draft
    record['lines'][0]['bbox'] = [-1, 1, 90, 20]
    with pytest.raises(gold.GoldError, match='invalid'):
        workflow.mark_line(record, 0, 'geometry', 'ocr-geometry', image)
    assert record['lines'][0]['geometry_status'] == 'candidate_unverified'


def test_actionable_note_and_line_location(draft):
    record, _ = draft
    assert workflow.error_line(record, 'line 0 is still candidate') == 0
    record['lines'][0]['note_id'] = 'note-a'
    assert workflow.error_line(record, 'new footnote has no marker: note-a') == 0
