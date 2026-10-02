"""Versioned TOC layout evidence and conservative semantic projection.

No reference text is an input. No OCR, support membership or raster is changed.
Uncertain evidence keeps text and positioning unchanged. This detector's
confirmed state means policy-confirmed, never independent human Gold.
"""
import base64
import io
import re

import numpy as np
from PIL import Image
from scipy import ndimage

import spatial_verification as v

POLICY = 'toc-dotted-leader/1'
SCHEMA = 'mpdf-toc-semantic-projection/1'


def otsu(gray):
    histogram = np.bincount(gray.ravel(), minlength=256).astype(float)
    weight = np.cumsum(histogram)
    weighted = np.cumsum(histogram * np.arange(256))
    denominator = weight * (weight[-1] - weight)
    variance = np.zeros(256)
    np.divide((weighted[-1]*weight - weighted*weight[-1])**2,
              denominator, out=variance, where=denominator > 0)
    return int(np.argmax(variance)) + 1


def components(mask):
    labels, _ = ndimage.label(mask, np.ones((3, 3)))
    rows = []
    for index, bounds in enumerate(ndimage.find_objects(labels), start=1):
        if bounds is None:
            continue
        ys, xs = bounds
        area = int(np.count_nonzero(labels[bounds] == index))
        if area >= 2:
            rows.append([xs.start, ys.start, xs.stop, ys.stop, area])
    return rows


def detect_mask(gray, threshold):
    mask = gray < threshold
    h, w = gray.shape
    items = components(mask)
    glyphs = [x for x in items if x[3]-x[1] >= max(5, h*.35)]
    if len(glyphs) < 2:
        return None
    height = float(np.median([x[3]-x[1] for x in glyphs]))
    baseline = float(np.median([x[3] for x in glyphs]))
    right = max(x[2] for x in glyphs)
    trailing = [x for x in items if x[0] >= right]
    dots = [x for x in trailing if max(x[2]-x[0], x[3]-x[1]) <= height*.35
            and .35 <= (x[2]-x[0])/(x[3]-x[1]) <= 2.5]
    if len(dots) < 6:
        return None
    dots.sort()
    fragments = [x for x in trailing if x not in dots]
    # A final Greek letter can have several disconnected, short components.
    # Keep a compact fragment cluster next to the last tall glyph in the title;
    # never absorb outliers embedded in the leader or a remote glyph/noise patch.
    if fragments and not all(x[2] <= dots[0][0] and x[2]-right <= height*2 for x in fragments):
        return None
    centers = np.array([[(x[0]+x[2])/2, (x[1]+x[3])/2] for x in dots])
    gaps = np.diff(centers[:, 0]);pitch = float(np.median(gaps))
    dotwidth = float(np.median([x[2]-x[0] for x in dots]))
    if pitch < max(1.2*dotwidth, height*.2) or np.std(gaps)/pitch > .35:
        return None
    if np.min(gaps) < pitch*.5 or np.max(gaps) > pitch*1.8:
        return None
    slope, intercept = np.polyfit(centers[:, 0], centers[:, 1], 1)
    if abs(slope) > .05 or np.max(np.abs(centers[:, 1] - (slope*centers[:, 0]+intercept))) > height*.2:
        return None
    # Compare at the same x: a sloping scan's far-right dots do not share the
    # left-hand glyph baseline y. The old constant-y comparison rejected even
    # a straight tilted leader. No raster deskew or geometry change is needed.
    glyph_x = float(np.median([(x[0]+x[2])/2 for x in glyphs]))
    if not baseline-height*.35 <= slope*glyph_x+intercept <= baseline+height*.2:
        return None
    left, right = min(x[0] for x in dots), max(x[2] for x in dots)
    if right-left < max(height*2, w*.1):
        return None
    prefix = np.argwhere(mask[:, :left])
    if len(prefix) == 0:
        return None
    title_right = int(prefix[:, 1].max())+1
    bbox = [left, min(x[1] for x in dots), right, max(x[3] for x in dots)]
    leader_mask = np.zeros_like(mask)
    leader_mask[:, left:] = mask[:, left:]
    return {'threshold': threshold, 'bbox': bbox, 'title_right': title_right,
            'glyph_height': height, 'dot_count_diagnostic': len(dots), 'components': dots,
            'leader_mask_sha256': v.sha(np.packbits(leader_mask).tobytes()), 'mask_shape': [h, w],
            'pitch': pitch, 'baseline_slope': float(slope)}


def detect(png):
    with Image.open(io.BytesIO(png)) as image:
        gray = np.asarray(image.convert('L'))
    threshold = otsu(gray)
    first = detect_mask(gray, threshold)
    second = detect_mask(gray, min(245, threshold+30))
    if first is None or second is None:
        return {'status': 'uncertain', 'reason': 'No conservative dual-threshold leader confirmation'}
    tolerance = first['glyph_height']*.25
    if max(abs(a-b) for a, b in zip(first['bbox'], second['bbox'])) > tolerance or abs(first['title_right']-second['title_right']) > tolerance:
        return {'status': 'uncertain', 'reason': 'Threshold-sensitive leader boundary'}
    return {'status': 'confirmed', 'confirmation': 'deterministic_policy_not_human_gold',
            'primary': first, 'secondary': second}


