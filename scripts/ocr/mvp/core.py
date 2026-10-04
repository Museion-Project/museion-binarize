"""Pixel-derived residuals and ownership-safe reading projections, v1.
Region boxes describe recognition support, never fabricated character precision.
"""
import copy
import hashlib
import json
import statistics
import unicodedata
from pathlib import Path
import cv2
import numpy as np
from scripts.ocr.free_local import pipeline as old

CONFIG_VERSION = 'mvp-local-v2'
CONSUMER_POLICY = 'row-sequence-complete-punctuation-v1'

def digest(x):
    return hashlib.sha256(json.dumps(x, sort_keys=True, ensure_ascii=False, allow_nan=False).encode()).hexdigest()

def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()

def nfc_words(words):
    return [dict(w, text=unicodedata.normalize('NFC', w['text'])) for w in words]

def discover_residual(image, apple, max_regions=32):
    """Detect visible ink not covered by Apple word support, without text references.
    Dilate ink into row fragments; keep raw target and padded recognition canvas separate.
    """
    gray = cv2.imread(str(image), cv2.IMREAD_GRAYSCALE)
    if gray is None:
        raise ValueError('INVALID_PAGE_IMAGE')
    h,w = gray.shape
    ink = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
    uncovered = ink.copy()
    for a in apple:
        x,y,r,b = a['bbox']
        cv2.rectangle(uncovered, (max(0,int(x)-2),max(0,int(y)-2)),
                      (min(w,int(r)+2),min(h,int(b)+2)), 0, -1)
    joined = cv2.dilate(uncovered, cv2.getStructuringElement(cv2.MORPH_RECT,(13,3)))
    count, labels, stats, _ = cv2.connectedComponentsWithStats(joined)
    regions=[]
    for x,y,bw,bh,pixels in stats[1:]:
        x,y,bw,bh=map(int,(x,y,bw,bh))
        actual=int(np.count_nonzero(uncovered[y:y+bh,x:x+bw]))
        if bw<12 or bh<7 or bh>150 or actual<30 or bw>int(w*.95):
            continue
        target=[x,y,x+bw,y+bh]
        canvas=[max(0,x-12),max(0,y-8),min(w,x+bw+12),min(h,y+bh+8)]
        regions.append(dict(member_id='res-'+digest(target)[:16],target_bbox=target,
                            canvas_bbox=canvas,ink_pixels=actual,precision='observed region'))
    # Join nearby uncovered fragments into observed row supports. Independent
    # connected components remain recorded; the canvas never creates word precision.
    groups=[]
    for region in sorted(regions,key=lambda r:(r['target_bbox'][1],r['target_bbox'][0])):
        box=region['target_bbox'];height=box[3]-box[1]
        group=next((g for g in groups if same_row(g['target_bbox'],box) and
                    max(0,box[0]-g['target_bbox'][2],g['target_bbox'][0]-box[2])<=4*height),None)
        if group is None:
            groups.append(dict(region,component_supports=[box]));continue
        previous=group['target_bbox'];joined=[min(previous[0],box[0]),min(previous[1],box[1]),max(previous[2],box[2]),max(previous[3],box[3])]
        group['target_bbox']=joined;group['canvas_bbox']=[max(0,joined[0]-12),max(0,joined[1]-8),min(w,joined[2]+12),min(h,joined[3]+8)]
        group['component_supports'].append(box);group['ink_pixels']+=region['ink_pixels'];group['member_id']='res-'+digest(group['component_supports'])[:16]
    regions=groups
    regions.sort(key=lambda r:(-r['ink_pixels'], r['target_bbox'][1],r['target_bbox'][0]))
    skipped=regions[max_regions:]
    return sorted(regions[:max_regions],key=lambda r:(r['target_bbox'][1],r['target_bbox'][0])), skipped

def same_row(a,b):
    return min(a[3],b[3])-max(a[1],b[1]) >= .55*min(a[3]-a[1],b[3]-b[1])

