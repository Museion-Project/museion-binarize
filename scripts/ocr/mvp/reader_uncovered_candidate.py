"""Second isolated merge candidate: revisit uncovered Greek blocked by padding.

Only an earlier partial-overlap decision is eligible. Existing v1 words are
never retired or changed. All recovered observations remain review drafts.
"""
import copy
import math
import unicodedata

import cv2

from . import core, reader_merge_candidate as v1

CANDIDATE_VERSION = 'pixel-supported-uncovered-Greek-adoption-v2'


def compose(apple, reader, residual, image):
    words, decisions = v1.compose(apple, reader, residual, image)
    words, decisions = copy.deepcopy(words), copy.deepcopy(decisions)
    gray = cv2.imread(str(image), cv2.IMREAD_GRAYSCALE)
    if gray is None:
        raise ValueError('INVALID_PAGE_IMAGE')
    ink = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
    _, labels, stats, _ = cv2.connectedComponentsWithStats(ink, connectivity=8)
    history = {d['reader_id']: d for d in decisions
               if d.get('reader_id') and d.get('reason') == 'partial_overlap'}
    selected = {w['id'] for w in words}
    for t in sorted(reader, key=lambda q: (q['bbox'][1], q['bbox'][0], q['id'])):
        box = t['bbox']
        if t['id'] in selected or t['id'] not in history or t['confidence'] < 80:
            continue
        if core.old.greek(t['text']) < 2 or any(
                c.isalpha() and 'LATIN' in unicodedata.name(c, '') for c in t['text']):
            continue
        if len(box) != 4 or not all(math.isfinite(x) for x in box) or not (
                0 <= box[0] < box[2] <= ink.shape[1] and 0 <= box[1] < box[3] <= ink.shape[0]):
            continue
        if not v1._ink_count(ink, box, box):
            continue
        # Two overlapping missing observations cannot establish a unique source.
        if any(q['id'] != t['id'] and q['id'] not in selected and
               core.same_row(box, q['bbox']) and core.old.overlap(box, q['bbox']) > .6
               for q in reader):
            continue
        conflicts = core.reader_support_conflicts(box, words)
        if any(c['overlap_fraction'] >= .45 for c in conflicts):
            continue
        proofs = []
        for c in conflicts:
            blocker = next(w for w in words if w['id'] == c['member_id'])
            proof = v1._padding_proof(blocker, t, reader, ink, (labels, stats))
            if proof is None:
                break
            proofs.append(dict(conflict=c, pixel_proof=proof))
        else:
            active = {w['id'] for w in words}
            retired = [c['member_id'] for c in history[t['id']]['support_conflicts']
                       if c['member_id'] not in active]
            words.append(dict(t, review=True, source_members=[],
                              reason='uncovered_independent_support',
                              candidate_policy=CANDIDATE_VERSION))
            selected.add(t['id'])
            decisions.append(dict(state='UNCOVERED_GREEK_PADDING_DRAFT', reader_id=t['id'],
                                  source_members=[], policy=CANDIDATE_VERSION,
                                  source_pixel_conflicts_resolved=proofs,
                                  prior_conflicting_supports_no_longer_active=retired,
                                  spelling_confirmed=False))
    words = core.nfc_words([w for row in core.reading_rows(words) for w in row])
    core.ownership(words)
    return words, decisions
