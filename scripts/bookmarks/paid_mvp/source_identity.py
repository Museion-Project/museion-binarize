"""Source-only identity primitives for a candidate observer, not an admission gate.

No reference counts or titles enter these functions. Words are immutable reader
observations; columns/regions must be source-bound. Ambiguous words/alternatives
remain unresolved. Do not equate successful ownership accounting with OCR quality.
"""
from collections import Counter
import hashlib
import json
import math
from statistics import median

VERSION='toc-source-identity-v1'


def identity(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,ensure_ascii=False,allow_nan=False).encode()).hexdigest()


def bind_words(words, *, source_sha256, image_sha256, page, reader):
    result=[];seen=set()
    for index, word in enumerate(words):
        box=word['bbox']
        if len(box)!=4 or not all(math.isfinite(v) for v in box) or box[2]<=box[0] or box[3]<=box[1]:
            raise ValueError('INVALID_OBSERVED_WORD_GEOMETRY')
        reader_id=word.get('source_word_id',word.get('id',index))
        if reader_id in seen:raise ValueError('DUPLICATE_READER_MEMBER')
        seen.add(reader_id)
        key=dict(source_sha256=source_sha256,image_sha256=image_sha256,page=page,reader=reader,
                 reader_id=reader_id,text=word['text'],bbox=list(box))
        result.append(dict(word,word_id=identity(key),identity=key,precision='observed word'))
    return result


def ownership(expected, groups, unresolved=()):
    """Multiplicity detects duplication within or across groups and alternatives."""
    baseline=Counter(expected)
    if any(n!=1 for n in baseline.values()):
        raise ValueError('DUPLICATE_INPUT_IDENTITY')
    represented=Counter(m for group in groups for m in group)+Counter(unresolved)
    return dict(state='PASS' if represented==baseline else 'FAIL',
                missing=list((baseline-represented).elements()),
                duplicate_or_unknown=list((represented-baseline).elements()),
                unresolved=list(unresolved),total_members=len(expected))


def physical_lines(words, regions):
    """Never merge same-y words in different declared source regions.

    No region means no ownership proof, not an implicit full-page single column.
    Tall/superscript/overlapping-region ambiguities remain individually unresolved.
    """
    by_region={r['id']:[] for r in regions}
    if len(by_region)!=len(regions):
        raise ValueError('DUPLICATE_SOURCE_REGION')
    unresolved=[]
    for word in words:
        x,y,r,b=word['bbox']
        zones=[zone for zone in regions if zone['bbox'][0]<=x and r<=zone['bbox'][2]
               and zone['bbox'][1]<=y and b<=zone['bbox'][3]
               and zone.get('page')==word['identity']['page']
               and zone.get('source_sha256')==word['identity']['source_sha256']
               and zone.get('image_sha256')==word['identity']['image_sha256']]
        if len(zones)!=1:
            unresolved.append(dict(word_id=word['word_id'],reason='source region missing or ambiguous'))
        else:
            by_region[zones[0]['id']].append(word)
    lines=[]
    for region, members in by_region.items():
        groups=[]
        for word in sorted(members,key=lambda w:(w['bbox'][3],w['bbox'][0])):
            tolerance=max(1,min(3,(word['bbox'][3]-word['bbox'][1])*.4))
            choices=[g for g in groups if all(abs(w['bbox'][3]-word['bbox'][3])<=tolerance for w in g)]
            if len(choices)>1:
                unresolved.append(dict(word_id=word['word_id'],reason='physical baseline ambiguous'))
            elif choices:
                choices[0].append(word)
            else:
                groups.append([word])
        for group in groups:
            group.sort(key=lambda w:w['bbox'][0]);ids=[w['word_id'] for w in group]
            box=[min(w['bbox'][0] for w in group),min(w['bbox'][1] for w in group),
                 max(w['bbox'][2] for w in group),max(w['bbox'][3] for w in group)]
            lines.append(dict(line_id=identity(ids),region_id=region,word_ids=ids,words=group,
                              text=' '.join(w['text'] for w in group),bbox=box,
                              page=group[0]['identity']['page']))
    lines.sort(key=lambda l:(l['page'],l['region_id'],l['bbox'][1],l['bbox'][0]))
    audit=ownership([w['word_id'] for w in words],[l['word_ids'] for l in lines],
                    [u['word_id'] for u in unresolved])
    return dict(schema=VERSION,lines=lines,unresolved=unresolved,ownership=audit,
                identity_ready=not unresolved and audit['state']=='PASS')


def compare_readers(primary, alternate):
    """Unique 2D exact agreement only. Alternative-only rows stay alternatives."""
    def eligible(a,b):
        if a['page']!=b['page'] or a['region_id']!=b['region_id']:
            return False
        if not a.get('words') or not b.get('words'):
            return False
        first=a['words'][0]['identity'];second=b['words'][0]['identity']
        if any(first[k]!=second[k] for k in ('source_sha256','image_sha256','page')):
            return False
        x,y,r,d=a['bbox'];xx,yy,rr,dd=b['bbox']
        overlap=max(0,min(r,rr)-max(x,xx))*max(0,min(d,dd)-max(y,yy))
        return overlap/max(1e-9,min((r-x)*(d-y),(rr-xx)*(dd-yy)))>=.92
    graph={i:[j for j,b in enumerate(alternate) if eligible(a,b)] for i,a in enumerate(primary)}
    reverse=Counter(j for choices in graph.values() for j in choices)
    rows=[];used=set()
    for i,a in enumerate(primary):
        choices=graph[i]
        exact=len(choices)==1 and reverse[choices[0]]==1 and a['text']==alternate[choices[0]]['text']
        if exact:used.add(choices[0])
        rows.append(dict(line_id=a['line_id'],state='AGREEMENT' if exact else 'UNKNOWN',
                         alternatives=[alternate[j] for j in choices],
                         reason=None if exact else 'reader text or ownership disagreement'))
    return dict(primary=rows,alternative_only=[b for j,b in enumerate(alternate) if j not in used],
                adds_formal_entries=False)


def entry_identity(lines):
    """Use immutable member IDs, never a normalized or corrected title."""
    ids=[w for line in lines for w in line['word_ids']]
    if len(ids)!=len(set(ids)):
        raise ValueError('DUPLICATE_ENTRY_WORD')
    if not ids:
        raise ValueError('EMPTY_SOURCE_ENTRY')
    return identity(ids)