def reading_rows(words):
    """Complete-link geometric rows, split at gutters; independent of engine IDs.

    A common export baseline removes top-edge jitter without inventing words or
    connecting columns across a wide gutter. Boxes and original IDs are retained.
    """
    groups=[]
    for word in sorted(words,key=lambda w:((w['bbox'][1]+w['bbox'][3])/2,w['bbox'][0],w['id'])):
        box=word['bbox']
        compatible=[g for g in groups if all(same_row(box,w['bbox']) and
                    abs((box[1]+box[3]-w['bbox'][1]-w['bbox'][3])/2)<=.4*max(box[3]-box[1],w['bbox'][3]-w['bbox'][1]) for w in g)]
        if compatible:
            min(compatible,key=lambda g:abs(statistics.median((w['bbox'][1]+w['bbox'][3])/2 for w in g)-(box[1]+box[3])/2)).append(word)
        else:groups.append([word])
    rows=[]
    for group in groups:
        group.sort(key=lambda w:(w['bbox'][0],w['id']))
        height=statistics.median(w['bbox'][3]-w['bbox'][1] for w in group)
        current=[]
        for word in group:
            if current and word['bbox'][0]-current[-1]['bbox'][2]>2.5*height:
                rows.append(current);current=[]
            current.append(word)
        if current:rows.append(current)
    return sorted(rows,key=lambda row:(statistics.median(w['bbox'][1] for w in row),row[0]['bbox'][0]))

def punctuation_core(text):
    text=unicodedata.normalize('NFC',text)
    return ''.join(c for c in text if not unicodedata.category(c).startswith('P'))

def punctuation_equivalent_positions(a,b):
    """A typographic variant may transfer; missing/extra marks cannot be inferred.
    Preserve punctuation position, multiplicity and kind, including comma pairs.
    """
    a=unicodedata.normalize('NFC',a);b=unicodedata.normalize('NFC',b)
    def kind(c):
        name=unicodedata.name(c,'')
        if c=='"' or ('DOUBLE' in name and 'QUOTATION' in name):return 'double_quote'
        if c=="'" or ('SINGLE' in name and 'QUOTATION' in name) or 'APOSTROPHE' in name:return 'single_quote'
        if unicodedata.category(c)=='Pd':return 'dash'
        return c
    return len(a)==len(b) and all(x==y or (unicodedata.category(x).startswith('P') and
                unicodedata.category(y).startswith('P') and kind(x)==kind(y)) for x,y in zip(a,b))

def complete_residual_support(image, regions):
    """Recover complete ink word support before reading a residual fragment.

    The unmasked image is the only source for expansion. Context is bounded to
    the observed row; it is not adoption support. Ambiguous tokens crossing the
    target boundary are recorded by the worker as UNKNOWN.
    """
    gray=cv2.imread(str(image),cv2.IMREAD_GRAYSCALE)
    if gray is None:raise ValueError('INVALID_PAGE_IMAGE')
    h,w=gray.shape
    ink=cv2.threshold(gray,0,255,cv2.THRESH_BINARY_INV+cv2.THRESH_OTSU)[1]
    joined=cv2.dilate(ink,cv2.getStructuringElement(cv2.MORPH_RECT,(13,3)))
    _,_,stats,_=cv2.connectedComponentsWithStats(joined)
    supports=[[int(x),int(y),int(x+bw),int(y+bh)] for x,y,bw,bh,n in stats[1:] if bh>=7 and bh<=150 and n>=30]
    output=[]
    for region in regions:
        region=copy.deepcopy(region);original=region['target_bbox']
        matches=[b for b in supports if same_row(original,b) and old.intersection(original,b)>0 and b[2]-b[0]<w*.95]
        if matches:
            target=[min([original[0]]+[b[0] for b in matches]),min([original[1]]+[b[1] for b in matches]),max([original[2]]+[b[2] for b in matches]),max([original[3]]+[b[3] for b in matches])]
        else:target=list(original)
        height=target[3]-target[1]
        region.update(fragment_bbox=original,target_bbox=target,canvas_bbox=[max(0,target[0]-4*height),max(0,target[1]-8),min(w,target[2]+4*height),min(h,target[3]+8)],support_policy='complete-ink-word-v1')
        output.append(region)
    return output

