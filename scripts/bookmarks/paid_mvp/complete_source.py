"""Explicit candidate observer: source pixels/raw -> words -> lines -> entries.

No prediction, reference titles, expected counts or risk configuration are inputs.
Cached reader bytes are replayed; this candidate never starts a subprocess. Layout
regions are source-image-bound observations, not silently assumed single columns.
Missing continuation evidence stays UNKNOWN. No admission or export authority.
"""
import csv
import io
import math
import re
from pathlib import Path
import fitz
from PIL import Image
from .source_identity import bind_words, physical_lines, compare_readers, ownership, identity, entry_identity
from .source_coverage import digest

VERSION = 'toc-complete-source-v2'
TSV_FIELDS = ('level','page_num','block_num','par_num','line_num','word_num','left','top','width','height','conf','text')
ROMAN = re.compile(r'M{0,3}(?:CM|CD|D?C{0,3})(?:XC|XL|L?X{0,3})(?:IX|IV|V?I{0,3})$',re.I)


def parse_tsv(raw):
    # Quotes are literal OCR characters. Never use CSV quote processing.
    records = list(csv.reader(io.StringIO(raw),delimiter='\t',quoting=csv.QUOTE_NONE))
    if not records or tuple(records[0]) != TSV_FIELDS:raise ValueError('INVALID_TSV_HEADER')
    words=[]
    for index, fields in enumerate(records[1:]):
        if len(fields)!=len(TSV_FIELDS):raise ValueError('INVALID_TSV_FIELDS')
        item=dict(zip(TSV_FIELDS,fields))
        for key in TSV_FIELDS[:10]:
            if not re.fullmatch(r'[0-9]+',item[key]):raise ValueError('INVALID_TSV_INTEGER')
            item[key]=int(item[key])
        conf=float(item['conf'])
        if not math.isfinite(conf) or not -1<=conf<=100:raise ValueError('INVALID_TSV_CONFIDENCE')
        level=item['level'];lower=('block_num','par_num','line_num','word_num')
        if not 1<=level<=5 or item['page_num']!=1:raise ValueError('INVALID_TSV_PAGE_OR_LEVEL')
        if any(item[k]<=0 for k in lower[:level-1]) or any(item[k]!=0 for k in lower[level-1:]):
            raise ValueError('INVALID_TSV_HIERARCHY')
        if any(ord(c)<32 for c in item['text']):raise ValueError('INVALID_TSV_TEXT')
        if level!=5:
            if item['text'] or conf!=-1:raise ValueError('INVALID_TSV_CONTAINER')
            continue
        if conf<0 or item['width']<=0 or item['height']<=0:raise ValueError('INVALID_TSV_WORD')
        if item['text'].strip():words.append(dict(item,conf=conf,raw_record=index))
    return words


def contains(box,other):
    return box[0]<=other[0] and box[1]<=other[1] and other[2]<=box[2] and other[3]<=box[3]


def folio_member(line,zone):
    words=line['words']
    if not words:return None
    last=words[-1];literal=last['text']
    legal=bool(re.fullmatch(r'[0-9]+(?:[-–][0-9]+)?',literal) or literal and ROMAN.fullmatch(literal))
    if not legal or not zone.get('folio_bbox') or not contains(zone['folio_bbox'],last['bbox']):return None
    if len(words)>1:
        gap=last['bbox'][0]-words[-2]['bbox'][2]
        leader=all(c=='.' for c in words[-2]['text'])
        if gap<4 and not leader:return None
    return dict(word_id=last['word_id'],literal=literal,semantic=None,
                interpretation='UNKNOWN_I_OR_ONE' if literal=='I' else 'LITERAL_ONLY',bbox=last['bbox'])


