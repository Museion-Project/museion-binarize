"""Transactional workbench operations; dev QA receipts are never Gold records."""
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import fcntl

import closed_world_gold as gold


def digest(path: Path) -> str | None:
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def mark_line(record, index, stage, scope, image_path):
    candidate = deepcopy(record)
    line = candidate['lines'][index]
    if scope == 'ocr-geometry':
        if stage == 'transcription':
            if line['geometry_status'] != 'human_verified':
                raise gold.GoldError(f'line {index}: verify geometry before transcription')
            if not line['text'].strip():
                raise gold.GoldError(f'line {index}: transcription cannot be empty')
        line[f'{stage}_status'] = 'human_verified'
    else:
        try:
            getattr(gold, f'mark_line_{stage}_verified')(line)
        except gold.GoldError as error:
            raise gold.GoldError(f'line {index}: {error}') from error
    candidate['coverage'].update(status='draft', verified_at=None)
    gold.validate_record(candidate, require_complete=False, image_path=image_path)
    return candidate


def complete(record, reviewer, scope, image_path):
    candidate = deepcopy(record)
    if scope == 'full-gold':
        gold.mark_verified(candidate, reviewer)
        gold.validate_record(candidate, require_complete=True, image_path=image_path)
    else:
        candidate['coverage'].update(status='draft', verified_at=None, reviewer=reviewer.strip())
        gold.validate_record(candidate, require_complete=False, image_path=image_path)
        if not candidate['lines']:
            raise gold.GoldError('page has no visible lines')
        for index, line in enumerate(candidate['lines']):
            if (line['geometry_status'] != 'human_verified'
                    or line['transcription_status'] != 'human_verified' or not line['text'].strip()):
                raise gold.GoldError(f'line {index}: geometry and nonempty transcription must be human verified')
        coverage = candidate['coverage']
        if not reviewer.strip() or not coverage['all_visible_lines_exhaustively_reviewed']:
            raise gold.GoldError('reviewer and exhaustive visible-line coverage are required')
        if coverage['unresolved_notes'].strip():
            raise gold.GoldError('resolve page OCR notes before Dev reference QA completion')
    return candidate


def receipt_path(path):
    return path.parent.parent / 'dev-qa-receipts' / path.name


def receipt_valid(path, record, image_path):
    try:
        receipt = json.loads(receipt_path(path).read_text())
        return (receipt['scope'] == 'ocr-geometry'
                and receipt['record_sha256'] == digest(path)
                and receipt['image_sha256'] == digest(image_path)
                and gold.load_record(path) == record)
    except (OSError, ValueError, KeyError):
        return False


def save(path, record, expected_digest, image_path, *, dev_complete=False):
    """Serialize workbench writers and reject disk edits since load/last save."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.with_suffix(path.suffix + '.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if digest(path) != expected_digest:
            raise gold.GoldError('File changed on disk since loading. Save refused to protect newer edits; keep this window open and reconcile/reload the file.')
        gold.validate_record(record, require_complete=False, image_path=image_path)
        gold.atomic_write_json(path, record)
        new_digest = digest(path)
        if dev_complete:
            gold.atomic_write_json(receipt_path(path), {
                'kind': 'dev_reference_qa_receipt_not_gold', 'scope': 'ocr-geometry',
                'page_id': record['page_id'], 'reviewer': record['coverage']['reviewer'],
                'record_sha256': new_digest, 'image_sha256': digest(image_path),
                'reviewed_at': datetime.now(timezone.utc).isoformat(),
            })
        return new_digest


def error_line(record, message):
    match = re.search(r'\bline (\d+)\b', message)
    if match and int(match[1]) < len(record['lines']):
        return int(match[1])
    for index, line in enumerate(record['lines']):
        if line.get('note_id') and line['note_id'] in message:
            return index
    return None