def residual_member_in_target(box,target):
    """Adopt only a complete observed token, never a clipping intersection."""
    area=(box[2]-box[0])*(box[3]-box[1])
    return area>0 and old.intersection(box,target)/area>=.9 and same_row(box,target)

def reader_support_conflicts(box, words):
    """Expose the observed supports that trigger the existing overlap guard.

    This records reader geometry, not source ink or a safe adoption decision.
    Preserve the strict positive-intersection rule, including fractional edges.
    """
    conflicts=[]
    for word in words:
        intersect=old.intersection(box,word['bbox'])
        if intersect>0:
            conflicts.append(dict(member_id=word['id'],engine=word['engine'],
                bbox=copy.deepcopy(word['bbox']),source_members=copy.deepcopy(word.get('source_members',[])),
                intersection_area=intersect,overlap_fraction=old.overlap(box,word['bbox']),
                same_row=same_row(box,word['bbox']),support_status='READER_OBSERVATION_UNVERIFIED'))
    return conflicts

def compose(apple, reader, residual):
    """All-or-nothing ownership transfer. Ambiguous broad words remain pending.
    A broad Apple row may be retired only if independent real words span its entire
    support on multiple rows. Otherwise it is quarantined, never silently replaced.
    """
    if len({w['id'] for w in apple})!=len(apple) or len({w['id'] for w in reader})!=len(reader):raise ValueError('DUPLICATE_INPUT_MEMBER')
    original=copy.deepcopy(apple); active={w['id']:dict(w, review=False, source_members=[w['id']]) for w in original}
    decisions=[]; consumed=set(); selected=[]
    # Mixed-row Apple boxes must not overlap other rows in the PDF projection.
    for a in original:
        ts=[t for t in reader if old.overlap(a['bbox'],t['bbox'])>=.45]
        rows={t['line_id'] for t in ts}
        if len(rows)>1 and ts:
            union=[min(t['bbox'][0] for t in ts),min(t['bbox'][1] for t in ts),
                   max(t['bbox'][2] for t in ts),max(t['bbox'][3] for t in ts)]
            covers=(union[0]<=a['bbox'][0]+8 and union[2]>=a['bbox'][2]-8)
            if covers and all(t['confidence']>=45 for t in ts):
                active.pop(a['id'],None)
                decisions.append(dict(state='MIXED_ROW_RETIRED',source_members=[a['id']],reader_ids=[t['id'] for t in ts]))
                for t in ts:
                    if t['id'] not in consumed:
                        selected.append(dict(t,review=True,source_members=[a['id']],reason='mixed_row_independent_words'))
                        consumed.add(t['id'])
            else:
                # Retain raw evidence, quarantine malformed geometry rather than inserting duplicates.
                active.pop(a['id'],None)
                decisions.append(dict(state='REVIEW',reason='mixed_row_incomplete_scope',source_members=[a['id']]))
    # A single Apple token can cover two actual Greek words. Transfer the
    # complete independently observed group to one owner, never erase it piecemeal.
    for a in list(active.values()):
        if a['id'] not in active:continue
        box=a['bbox'];tolerance=max(8,(box[2]-box[0])*.12)
        ts=[t for t in reader if t['id'] not in consumed and same_row(box,t['bbox']) and
            old.overlap(box,t['bbox'])>=.45 and box[0]-8<=(t['bbox'][0]+t['bbox'][2])/2<=box[2]+8]
        if len(ts)<2 or not all(old.greek(t['text']) and t['confidence']>=30 and
            not any(c.isalpha() and 'LATIN' in unicodedata.name(c,'') for c in t['text']) for t in ts):continue
        span=[min(t['bbox'][0] for t in ts),min(t['bbox'][1] for t in ts),max(t['bbox'][2] for t in ts),max(t['bbox'][3] for t in ts)]
        owners=[v for v in active.values() if any(old.intersection(v['bbox'],t['bbox']) for t in ts)]
        complete=span[0]<=box[0]+tolerance and span[2]>=box[2]-tolerance and all(
            same_row(v['bbox'],span) and span[0]<=v['bbox'][0]+tolerance and span[2]>=v['bbox'][2]-tolerance for v in owners)
        if not complete or any(old.intersection(t['bbox'],e['bbox']) for t in ts for e in selected):continue
        members=[v['id'] for v in owners]
        for v in owners:active.pop(v['id'])
        for t in ts:
            selected.append(dict(t,review=True,source_members=members,reason='complete_row_group_independent_words'));consumed.add(t['id'])
        decisions.append(dict(state='GREEK_GROUP_DRAFT',source_members=members,reader_ids=[t['id'] for t in ts]))
    # Transfer punctuation only for one complete, uniquely owned independent
    # word with exactly the same letter/digit/mark body. No character mapping.
    for t in reader:
        if t['id'] in consumed or t['confidence']<80 or any(c.isspace() for c in t['text']):continue
        hits=[a for a in active.values() if old.intersection(a['bbox'],t['bbox'])>0]
        if len(hits)!=1:continue
        a=hits[0];ab=a['bbox'];tb=t['bbox']
        candidates=[q for q in reader if q['id'] not in consumed and old.overlap(ab,q['bbox'])>=.45]
        letter_body=punctuation_core(t['text'])
        if len(candidates)!=1 or not any(c.isalpha() for c in letter_body) or any(c.isspace() for c in a['text']):continue
        tolerance=max(4,.15*(ab[2]-ab[0]))
        complete=same_row(ab,tb) and abs(ab[0]-tb[0])<=tolerance and abs(ab[2]-tb[2])<=tolerance
        if (complete and letter_body==punctuation_core(a['text']) and t['text']!=a['text'] and punctuation_equivalent_positions(a['text'],t['text']) and
                not any(old.intersection(tb,q['bbox'])>0 for q in selected)):
            active.pop(a['id']);consumed.add(t['id'])
            selected.append(dict(t,review=True,source_members=[a['id']],reason='complete_word_punctuation_support'))
            decisions.append(dict(state='PUNCTUATION_DRAFT',source_members=[a['id']],reader_ids=[t['id']],policy=CONSUMER_POLICY))
    for t in reader+residual:
        if t['id'] in consumed:
            continue
        hits=[a for a in active.values() if old.overlap(a['bbox'],t['bbox'])>=.45]
        g=old.greek(t['text']);latin=sum(c.isalpha() and 'LATIN' in unicodedata.name(c,'') for c in t['text'])
        neighbor=any(q['line_id']==t['line_id'] and old.greek(q['text'])>=2 for q in reader+residual)
        strong_context=sum(q['line_id']==t['line_id'] and old.greek(q['text'])>=2 and q['confidence']>=30 for q in reader+residual)>=3
        isgreek=(g>=2 or (g and neighbor)) and latin<=g*.25 and (t['confidence']>=30 or (g>=2 and strong_context and t['confidence']>=10))
        if hits and isgreek:
            safe=all(same_row(a['bbox'],t['bbox']) and a['bbox'][2]-a['bbox'][0]<=3*(t['bbox'][2]-t['bbox'][0]) and not any(q['id']!=t['id'] and same_row(q['bbox'],a['bbox']) and old.overlap(a['bbox'],q['bbox'])>=.45 for q in reader) for a in hits)
            # A candidate intersecting another active support needs complete scope, not partial erasure.
            all_intersections=[a for a in active.values() if old.intersection(a['bbox'],t['bbox'])>0]
            safe=safe and all(a in hits for a in all_intersections)
            if safe and not any(old.overlap(t['bbox'],e['bbox'])>.3 for e in selected):
                members=[a['id'] for a in hits]
                for a in hits:active.pop(a['id'])
                selected.append(dict(t,review=True,source_members=members,reason='greek_independent_support'))
                consumed.add(t['id']);decisions.append(dict(state='GREEK_DRAFT',reader_id=t['id'],source_members=members))
            else:
                decisions.append(dict(state='REVIEW',reader_id=t['id'],reason='ambiguous_overlap'))
        elif not hits and (isgreek or t['confidence']>=65) and old.base(t['text']):
            conflicts=reader_support_conflicts(t['bbox'],list(active.values())+selected)
            if not conflicts:
                selected.append(dict(t,review=True,source_members=[],reason='uncovered_independent_support'))
                consumed.add(t['id']);decisions.append(dict(state='RESIDUAL_DRAFT',reader_id=t['id'],source_members=[]))
            else:
                decisions.append(dict(state='REVIEW',reader_id=t['id'],reason='partial_overlap',support_conflicts=conflicts))
        else:
            # These branches previously kept the reading only in raw evidence.
            # Explain the existing policy without treating it as source truth,
            # changing adoption, or calling a covered alternative a missing word.
            decisions.append(dict(state='NOT_ADOPTED',reader_id=t['id'],
                reason='covered_reader_policy_not_adopted' if hits else 'reader_admission_not_supported',
                support_conflicts=reader_support_conflicts(t['bbox'],list(active.values())+selected),
                admission_observation=dict(greek_admission_supported=isgreek,
                    confidence=t['confidence'],greek_characters=g,latin_characters=latin,
                    shared_row_greek_neighbor=neighbor,has_base_text=bool(old.base(t['text'])),
                    support_status='READER_OBSERVATION_UNVERIFIED')))
    words=nfc_words(list(active.values())+selected)
    # Apple duplicate boxes are not certificates. Quarantine later overlapping copies.
    final=[]
    for a in sorted(words,key=lambda x:(x['bbox'][1],x['bbox'][0],x['id'])):
        conflict=next((b for b in final if old.overlap(a['bbox'],b['bbox'])>.6),None)
        if conflict:
            decisions.append(dict(state='REVIEW',reason='projection_overlap',reader_id=a['id'],conflict=conflict['id']))
        else:final.append(a)
    return [w for row in reading_rows(final) for w in row],decisions

