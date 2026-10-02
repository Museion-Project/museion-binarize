"""Image-only recovery candidates corroborated by retained local detections.

Complete connected components may survive a truncated detector row. This does
not authorize accepting that row's whole rectangle or ignoring duplicate guards.
"""
import copy
import statistics
import finalist_adapters as a
import shared_geometry_adapter as s


def candidates(doc, image, records):
    _, _, _, mask = s.masks(image)
    crops = [c['transcription_crop_bbox'] for c in
             s.crops(doc['logical_lines'], mask, doc['column_bands'])]
    heights = [a.height(b) for b in crops if a.width(b) > 4*a.height(b)]
    h = statistics.median(heights) if heights else 20
    components = a.components(mask, [0, 0, image.width, image.height])
    unowned = [c for c in components if .22*h < a.height(c) < 1.5*h
               and .08*h < a.width(c) < 2.5*h and c[4] > .025*h*h
               and not any(a.overlap(c, b) > .1*a.area(c) for b in crops)]
    result = []
    claimed = set()
    for record in records:
        receipt = record['receipt']; roi = receipt['roi']['bbox']
        for row in receipt.get('mapped', []):
            if row['accepted'] or (row.get('confidence') or 0) < .9:
                continue
            b = row['bbox']
            found = [c for c in unowned if tuple(c) not in claimed
                     and roi[0]+2 < c[0] and roi[1]+2 < c[1]
                     and c[2] < roi[2]-2 and c[3] < roi[3]-2
                     and a.overlap(c, b) >= .8*a.area(c)]
            if not found:
                continue
            # Attach punctuation and detached diacritics only to a confirmed
            # glyph cluster, never use these small components as triggers.
            for _ in range(3):
                envelope = a.envelope([c[:4] for c in found])
                satellites = [c for c in components if c not in found
                              and tuple(c) not in claimed and a.height(c) < 1.5*h
                              and roi[0]+2 < c[0] and roi[1]+2 < c[1]
                              and c[2] < roi[2]-2 and c[3] < roi[3]-2
                              and a.overlap(c, b) >= .5*a.area(c)
                              and max(0, envelope[0]-c[2], c[0]-envelope[2]) < .65*h
                              and c[1] >= envelope[1]-.35*h and c[3] <= envelope[3]+.2*h
                              and not any(a.overlap(c, z) > .1*a.area(c) for z in crops)]
                if not satellites:
                    break
                found += satellites
            box = a.envelope([c[:4] for c in found])
            if a.height(box) > 1.5*h:
                continue
            neighbors = [f for f in doc['fragments']
                         if a.overlap(f['bbox'], b) > 0
                         and abs(a.yc(f['bbox'])-a.yc(box)) < .45*h
                         and 0 <= f['bbox'][0]-box[2] < 1.5*h
                         and a.width(f['bbox']) > 2*h]
            if len(neighbors) > 1:
                continue
            if not neighbors and any(a.overlap(box, z) > 0 for z in crops):
                continue
            result.append({'receipt_path': record['path'],
                           'receipt_sha256': record['sha256'],
                           'source_local_index': row['source_local_index'],
                           'components': found, 'bbox': box,
                           'extend_fragment_id': neighbors[0]['fragment_id'] if neighbors else None})
            claimed.update(tuple(c) for c in found)
    return result


def recover(doc, image, records):
    evidence = candidates(doc, image, records)
    if not evidence:
        return doc
    d = copy.deepcopy(doc)
    for item in evidence:
        fid = item['extend_fragment_id']
        if fid:
            fragment = next(f for f in d['fragments'] if f['fragment_id'] == fid)
            fragment['bbox'] = a.envelope([fragment['bbox'], item['bbox']])
            fragment['polygon'] = s.rectpoly(fragment['bbox'])
        else:
            fid = 'component-'+a.digest(item)[:20]
            b = item['bbox']
            row = next(r for r in records if r['path'] == item['receipt_path'])['receipt']['mapped'][item['source_local_index']]
            fragment = {'fragment_id': fid, 'bbox': b, 'polygon': s.rectpoly(b),
                        'source_box_index': item['source_local_index'],
                        'source_pointer': item['receipt_path']+'#/mapped/'+str(item['source_local_index']),
                        'confidence': row['confidence'], 'region_ids': [], 'hints': [],
                        'features': {'width': a.width(b), 'height': a.height(b), 'aspect': a.width(b)/a.height(b)}}
            d['fragments'].append(fragment)
        fragment['hints'].append({'kind': 'complete_component_local_evidence', **item})
    _, _, _, mask = s.masks(image)
    bands = a.adaptive_bands(d['fragments']); a.assign_columns(d['fragments'], bands)
    d['column_bands'] = bands
    d['logical_lines'] = s.organize(d['fragments'], mask, bands, d['page_id'])
    d['provenance']['component_recovery'] = {'version': 1, 'base': doc, 'evidence': evidence}
    d['geometry_sha256'] = a.digest({k: v for k, v in d.items() if k != 'geometry_sha256'})
    return d
