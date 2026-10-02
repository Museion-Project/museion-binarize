"""Reference-free spatial supports, separate from logical membership.

Internal prototype: a support is an observed fragment and its bounded crop,
not the envelope of a semantic group. No text is assigned here.
"""
import copy
import math
from PIL import Image
import shared_geometry_adapter as s
import finalist_adapters as a

VERSION = 'spatial-supports/1'


def row_provenance(left, right, fragments):
    """Require one uniquely overlapping source row for a detached prefix."""
    b, z = left['bbox'], right['bbox']
    h = min(a.height(b), a.height(z))
    pre = right.get('pre_refinement_bbox')
    source = right.get('source_fragment_id')
    if not pre or not source or not any(q['kind'] == 'image_supported_row_split' for q in right['hints']):
        return False
    if a.width(b) > 2*a.height(b) or a.width(z) <= 4*h or b[2] > z[0]:
        return False
    if pre[1] != z[1] or pre[3] != z[3] or not (pre[0] < z[0] and pre[2] >= z[2]):
        return False
    if min(b[2], pre[2]) <= max(b[0], pre[0]):
        return False
    siblings = [q for q in fragments if q.get('source_fragment_id') == source
                and q['column_id'] == right['column_id']
                and any(v['kind'] == 'image_supported_row_split' for v in q['hints'])]
    overlapping = [q for q in siblings if min(b[3], q['bbox'][3])-max(b[1], q['bbox'][1])
                   > .5*min(a.height(b), a.height(q['bbox']))]
    return len(siblings) > 1 and len(overlapping) == 1 and overlapping[0]['fragment_id'] == right['fragment_id']


def units_with_provenance(doc):
    """Re-evaluate only an unprotected singleton pair with retained row evidence.

    This hypothesis changes membership, never the support/crop coordinates.
    Independent token, column and baseline guards remain mandatory.
    """
    units = copy.deepcopy(doc['logical_lines'])
    fs = {f['fragment_id']: f for f in doc['fragments']}
    consumed = set()
    for unit in units:
        if unit['line_id'] in consumed or len(unit['fragment_ids']) != 1:
            continue
        left = fs[unit['fragment_ids'][0]]
        if any(h['kind'] in {'recurring_margin_lane', 'independent_end_unit'} for h in left['hints']):
            continue
        possible = []
        for other in units:
            if other is unit or other['line_id'] in consumed or len(other['fragment_ids']) != 1:
                continue
            right = fs[other['fragment_ids'][0]]
            if any(h['kind'] in {'recurring_margin_lane', 'independent_end_unit'} for h in right['hints']):
                continue
            b, z = left['bbox'], right['bbox']; h = min(a.height(b), a.height(z))
            if left['column_id'] != right['column_id'] or abs(a.yc(b)-a.yc(z)) > .42*h or max(a.height(b), a.height(z)) > 1.6*h:
                continue
            envelope = a.envelope([b, z])
            if any(envelope[0] < band['cut_x'] < envelope[2] and band['bbox'][1] <= a.yc(envelope) <= band['bbox'][3] for band in doc['column_bands']):
                continue
            if row_provenance(left, right, doc['fragments']):
                possible.append((other, right))
        if len(possible) != 1:
            continue
        other, right = possible[0]
        consumed.add(other['line_id'])
        unit['fragment_ids'] += other['fragment_ids']
        unit['bbox'] = a.envelope([unit['bbox'], other['bbox']])
        unit['line_id'] = doc['page_id']+'-l-'+a.digest(sorted(unit['fragment_ids']))[:16]
        unit['hints'].append({'kind': 'split_row_provenance_support', 'source_fragment_id': right['source_fragment_id'],
                              'prefix_fragment_id': left['fragment_id'], 'body_fragment_id': right['fragment_id']})
    return s.order_units([u for u in units if u['line_id'] not in consumed], doc['column_bands'])


