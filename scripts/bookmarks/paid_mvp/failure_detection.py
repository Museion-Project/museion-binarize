"""Frozen semantic admission. No book names, reference paths or evaluator imports."""
import copy
import hashlib
import json
import re
from difflib import SequenceMatcher

CONFIG=dict(version='toc-admission-v2',epoch=2,severe_missing_fraction=.2,match_ratio=.64,ownership_ratio=.92,min_regions=2)


def hash_value(value):return hashlib.sha256(json.dumps(value,sort_keys=True,ensure_ascii=False).encode()).hexdigest()


def norm(text):return ' '.join(re.findall(r'\w+',text.casefold()))


def decide(rows,probe,source_sha256,raw_sha256,revision=0):
    if probe.get('source_sha256')!=source_sha256:raise ValueError('probe source mismatch')
    risks=[]; pages=[]; missing=total=0; unknown=0
    for page in probe['pages']:
        number=page['page_number']; observed=page['regions']; selected=[r for r in rows if r['source_page']==number]
        used=set(); uncovered=[]; ownership=[]
        for region in observed:
            text=norm(region['text']); scores=[SequenceMatcher(None,text,norm(r['title']+' '+(r.get('printed_page') or ''))).ratio() for r in selected]
            best=max(scores,default=0);index=scores.index(best) if scores else None
            if best>=CONFIG['match_ratio'] and index not in used:used.add(index)
            else:
                uncovered.append(region)
                if any(SequenceMatcher(None,text,norm(r['title']+' '+(r.get('printed_page') or ''))).ratio()>=CONFIG['ownership_ratio'] for r in rows if r['source_page']!=number):ownership.append(region)
        n=len(observed);total+=n;missing+=len(uncovered)
        status='unknown' if n<CONFIG['min_regions'] else 'missing' if uncovered else 'covered'
        if status=='unknown':unknown+=1
        if n>=CONFIG['min_regions'] and not selected:risks.append(dict(code='entire_source_page_missing',page_number=number,severity='severe'))
        if ownership:risks.append(dict(code='source_page_ownership_conflict',page_number=number,severity='severe',regions=ownership))
        pages.append(dict(page_number=number,status=status,observed_regions=n,covered_regions=len(used),missing_regions=uncovered,image_sha256=page['image_sha256'],readability=page['readability']))
    seen=set()
    for row in rows:
        key=(norm(row['title']),row.get('printed_page'))
        if key in seen:risks.append(dict(code='duplicate_source_coverage',row=row['raw_index'],severity='severe'))
        seen.add(key)
        c=row.get('continuation_of')
        if c is not None and (row.get('printed_page') and rows[c].get('printed_page') and row['printed_page']!=rows[c]['printed_page']):risks.append(dict(code='continuation_page_conflict',row=row['raw_index'],severity='severe'))
        if row.get('parent_index') is not None and rows[row['parent_index']]['source_page']>row['source_page']:risks.append(dict(code='source_order_conflict',row=row['raw_index'],severity='severe'))
    fraction=missing/total if total else None
    if fraction is not None and fraction>=CONFIG['severe_missing_fraction']:risks.append(dict(code='large_source_region_gap',severity='severe',fraction=fraction))
    severe=any(r['severity']=='severe' for r in risks)
    # Insufficient independent coverage cannot be made safe by model self reporting.
    decision='abstain' if severe or unknown else 'review' if missing else 'accept-draft'
    if unknown:risks.append(dict(code='independent_coverage_unknown',severity='unknown',page_count=unknown))
    result=dict(schema='toc-admission-decision/1',decision=decision,export_locked=decision in ('abstain','failed'),alert='SEVERE_TOC_EXTRACTION_RISK' if severe else 'TOC_COVERAGE_UNKNOWN' if unknown else None,pages=pages,risks=risks,config=copy.deepcopy(CONFIG),config_sha256=hash_value(CONFIG),source_sha256=source_sha256,raw_model_sha256=raw_sha256,probe_sha256=hash_value(probe),revision=revision,human_checked=False)
    result['decision_sha256']=hash_value(result)
    return result


def tree_hash(table):
    fields=('id','title','parent','level','printed_page','source_page','source_rows','evidence_ids','toc_group')
    return hash_value(dict(entries=[{k:e.get(k) for k in fields} for e in table['entries']],images=table.get('images'),operation_id=table.get('operation_id'),source_sha256=table['source_sha256'],page_count=table.get('page_count')))


def seal_tree(decision,table):
    decision['candidate_sha256']=tree_hash(table)
    decision.pop('decision_sha256',None)
    decision['decision_sha256']=hash_value(decision)
    return decision


def verify(table, *, allow_legacy=False):
    guard=table.get('admission')
    if not guard:
        if allow_legacy:return # Explicit historical offline call only.
        raise ValueError('guarded admission required; absent fields never select historical mode')
    check=copy.deepcopy(guard);saved=check.pop('decision_sha256',None)
    if saved!=hash_value(check) or guard['config_sha256']!=hash_value(CONFIG) or guard['config']!=CONFIG:raise ValueError('admission configuration or decision changed; new source-bound decision required')
    if guard['source_sha256']!=table['source_sha256'] or guard['raw_model_sha256']!=table['raw_model_sha256'] or guard['revision']!=table['revision']:raise ValueError('stale admission revision/source/raw')
    if guard.get('candidate_sha256')!=tree_hash(table):raise ValueError('candidate semantic identity changed')
    if guard['decision'] in ('abstain','failed') or guard['export_locked']:raise ValueError('SEVERE_TOC_EXPORT_LOCK: no saveable tree')
