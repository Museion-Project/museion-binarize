"""Explicit complete-member admission candidate; formal v2 stays unchanged.

Only immutable source-observer entries and source-member-bound prediction rows
enter this function. No evaluator counts/titles or known book names are inputs.
Mechanical candidate decisions grant no export, natural-quality or App admission.
"""
from collections import Counter
import copy
from difflib import SequenceMatcher

from .failure_detection import CONFIG,hash_value,norm


def decide_complete(rows,source,source_sha256,raw_sha256,revision=0):
    if source.get('source_sha256')!=source_sha256:raise ValueError('COMPLETE_PROBE_SOURCE_MISMATCH')
    document=source.get('document')
    if not document or document['ownership']['state']!='PASS' or document['group_ownership']['state']!='PASS':
        raise ValueError('COMPLETE_PROBE_OWNERSHIP_UNPROVED')
    entries=document['entries'];by_id={e['entry_id']:e for e in entries}
    if len(by_id)!=len(entries):raise ValueError('DUPLICATE_COMPLETE_SOURCE_ENTRY')
    ids=[r['raw_index'] for r in rows]
    if len(ids)!=len(set(ids)):raise ValueError('DUPLICATE_PREDICTION_ROW_IDENTITY')
    rows_by_id={r['raw_index']:r for r in rows}
    known={e['entry_id']:e for e in entries if e['state']=='OBSERVED' and e.get('printed_folio')}
    matched={};unresolved=[];risks=[];observations=[];opaque_prediction=False;unknown_coverage=set()
    for row in rows:
        binding=row.get('source_member_binding');entry=by_id.get((binding or {}).get('entry_id'))
        if not binding or not entry:
            unresolved.append(dict(raw_index=row['raw_index'],reason='complete source-member binding absent'))
            opaque_prediction=True
            continue
        if (binding.get('source_sha256')!=source_sha256 or binding.get('raw_model_sha256')!=raw_sha256
            or Counter(binding.get('word_ids',[]))!=Counter(entry['word_ids'])):
            unresolved.append(dict(raw_index=row['raw_index'],reason='incomplete or stale source-member binding'))
            opaque_prediction=True
            risks.append(dict(code='incomplete_source_member_ownership',severity='unknown',raw_index=row['raw_index']))
            continue
        if row['source_page']!=entry['page']:
            risks.append(dict(code='source_page_ownership_conflict',severity='severe',raw_index=row['raw_index'],entry_id=entry['entry_id']))
            continue
        if entry['entry_id'] in matched:
            risks.append(dict(code='duplicate_source_coverage',severity='severe',raw_index=row['raw_index'],entry_id=entry['entry_id']))
            continue
        folio=entry.get('printed_folio');ratio=SequenceMatcher(None,norm(entry['title_literal']),norm(row['title'])).ratio()
        eligible=(entry['entry_id'] in known and ratio>=CONFIG['match_ratio']
                  and row.get('printed_page')==folio['literal'])
        observations.append(dict(raw_index=row['raw_index'],entry_id=entry['entry_id'],title_match_ratio=ratio,
                                 complete_member_binding=True,literal_folio_equal=bool(folio and row.get('printed_page')==folio['literal']),
                                 state='KNOWN_VALID' if eligible else 'UNKNOWN'))
        if eligible:matched[entry['entry_id']]=row['raw_index']
        else:
            unresolved.append(dict(raw_index=row['raw_index'],entry_id=entry['entry_id'],reason='source title/folio or semantic identity unresolved'))
            if entry['entry_id'] in known:unknown_coverage.add(entry['entry_id'])
    if opaque_prediction:unknown_coverage.update(set(known)-set(matched))
    unknown_coverage.difference_update(matched)
    # Only a fully source-bound prediction inventory can prove absence. An
    # opaque prediction might cover an unmatched known entry: retain UNKNOWN,
    # rather than manufacturing a catastrophic omission from adapter failure.
    missing=[eid for eid in known if eid not in matched and eid not in unknown_coverage]
    total=len(known);fraction=len(missing)/total if total else None
    # UNKNOWN source and unbound model rows can never enlarge this denominator.
    unknown_source=[e['entry_id'] for e in entries if e['entry_id'] not in known]
    unknown_members=list(document.get('unresolved',[]))
    # Page evidence also protects persisted pre-v3 candidate results whose
    # document aggregate dropped alternate-only rows or unresolved members.
    reader_alternatives=[];reader_unresolved=[];seen_alternatives=set();seen_members=set()
    def retain(records,target,seen):
        for record in records:
            key=hash_value(record)
            if key not in seen:
                target.append(copy.deepcopy(record));seen.add(key)
    retain(document.get('reader_alternatives',[]),reader_alternatives,seen_alternatives)
    retain(document.get('reader_unresolved_members',[]),reader_unresolved,seen_members)
    for page in source['pages']:
        comparison=page.get('reader_comparison') or {}
        retain(comparison.get('alternative_only',[]),reader_alternatives,seen_alternatives)
        retain(comparison.get('unresolved',[]),reader_unresolved,seen_members)
    if reader_alternatives or reader_unresolved:
        risks.append(dict(code='independent_reader_coverage_unknown',severity='unknown'))
    if fraction is not None and fraction>=CONFIG['severe_missing_fraction']:
        risks.append(dict(code='large_source_region_gap',severity='severe',fraction=fraction))
    for page in source['pages']:
        page_known=[eid for eid,e in known.items() if e['page']==page['page_number']]
        if len(page_known)>=CONFIG['min_regions'] and all(eid in missing for eid in page_known):
            risks.append(dict(code='entire_source_page_missing',severity='severe',page_number=page['page_number']))
    for group in document.get('groups',[]):
        group_known=[eid for eid in group['entry_ids'] if eid in known]
        if len(group_known)>=CONFIG['min_regions'] and all(eid in missing for eid in group_known):
            risks.append(dict(code='entire_source_group_missing',severity='severe',group_id=group['group_id']))
    # Retain the original structural risk labels. Indices are immutable raw
    # identities, not positions in a filtered/reordered list.
    for row in rows:
        continuation=row.get('continuation_of');parent=row.get('parent_index')
        if continuation is not None:
            previous=rows_by_id.get(continuation)
            if previous is None:
                unresolved.append(dict(raw_index=row['raw_index'],reason='continuation row identity absent'))
            elif row.get('printed_page') and previous.get('printed_page') and row['printed_page']!=previous['printed_page']:
                risks.append(dict(code='continuation_page_conflict',severity='severe',raw_index=row['raw_index']))
        if parent is not None:
            previous=rows_by_id.get(parent)
            if previous is None:
                unresolved.append(dict(raw_index=row['raw_index'],reason='parent row identity absent'))
            elif previous['source_page']>row['source_page']:
                risks.append(dict(code='source_order_conflict',severity='severe',raw_index=row['raw_index']))
    identity_complete=document.get('complete_identity_ready') is True
    if not identity_complete or unknown_source or unknown_members or unresolved or not total:
        risks.append(dict(code='independent_complete_identity_unknown',severity='unknown'))
    severe=any(r['severity']=='severe' for r in risks)
    decision='abstain' if severe else 'review' if missing or any(r['severity']=='unknown' for r in risks) else 'accept-draft'
    result=dict(schema='toc-complete-member-decision/1',candidate_version='complete-member-admission-v2',
                decision=decision,export_locked=True,export_authority='CANDIDATE_ONLY',
                alert='SEVERE_TOC_EXTRACTION_RISK' if severe else 'TOC_COVERAGE_UNKNOWN' if decision=='review' else None,
                source_sha256=source_sha256,
                raw_model_sha256=raw_sha256,revision=revision,config=copy.deepcopy(CONFIG),config_sha256=hash_value(CONFIG),
                source_probe_sha256=hash_value(source),known_units=total,known_valid=len(matched),
                known_missing=len(missing),known_missing_entry_ids=missing,known_missing_fraction=fraction,
                known_coverage_unknown=len(unknown_coverage),unknown_coverage_entry_ids=sorted(unknown_coverage),
                unknown_source_entry_ids=unknown_source,unknown_source_members=unknown_members,
                unknown_reader_alternatives=reader_alternatives,unknown_reader_members=reader_unresolved,
                unresolved_predictions=unresolved,observations=observations,risks=risks,
                human_checked=False,admission_ready=False,quality_ready=False,natural_safety_verified=False,
                limitation='Explicit source-member contract candidate; no formal engine/export integration or natural quality proof')
    result['decision_sha256']=hash_value(result)
    return result