def build(doc, image):
    """Deterministic observed supports and their ordered logical ownership."""
    _, _, _, mask = s.masks(image)
    fs = {f['fragment_id']: f for f in doc['fragments']}
    atoms = [{'line_id': fid, 'bbox': f['bbox'][:]} for fid, f in fs.items()]
    atom_crops = {c['line_id']: c for c in s.crops(atoms, mask, doc['column_bands'])}
    parent_crops = {c['line_id']: c['transcription_crop_bbox'] for c in s.crops(doc['logical_lines'], mask, doc['column_bands'])}
    parents = {fid: unit['line_id'] for unit in doc['logical_lines'] for fid in unit['fragment_ids']}
    logical = units_with_provenance(doc)
    supports = []; units = []
    for unit in logical:
        ids = []; regions = []
        for fid in unit['fragment_ids']:
            fragment = fs[fid]
            crop = atom_crops[fid]['transcription_crop_bbox']
            limit = parent_crops[parents[fid]]
            # Repackaging must never expand the previously bounded input.
            bounded = [max(crop[0], limit[0]), max(crop[1], limit[1]),
                       min(crop[2], limit[2]), min(crop[3], limit[3])]
            regions.append({'fragment_id': fid, 'bbox': fragment['bbox'][:],
                             'polygon': copy.deepcopy(fragment['polygon']),
                             'crop_bbox': bounded, 'crop_parent_unit_id': parents[fid],
                             'crop_parent_bbox': limit[:],
                             'source_fragment_id': fragment.get('source_fragment_id', fid),
                             'source_pointer': fragment['source_pointer'],
                             'column_id': fragment['column_id'], 'band_id': fragment['band_id']})
        # Raw detections can overlap. One physical pixel cannot be transcribed
        # independently twice. Coalesce connected crop regions within a unit,
        # retaining their exact union as the input mask, never its raw envelope.
        groups = []
        for region in regions:
            touching = [g for g in groups if any(a.overlap(region['crop_bbox'], r['crop_bbox']) > 0 for r in g)]
            merged = [region] + [r for g in touching for r in g]
            groups = [g for g in groups if g not in touching] + [merged]
        positions = {fid: i for i, fid in enumerate(unit['fragment_ids'])}
        groups.sort(key=lambda group: min(positions[r['fragment_id']] for r in group))
        for group in groups:
            group.sort(key=lambda r: positions[r['fragment_id']])
            fids = [r['fragment_id'] for r in group]
            sid = doc['page_id']+'-s-'+a.digest(sorted(fids))[:16]
            ids.append(sid)
            supports.append({'support_id': sid, 'reading_order': len(supports),
                             'fragment_ids': fids, 'bbox': a.envelope([r['bbox'] for r in group]),
                             'crop_bbox': a.envelope([r['crop_bbox'] for r in group]),
                             'regions': group, 'column_id': unit['column_id'], 'band_id': unit['band_id']})
        units.append({'unit_id': unit['line_id'], 'reading_order': unit['reading_order'],
                      'support_ids': ids, 'column_id': unit['column_id'], 'band_id': unit['band_id'],
                      'membership_evidence': copy.deepcopy(unit['hints'])})
    result = {'version': VERSION, 'page_id': doc['page_id'], 'provider': doc['provider'],
              'width': doc['width'], 'height': doc['height'], 'supports': supports, 'units': units,
              'parent_geometry_sha256': doc['geometry_sha256']}
    result['spatial_sha256'] = a.digest(result)
    validate(result, doc)
    return result


def materialize(support, image):
    """Single source-raster view of a connected support; no new scene pixels."""
    x0,y0,x1,y1 = s.bounds(support['crop_bbox'], (image.height,image.width))
    crop = Image.new('RGB', (x1-x0,y1-y0), 'white')
    for region in support['regions']:
        left,top,right,bottom = s.bounds(region['crop_bbox'], (image.height,image.width))
        crop.paste(image.crop((left,top,right,bottom)), (left-x0,top-y0))
    return crop


def validate(spatial, doc):
    supports = spatial['supports']; units = spatial['units']
    assert [q['reading_order'] for q in supports] == list(range(len(supports)))
    assert [q['reading_order'] for q in units] == list(range(len(units)))
    assert [sid for u in units for sid in u['support_ids']] == [q['support_id'] for q in supports]
    assert len({q['support_id'] for q in supports}) == len(supports)
    fragments = {f['fragment_id']: f for f in doc['fragments']}
    assert sorted(fid for q in supports for fid in q['fragment_ids']) == sorted(fragments)
    for support in supports:
        assert [r['fragment_id'] for r in support['regions']] == support['fragment_ids']
        for region in support['regions']:
            fragment = fragments[region['fragment_id']]
            assert region['bbox'] == fragment['bbox'] and region['polygon'] == fragment['polygon']
            b, c = region['bbox'], region['crop_bbox']
            assert 0 <= c[0] <= b[0] < b[2] <= c[2] <= doc['width']
            assert 0 <= c[1] <= b[1] < b[3] <= c[3] <= doc['height']
        assert support['bbox'] == a.envelope([r['bbox'] for r in support['regions']])
        assert support['crop_bbox'] == a.envelope([r['crop_bbox'] for r in support['regions']])