def complete_entries(line_result,zones,relations=(),reader_comparison=None,line_roles=()):
    """Attach only uniquely proved adjacent continuation/author/folio lines.

    Relations contain immutable line IDs and source-layout evidence. No title
    supplied by a model is used. A cross-page link additionally needs both image
    identities, exact endpoint bounds and consecutive source-page head/tail proof.
    """
    all_lines=line_result['lines'];zone_map={(z['page'],z['id']):z for z in zones};headers=[];role_rejections=[]
    if len(zone_map)!=len(zones):raise ValueError('DUPLICATE_SOURCE_PAGE_REGION')
    if len({l['line_id'] for l in all_lines})!=len(all_lines):raise ValueError('DUPLICATE_SOURCE_LINE_IDENTITY')
    def zone_for(line):return zone_map[(line['page'],line['region_id'])]
    roles={r['line_id']:r for r in line_roles}
    if len(roles)!=len(line_roles):raise ValueError('DUPLICATE_SOURCE_LINE_ROLE')
    for line in all_lines:
        role=roles.get(line['line_id'])
        if role is None:continue
        e=role.get('evidence',{});key=line['words'][0]['identity']
        sequence=sorted([l for l in all_lines if l['page']==line['page'] and l['region_id']==line['region_id']],key=lambda l:(l['bbox'][1],l['bbox'][0]))
        isolated=sequence[0] is line and len(sequence)>1 and sequence[1]['bbox'][1]-line['bbox'][3]>=line['bbox'][3]-line['bbox'][1]
        valid=(role.get('role')=='header' and isolated and folio_member(line,zone_for(line)) is None
               and e.get('basis')=='source-pixels' and e.get('source_sha256')==key['source_sha256']
               and e.get('image_sha256')==key['image_sha256'] and bool(e.get('note')))
        if valid:headers.append(dict(line,source_role=role))
        else:role_rejections.append(dict(role=role,reason='source header position or evidence unproved'))
    if set(roles)-{l['line_id'] for l in all_lines}:raise ValueError('UNKNOWN_SOURCE_LINE_ROLE')
    header_ids={h['line_id'] for h in headers};lines=[l for l in all_lines if l['line_id'] not in header_ids]
    by_id={l['line_id']:l for l in lines}
    parent={lid:lid for lid in by_id};incoming={};outgoing={};rejected=[];accepted=[]
    agreement={r['line_id']:r['state'] for r in (reader_comparison or {}).get('primary',[])}
    def root(lid):
        while parent[lid]!=lid:lid=parent[lid]
        return lid
    candidates=[]
    for rel in relations:
        a,b=by_id.get(rel.get('from')),by_id.get(rel.get('to'));e=rel.get('evidence',{})
        reason=None
        if a is None or b is None or a is b:reason='missing or self continuation member'
        elif rel.get('kind') not in ('continuation','author','folio'):reason='unknown relation kind'
        elif a['page']!=b['page']:
            ak=a['words'][0]['identity'];bk=b['words'][0]['identity']
            tail=sorted([l for l in lines if l['page']==a['page'] and l['region_id']==a['region_id']],key=lambda l:(l['bbox'][1],l['bbox'][0]))
            head=sorted([l for l in lines if l['page']==b['page'] and l['region_id']==b['region_id']],key=lambda l:(l['bbox'][1],l['bbox'][0]))
            source_bound=(ak['source_sha256']==bk['source_sha256']==e.get('source_sha256')
                          and e.get('basis')=='source-pixels' and bool(e.get('note'))
                          and e.get('from_image_sha256')==ak['image_sha256']
                          and e.get('to_image_sha256')==bk['image_sha256']
                          and e.get('from_bbox')==a['bbox'] and e.get('to_bbox')==b['bbox']
                          and e.get('from_region')==a['region_id'] and e.get('to_region')==b['region_id'])
            if b['page']!=a['page']+1 or tail[-1] is not a or head[0] is not b:
                reason='cross-page relation lacks consecutive source head/tail'
            elif not source_bound:reason='cross-page source-pixel endpoint evidence unproved'
            elif rel['kind']=='continuation' and folio_member(a,zone_for(a)) is not None:
                reason='completed title cannot absorb next-page title'
        elif a['region_id']!=b['region_id']:reason='cross-region relation unproved'
        else:
            members=a['words']+b['words'];key=members[0]['identity']
            sequence=sorted([l for l in lines if l['page']==a['page'] and l['region_id']==a['region_id']],key=lambda l:(l['bbox'][1],l['bbox'][0]))
            adjacent=sequence.index(b)==sequence.index(a)+1
            close=-1<=b['bbox'][1]-a['bbox'][3]<=max(a['bbox'][3]-a['bbox'][1],b['bbox'][3]-b['bbox'][1])
            if not adjacent or not close:reason='non-adjacent source layout'
            elif e.get('basis')!='source-pixels' or e.get('source_sha256')!=key['source_sha256'] or e.get('image_sha256')!=key['image_sha256'] or not e.get('note'):
                reason='missing source-pixel relation evidence'
        if reason:rejected.append(dict(relation=rel,reason=reason))
        else:
            candidates.append(rel);outgoing.setdefault(rel['from'],[]).append(rel);incoming.setdefault(rel['to'],[]).append(rel)
    # Never choose the first competing link and hide an alternative parent.
    for rel in candidates:
        a,b=rel['from'],rel['to']
        if len(outgoing[a])!=1 or len(incoming[b])!=1 or root(a)==root(b):
            rejected.append(dict(relation=rel,reason='non-unique continuation ownership'));continue
        parent[root(b)]=root(a);accepted.append(rel)
    grouped={}
    for line in lines:grouped.setdefault(root(line['line_id']),[]).append(line)
    entries=[]
    rejected_members={r['relation'].get(k) for r in rejected for k in ('from','to')}
    for members in grouped.values():
        members.sort(key=lambda l:(l['page'],l['bbox'][1],l['bbox'][0]))
        folios=[folio_member(l,zone_for(l)) for l in members];folios=[f for f in folios if f]
        reasons=[]
        if len(folios)!=1:reasons.append('missing or multiple uniquely owned folios')
        if any(l['line_id'] in rejected_members for l in members):reasons.append('continuation ownership unresolved')
        if any(agreement.get(l['line_id'],'AGREEMENT')!='AGREEMENT' for l in members):reasons.append('reader disagreement')
        f=folios[0] if len(folios)==1 else None
        if f and f['interpretation']=='UNKNOWN_I_OR_ONE':reasons.append('literal I semantics unresolved')
        if f and not any(w['word_id']!=f['word_id'] and any(c.isalpha() for c in w['text']) for l in members for w in l['words']):
            reasons.append('orphan folio without complete title')
        ids=[w for l in members for w in l['word_ids']]
        roles={r['to']:('author' if r['kind']=='author' else 'folio' if r['kind']=='folio' else 'title') for r in accepted}
        title_words=[w['text'] for l in members if roles.get(l['line_id'],'title')=='title' for w in l['words'] if not f or w['word_id']!=f['word_id']]
        authors=[l['text'] for l in members if roles.get(l['line_id'])=='author']
        entries.append(dict(entry_id=entry_identity(members),region_id=members[0]['region_id'],page=members[0]['page'],
                            source_pages=sorted({l['page'] for l in members}),
                            source_regions=[dict(page=l['page'],region_id=l['region_id'],line_id=l['line_id']) for l in members],
                            word_ids=ids,line_ids=[l['line_id'] for l in members],members=[dict(l,member_role=roles.get(l['line_id'],'title')) for l in members],
                            title_literal=' '.join(title_words),authors_literal=authors,
                            text=' '.join(l['text'] for l in members),printed_folio=f,
                            relations=[r for r in accepted if r['from'] in {l['line_id'] for l in members}],
                            state='UNKNOWN' if reasons else 'OBSERVED',reasons=reasons))
    groups=[]
    for zone in zones:
        subset=[e for e in entries if e['page']==zone['page'] and e['region_id']==zone['id']]
        region_headers=[h for h in headers if h['page']==zone['page'] and h['region_id']==zone['id']]
        # Region grouping is not a semantic section hierarchy. Keep that limit.
        group_key=dict(region_id=zone['id'],page=zone['page'],source_sha256=zone['source_sha256'],image_sha256=zone['image_sha256'],
                       entry_ids=[e['entry_id'] for e in subset],header_word_ids=[w for h in region_headers for w in h['word_ids']])
        groups.append(dict(group_id=identity(group_key),identity=group_key,region_id=zone['id'],
                           entry_ids=[e['entry_id'] for e in subset],headers=region_headers,
                           kind='source-layout-region',semantic_hierarchy='UNVERIFIED'))
    unresolved=[u['word_id'] for u in line_result['unresolved']]
    all_words=[w for l in all_lines for w in l['word_ids']]+unresolved
    member_audit=ownership(all_words,[e['word_ids'] for e in entries]+[h['word_ids'] for h in headers],unresolved)
    group_audit=ownership([e['entry_id'] for e in entries],[g['entry_ids'] for g in groups])
    return dict(entries=entries,groups=groups,ownership=member_audit,group_ownership=group_audit,
                unresolved=line_result['unresolved'],rejected_relations=rejected,rejected_line_roles=role_rejections,
                complete_identity_ready=bool(entries) and line_result['identity_ready'] and member_audit['state']=='PASS'
                    and group_audit['state']=='PASS' and not rejected and not role_rejections and all(e['state']=='OBSERVED' for e in entries),
                human_checked=False,admission_ready=False)


