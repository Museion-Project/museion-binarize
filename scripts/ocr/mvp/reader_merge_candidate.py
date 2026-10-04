"""Generic same-image merge repairs, isolated from the admitted local route.

The baseline and raw observations stay intact. Every proposed transfer is a
review draft; pixel separation proves geometry, never the spelling of a word.
"""
import copy
import math
import unicodedata

import cv2
import numpy as np

from . import core

CANDIDATE_VERSION = 'reader-merge-mechanisms-v1'


def _body(text):
    return ''.join(c for c in unicodedata.normalize('NFC', text)
                   if not c.isspace() and not unicodedata.category(c).startswith('P'))


def _punctuation(text):
    return [c for c in unicodedata.normalize('NFC', text)
            if unicodedata.category(c).startswith('P')]


def _ink_count(ink, a, b):
    left, top = math.floor(max(a[0], b[0])), math.floor(max(a[1], b[1]))
    right, bottom = math.ceil(min(a[2], b[2])), math.ceil(min(a[3], b[3]))
    h, w = ink.shape
    return int(np.count_nonzero(ink[max(0, top):min(h, bottom),
                                     max(0, left):min(w, right)])) if right > left and bottom > top else 0


def _complete_horizontal(a, b):
    tolerance = max(8, .15 * (a[2] - a[0]))
    return abs(a[0] - b[0]) <= tolerance and abs(a[2] - b[2]) <= tolerance


def _separate_components(a, b, labels, stats):
    left, top = math.floor(max(a[0], b[0])), math.floor(max(a[1], b[1]))
    right, bottom = math.ceil(min(a[2], b[2])), math.ceil(min(a[3], b[3]))
    shared = set(np.unique(labels[max(0, top):min(labels.shape[0], bottom),
                                 max(0, left):min(labels.shape[1], right)])) - {0}
    supports = []
    for label in sorted(shared):
        x, y, width, height, _ = map(int, stats[label])
        box = [x, y, x + width, y + height]
        in_a = a[0] <= x and a[1] <= y and box[2] <= a[2] and box[3] <= a[3]
        in_b = b[0] <= x and b[1] <= y and box[2] <= b[2] and box[3] <= b[3]
        if in_a == in_b:
            return None
        supports.append(dict(bbox=box, owner='anchor' if in_a else 'target'))
    return supports or None


def _padding_proof(blocker, target, reader, ink, components=None):
    box, tb = blocker['bbox'], target['bbox']
    if not _ink_count(ink, box, tb):
        return dict(kind='empty_pixel_intersection', member_id=blocker['id'])
    if components is not None:
        supports = _separate_components(box, tb, *components)
        if supports is not None:
            return dict(kind='exclusive_complete_ink_components', member_id=blocker['id'],
                        component_supports=supports)
    anchors = [q for q in reader if q['confidence'] >= 80 and
               unicodedata.normalize('NFC', q['text']) == unicodedata.normalize('NFC', blocker['text']) and
               core.same_row(box, q['bbox']) and core.old.overlap(box, q['bbox']) >= .45 and
               _complete_horizontal(box, q['bbox'])]
    if len(anchors) != 1:
        return None
    q = anchors[0]
    if q['id'] == target['id'] or core.same_row(q['bbox'], tb):
        return None
    supports = []
    if _ink_count(ink, q['bbox'], tb):
        if components is None:
            _, labels, stats, _ = cv2.connectedComponentsWithStats(ink, connectivity=8)
        else:
            labels, stats = components
        supports = _separate_components(q['bbox'], tb, labels, stats)
        if supports is None:
            return None
    return dict(kind='separate_exact_reader_anchor', member_id=blocker['id'],
                anchor_id=q['id'], anchor_bbox=copy.deepcopy(q['bbox']),
                exclusive_complete_ink_components=supports)


def _blocker_proofs(words, owner, targets, reader, ink, components):
    proofs = []
    for blocker in words:
        if blocker['id'] == owner['id']:
            continue
        for t in targets:
            if core.old.intersection(blocker['bbox'], t['bbox']) <= 0:
                continue
            if core.old.overlap(blocker['bbox'], t['bbox']) > .6:
                return None
            proof = _padding_proof(blocker, t, reader, ink, components)
            if proof is None:
                return None
            proofs.append(dict(proof, target_id=t['id']))
    return proofs


def _observed_space(ink, left, right):
    a, b = left['bbox'], right['bbox']
    gap = b[0] - a[2]
    if gap < max(2, .2 * min(a[3] - a[1], b[3] - b[1])):
        return False
    x0, x1 = math.ceil(a[2]), math.floor(b[0])
    y0, y1 = math.floor(min(a[1], b[1])), math.ceil(max(a[3], b[3]))
    slab = ink[max(0, y0):min(ink.shape[0], y1), max(0, x0):min(ink.shape[1], x1)]
    return bool(slab.size and np.any(np.count_nonzero(slab, axis=0) == 0))