def build(prepared, response_json, response_sha, context_json, context_sha):
    prepared.validate_response(response_json, response_sha)
    v.hash_matches(context_json, context_sha, 'Selected TOC context')
    context = v.parse(context_json)
    g = v.parse(prepared.geometry_json)
    v.require(context['schema'] == 'mpdf-toc-context/1' and context['page_id'] == g['spatial']['page_id']
              and context['image_sha256'] == g['image_sha256'], 'TOC context source mismatch')
    known_units = {u['unit_id'] for u in g['spatial']['units']}
    v.require(len(context['unit_ids']) == len(set(context['unit_ids']))
              and set(context['unit_ids']) <= known_units, 'TOC scope membership')
    supports = g['spatial']['supports'];texts = v.parse(response_json)['supports']
    byid = {s['support_id']: s for s in supports}
    textmap = {s['support_id']: s['text'] for s in texts}
    payload = prepared.checked_payload()
    pngs = {s['support_id']: base64.b64decode(payload['contents'][0]['parts'][2+2*i]['inlineData']['data'])
            for i, s in enumerate(supports)}
    evidence, changes = [], {}
    for unit in g['spatial']['units']:
        if context.get('scope') != 'source_reviewed_toc_region' or unit['unit_id'] not in context['unit_ids']:
            continue
        if len(unit['support_ids']) != 2:
            continue
        title_id, number_id = unit['support_ids']
        title, number = byid[title_id], byid[number_id]
        tb, nb = title['bbox'], number['bbox']
        # Existing membership is necessary, not inferred or changed here.
        aligned = min(tb[3], nb[3]) > max(tb[1], nb[1]) and tb[2] < nb[0]
        if not aligned or not re.fullmatch(r'[0-9]{1,5}', textmap[number_id]):
            evidence.append({'unit_id': unit['unit_id'], 'support_id': title_id, 'status': 'uncertain',
                             'reason': 'No aligned right-hand numeric support'})
            continue
        result = detect(pngs[title_id])
        result.update(unit_id=unit['unit_id'], support_id=title_id, number_support_id=number_id,
                      png_sha256=v.sha(pngs[title_id]))
        evidence.append(result)
        if result['status'] != 'confirmed':
            continue
        original = textmap[title_id]
        suffix = re.search(r'[. \t]+$', original)
        if suffix and '.' in suffix.group():
            start = suffix.start()
        elif original and original[-1].isalnum():
            # Model may omit a leader entirely; no letters are invented/deleted.
            start = len(original)
        else:
            result['status'] = 'uncertain'
            result['reason'] = 'Cannot safely map raw terminal punctuation to leader'
            continue
        semantic = original[:start]
        crop = title['crop_bbox']
        right = min(tb[2], crop[0] + result['primary']['title_right'])
        bbox = [tb[0], tb[1], right, tb[3]]
        if not semantic or not (tb[0] < right < tb[2]):
            result['status'] = 'uncertain';result['reason'] = 'Invalid title projection bounds'
            continue
        changes[title_id] = {'semantic_text': semantic, 'removed_span': [start, len(original)],
                            'render_bbox': bbox, 'leader_present': True,
                            'leader_bbox': [crop[0]+result['primary']['bbox'][0], crop[1]+result['primary']['bbox'][1],
                                            crop[0]+result['primary']['bbox'][2], crop[1]+result['primary']['bbox'][3]]}
    evidence_doc = {'schema': 'mpdf-toc-leader-evidence/1', 'policy': POLICY,
                    'geometry_sha256': prepared.expected.contract_sha256, 'context_sha256': context_sha,
                    'records': evidence}
    projection = {'schema': SCHEMA, 'policy': POLICY, 'page_index': g['page_index'],
                  'geometry_sha256': prepared.expected.contract_sha256, 'response_sha256': response_sha,
                  'input_manifest_sha256': prepared.manifest_sha256, 'context_sha256': context_sha,
                  'evidence_sha256': v.sha(v.canonical(evidence_doc).encode()),
                  'supports': []}
    for support in supports:
        sid = support['support_id'];original = textmap[sid]
        row = {'support_id': sid, 'source_text': original, 'semantic_text': original,
               'removed_span': [len(original), len(original)], 'render_bbox': support['bbox'],
               'leader_present': False, 'leader_bbox': None}
        row.update(changes.get(sid, {}));projection['supports'].append(row)
    return evidence_doc, projection


def verify(prepared, response, response_sha, context, context_sha, evidence, projection):
    expected_evidence, expected_projection = build(prepared, response, response_sha, context, context_sha)
    v.require(evidence == expected_evidence and projection == expected_projection,
              'Projection differs from deterministic image/text derivation')