def verify_image(page,image):
    raw=Path(image['path']).read_bytes()
    if digest(raw)!=image['sha256']:raise ValueError('SOURCE_IMAGE_CHANGED')
    clip=fitz.Rect(image['crop_bbox']) if image['kind']=='crop' else None
    original=Image.open(io.BytesIO(raw)).convert('RGB');width=clip.width if clip else page.rect.width
    dpi=round(original.width*72/width)
    pix=page.get_pixmap(matrix=fitz.Matrix(dpi/72,dpi/72),clip=clip,alpha=False)
    if original.size!=(pix.width,pix.height) or original.tobytes()!=pix.samples:raise ValueError('SOURCE_PAGE_PIXELS_CHANGED')
    return dpi,clip


def cached_words(receipt,page,image,clip,verified_dpi):
    if receipt.get('image_sha256')!=image['sha256'] or receipt.get('returncode')!=0:
        raise ValueError('READER_IMAGE_OR_RESULT_MISMATCH')
    if receipt.get('source_sha256',image['source_sha256'])!=image['source_sha256'] or receipt.get('page_number',image['page_number'])!=image['page_number']:
        raise ValueError('READER_SOURCE_OR_PAGE_MISMATCH')
    if digest(receipt['raw_tsv'].encode())!=receipt['stdout_sha256']:raise ValueError('READER_RAW_CHANGED')
    dpi=receipt.get('reader_render_dpi',verified_dpi)
    if not isinstance(dpi,(int,float)) or not 72<=dpi<=600:raise ValueError('INVALID_READER_DPI')
    pixels=page.get_pixmap(matrix=fitz.Matrix(dpi/72,dpi/72),clip=clip,alpha=False).tobytes('png')
    if dpi!=verified_dpi and digest(pixels)!=receipt.get('reader_render_sha256'):
        raise ValueError('READER_RENDERING_UNBOUND')
    scale=72/dpi;ox=clip.x0 if clip else 0;oy=clip.y0 if clip else 0
    words=[dict(text=w['text'],bbox=[ox+w['left']*scale,oy+w['top']*scale,ox+(w['left']+w['width'])*scale,oy+(w['top']+w['height'])*scale],
                confidence=w['conf'],id=w['raw_record'],raw_hierarchy=[w[k] for k in ('block_num','par_num','line_num','word_num')]) for w in parse_tsv(receipt['raw_tsv'])]
    return words,dict(raw_sha256=receipt['stdout_sha256'],source_pixels_verified=True,new_reader_calls=0,
                      invocation_binary_identity='UNVERIFIED',historical_receipt=receipt)