def ownership(words):
    """Source members have one active owner; joint mixed-row output is one unit.
    Reader tokens are consumed once. A crop region may own multiple observed tokens.
    """
    units={};source_owner={};reader_ids=set()
    for word in words:
        members=word.get('source_members',[])
        if word.get('reason') in ('mixed_row_independent_words','complete_row_group_independent_words'):unit='joint:'+digest(sorted(members))[:16]
        elif word.get('residual_member'):unit=word['residual_member']
        else:unit=word['id']
        owner=units.setdefault(unit,dict(unit_id=unit,source_members=[],reader_members=[],output_members=[]))
        for member in members:
            if member in source_owner and source_owner[member]!=unit:raise ValueError('DUPLICATE_SOURCE_OWNERSHIP')
            source_owner[member]=unit
            if member not in owner['source_members']:owner['source_members'].append(member)
        if word['engine']!='apple':
            if word['id'] in reader_ids:raise ValueError('DUPLICATE_READER_CONSUMPTION')
            reader_ids.add(word['id']);owner['reader_members'].append(word['id'])
        owner['output_members'].append(word['id'])
    return list(units.values())

def invariant(snapshot):
    assert snapshot['schema_version']==1
    if digest(snapshot['pages'])!=snapshot['pages_hash']:
        raise ValueError('SNAPSHOT_CORRUPT')
    for p in snapshot['pages']:
        registry=ownership(p.get('words',[]))
        if 'contributions' in p and p['contributions']!=registry:raise ValueError('OWNERSHIP_PROJECTION_MISMATCH')
        ids=[w['id'] for w in p.get('words',[])]
        if len(ids)!=len(set(ids)):raise ValueError('DUPLICATE_CONSUMPTION')
        for w in p.get('words',[]):
            x,y,r,b=w['bbox']
            if not (0<=x<r<=p['width'] and 0<=y<b<=p['height']):raise ValueError('POSITION_BOUNDS')