def compose(apple, reader, residual, image):
    """Compare with the unchanged baseline using full raw arrays and source pixels.

    This candidate is not selected by the worker, store, bridge, or App. It accepts
    no evaluation labels, source vocabulary, expected counts, or target IDs.
    """
    gray = cv2.imread(str(image), cv2.IMREAD_GRAYSCALE)
    if gray is None:
        raise ValueError('INVALID_PAGE_IMAGE')
    ink = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
    _, labels, stats, _ = cv2.connectedComponentsWithStats(ink, connectivity=8)
    components = labels, stats
    words, decisions = core.compose(apple, reader, residual)
    words, decisions = copy.deepcopy(words), copy.deepcopy(decisions)
    selected = {w['id'] for w in words}
    for a in list(words):
        if a['engine'] != 'apple' or a['id'] not in selected or any(c.isspace() for c in a['text']):
            continue
        ts = [t for t in reader if t['id'] not in selected and core.same_row(a['bbox'], t['bbox']) and
              core.old.overlap(a['bbox'], t['bbox']) >= .45 and
              a['bbox'][0] - 8 <= (t['bbox'][0] + t['bbox'][2]) / 2 <= a['bbox'][2] + 8]
        ts.sort(key=lambda t: (t['bbox'][0], t['id']))
        if len(ts) < 2 or len({t['line_id'] for t in ts}) != 1 or any(
                t['confidence'] < 30 or not any(c.isalpha() for c in _body(t['text'])) or
                not _ink_count(ink, t['bbox'], t['bbox']) for t in ts):
            continue
        if _body(a['text']) != ''.join(_body(t['text']) for t in ts):
            continue
        punct = _punctuation(''.join(t['text'] for t in ts))
        for mark in _punctuation(a['text']):
            if mark not in punct:
                break
            punct.remove(mark)
        else:
            span = [ts[0]['bbox'][0], min(t['bbox'][1] for t in ts),
                    max(t['bbox'][2] for t in ts), max(t['bbox'][3] for t in ts)]
            if not _complete_horizontal(a['bbox'], span) or not all(
                    _observed_space(ink, x, y) for x, y in zip(ts, ts[1:])):
                continue
            proofs = _blocker_proofs(words, a, ts, reader, ink, components)
            if proofs is None:
                continue
            members = copy.deepcopy(a['source_members'])
            words = [w for w in words if w['id'] != a['id']]
            selected.remove(a['id'])
            for t in ts:
                words.append(dict(t, review=True, source_members=members,
                                  reason='complete_row_group_independent_words',
                                  candidate_policy=CANDIDATE_VERSION))
                selected.add(t['id'])
            decisions.append(dict(state='TEXT_SEGMENTATION_DRAFT', source_members=members,
                                  reader_ids=[t['id'] for t in ts], policy=CANDIDATE_VERSION,
                                  source_pixel_spacing=True, support_conflicts_resolved=proofs))
    for t in reader:
        if t['id'] in selected or t['confidence'] < 80 or core.old.greek(t['text']) < 2 or not _ink_count(
                ink, t['bbox'], t['bbox']) or any(
                c.isalpha() and 'LATIN' in unicodedata.name(c, '') for c in t['text']):
            continue
        owners = [a for a in words if a['engine'] == 'apple' and core.same_row(a['bbox'], t['bbox']) and
                  core.old.overlap(a['bbox'], t['bbox']) >= .45 and _complete_horizontal(a['bbox'], t['bbox'])]
        if len(owners) != 1:
            continue
        a = owners[0]
        same_owner_readers = [q for q in reader if core.same_row(a['bbox'], q['bbox']) and
                             core.old.overlap(a['bbox'], q['bbox']) >= .45]
        if [q['id'] for q in same_owner_readers] != [t['id']]:
            continue
        proofs = _blocker_proofs(words, a, [t], reader, ink, components)
        if not proofs:
            continue
        words = [w for w in words if w['id'] != a['id']]
        selected.remove(a['id'])
        words.append(dict(t, review=True, source_members=copy.deepcopy(a['source_members']),
                          reason='pixel_separated_greek_independent_support', candidate_policy=CANDIDATE_VERSION))
        selected.add(t['id'])
        decisions.append(dict(state='GREEK_PADDING_DRAFT', reader_id=t['id'],
                              source_members=copy.deepcopy(a['source_members']), policy=CANDIDATE_VERSION,
                              support_conflicts_resolved=proofs))
    words = core.nfc_words([w for row in core.reading_rows(words) for w in row])
    core.ownership(words)
    return words, decisions