def observe_complete(request,*,layouts,cached_pages=(),document_relations=()):
    source=Path(request['input_pdf']);source_sha=request['input_sha256']
    if digest(source.read_bytes())!=source_sha:raise ValueError('SOURCE_PDF_CHANGED')
    cached={p['page_number']:p for p in cached_pages}
    if len(cached)!=len(cached_pages):raise ValueError('DUPLICATE_CACHED_PAGE')
    layout_map={l['page']:l for l in layouts}
    if len(layout_map)!=len(layouts):raise ValueError('DUPLICATE_LAYOUT_PAGE')
    numbers=[im['page_number'] for im in request['images']]
    if len(numbers)!=len(set(numbers)):raise ValueError('DUPLICATE_REQUEST_PAGE')
    pages=[];document_lines=[];document_unresolved=[];document_zones=[];document_roles=[];document_links=[];comparisons=[]
    with fitz.open(source) as doc:
        for raw_image in request['images']:
            image=dict(raw_image,source_sha256=source_sha);number=image['page_number'];page=doc[number-1]
            dpi,clip=verify_image(page,image);layout=layout_map.get(number)
            zones=[];relations=[]
            if layout:
                if layout.get('source_sha256')!=source_sha or layout.get('image_sha256')!=image['sha256'] or layout.get('basis')!='source-pixels':
                    raise ValueError('LAYOUT_SOURCE_IDENTITY_MISMATCH')
                evidence=Path(layout['evidence_path'])
                if digest(evidence.read_bytes())!=layout['evidence_sha256']:raise ValueError('LAYOUT_EVIDENCE_CHANGED')
                zones=[dict(z,page=number,source_sha256=source_sha,image_sha256=image['sha256']) for z in layout['regions']]
                for z in zones:
                    box=z['bbox']
                    if len(box)!=4 or not all(math.isfinite(v) for v in box) or box[2]<=box[0] or box[3]<=box[1] or not contains(list(page.rect),box):
                        raise ValueError('INVALID_LAYOUT_REGION')
                    if z.get('folio_bbox') and not contains(box,z['folio_bbox']):raise ValueError('INVALID_FOLIO_REGION')
                relations=layout.get('relations',[])
            cache=cached.get(number,{})
            if cache and (cache.get('source_sha256')!=source_sha or cache.get('image_sha256')!=image['sha256']):
                raise ValueError('CACHED_PAGE_IDENTITY_MISMATCH')
            receipt=cache.get('reader_receipt')
            if receipt:
                words,provenance=cached_words(receipt,page,image,clip,dpi);reader='tsv:'+receipt['stdout_sha256'];method='cached-raw-reader'
            elif cache and cache.get('method')!='native-source-text':
                words=[];reader='unavailable';method='UNKNOWN';provenance=dict(reason='missing cached full-reader raw',new_reader_calls=0)
            else:
                words=[dict(text=w[4],bbox=list(w[:4]),confidence=None,id=i,raw_hierarchy=list(w[5:])) for i,w in enumerate(page.get_text('words',clip=clip))]
                reader='native:'+str(fitz.VersionBind);method='native-source-words';provenance=dict(new_reader_calls=0,native_version=fitz.VersionBind,source_pixels_verified=True)
            bound=bind_words(words,source_sha256=source_sha,image_sha256=image['sha256'],page=number,reader=reader)
            line_result=physical_lines(bound,zones);comparison=None;alternate=cache.get('alternate_reader_receipt')
            if alternate:
                alt_words,alt_proof=cached_words(alternate,page,image,clip,dpi)
                alt_bound=bind_words(alt_words,source_sha256=source_sha,image_sha256=image['sha256'],page=number,reader='tsv:'+alternate['stdout_sha256'])
                alt_lines=physical_lines(alt_bound,zones);comparison=compare_readers(line_result['lines'],alt_lines['lines'])
                comparison.update(provenance=alt_proof,unresolved=alt_lines['unresolved'])
            result=complete_entries(line_result,zones,relations,comparison,(layout or {}).get('line_roles',[]))
            document_lines.extend(line_result['lines']);document_unresolved.extend(line_result['unresolved'])
            document_zones.extend(zones);document_roles.extend((layout or {}).get('line_roles',[]));document_links.extend(relations)
            if comparison:comparisons.extend(comparison['primary'])
            pages.append(dict(page_number=number,source_sha256=source_sha,image_sha256=image['sha256'],method=method,
                              words=bound,lines=line_result['lines'],line_ownership=line_result['ownership'],
                              reader_provenance=provenance,reader_comparison=comparison,layout=layout,**result))
    if digest(source.read_bytes())!=source_sha:raise ValueError('SOURCE_CHANGED_DURING_OBSERVATION')
    document_line_audit=ownership([w['word_id'] for p in pages for w in p['words']],
                                  [l['word_ids'] for l in document_lines],[u['word_id'] for u in document_unresolved])
    document=complete_entries(dict(lines=document_lines,unresolved=document_unresolved,
                                  identity_ready=not document_unresolved and document_line_audit['state']=='PASS'),
                              document_zones,document_links+list(document_relations),
                              dict(primary=comparisons),document_roles)
    return dict(schema='toc-complete-source/1',observer_version=VERSION,source_sha256=source_sha,pages=pages,
                document=document,document_line_ownership=document_line_audit,
                new_reader_calls=0,network_sent=False,admission_ready=False,
                limitation='Explicit source-layout candidate; counts alone do not certify source completeness or natural safety')
