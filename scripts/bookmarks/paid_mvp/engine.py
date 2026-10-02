"""Image-selected contents -> immutable observations -> editable bookmark DTO.
No body-derived TOC, no offset guessing. PDF indices in DTO are zero based.
"""
import argparse
import base64
import copy
import hashlib
import importlib.util
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import fitz

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
LOCAL = HERE.parent / 'local'
# Load the existing writer by explicit path; do not change another branch's code.
spec = importlib.util.spec_from_file_location('paid_contents_local_writer', LOCAL / 'bookmarks.py')
writer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(writer)
DESTINATION = 'https://api.openai.com/v1/responses'
MODEL = 'gpt-6-luna'
CONFIG = 'paid-contents-v1'
PROMPT = '''Extract only the actual printed table of contents from the selected source images. Return JSON only: {"entries":[{"title":string,"printed_page":string|null,"level":integer,"parent_index":integer|null,"source_page":integer,"toc_group":string,"continuation_of":integer|null}],"uncertainties":[string]}. Levels are 1-based and parent_index/continuation_of are 0-based row indices; parents and continued rows must precede their dependent row. source_page is the selected one-based PDF page printed before each image. Keep source row order. Preserve exact original wording, authors, numbering, Roman/Arabic page labels and ranges; do not expand abbreviated ranges. A continued title/author line must be explicitly linked via continuation_of, retaining its text and page reference; never discard author rows. A new TOC group has independent roots. Exclude Contents and running headers. Use indentation/typography/numbering for hierarchy. No invented titles, external knowledge, body-derived TOC or PDF destination guesses. Report ambiguity in uncertainties.'''

def load(path):
    return json.loads(Path(path).read_text())


def sha(path):
    return writer.sha(path)


def put(path, value):
    """New immutable artifact; never overwrite an earlier candidate."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
        handle.write('\n')
    return str(path.resolve())


def readiness():
    blockers = [dict(code='quality_unverified', message='Six-source recall, scan target audit and five local-error corrections remain incomplete'),
                dict(code='live_budget_checkpoint', message='No new live call until coordinator reconciles shared C+D reservations'),
                dict(code='app_consumer_unverified', message='Desktop GUI and packaged runtime not verified')]
    result = dict(schema_version=1, mode='paid-contents', component_ready=True,
                local_runtime_ready=True, app_ready=False, distribution_ready=False,
                quality_ready=False, live_verified=False, ready=False, blockers=blockers,
                dependencies=[dict(name='PyMuPDF', version=fitz.VersionBind, path=str(Path(fitz.__file__).resolve()), available=True),
                              dict(name='local bookmark writer', version='mpdf-bookmark-table/1', path=str(LOCAL / 'bookmarks.py'), available=True)],
                evidence=[])
    state_path=ROOT/'docs/evidence/ocr-mvp-app-preparation-2026-09-30/D/final-readiness.json'
    if state_path.is_file():
        state=load(state_path)
        if state.get('engine_sha256')==sha(__file__):
            result.update({k:v for k,v in state.items() if k in result})
    return result


def validate_request(request):
    for key in ('operation_id', 'input_pdf', 'input_sha256', 'page_numbers', 'mode', 'output_directory', 'config_version'):
        if key not in request:
            raise ValueError('missing field: ' + key)
    if request['mode'] != 'paid-contents' or request['config_version'] != CONFIG:
        raise ValueError('explicit mode/config version required')
    if not isinstance(request['operation_id'], str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', request['operation_id']):
        raise ValueError('invalid operation identity')
    source = Path(request['input_pdf']).resolve()
    if sha(source) != request['input_sha256']:
        raise ValueError('source hash mismatch')
    with fitz.open(source) as doc:
        if doc.is_encrypted:
            raise ValueError('encrypted source unsupported')
        pages = request['page_numbers']
        if not isinstance(pages, list) or not pages or len(pages)>40 or pages != sorted(set(pages)) or any(type(p) is not int or not 1<=p<=len(doc) for p in pages):
            raise ValueError('invalid selected contents pages')
        count = len(doc)
    images = request.get('images', [])
    if len(images) != len(pages) or [i['page_number'] for i in images] != pages:
        raise ValueError('exact one selected image per contents page required')
    for image in images:
        if sha(image['path']) != image['sha256']:
            raise ValueError('selected image changed')
        if image.get('source_sha256') != request['input_sha256']:
            raise ValueError('selected image/source binding missing')
        if image.get('kind') not in ('full-page', 'crop'):
            raise ValueError('image kind required')
        if image['kind']=='crop' and not image.get('crop_bbox'):
            raise ValueError('crop requires source coordinates')
    return source, count


def preflight(request):
    source, count = validate_request(request)
    return dict(schema_version=1, mode='paid-contents', operation_id=request['operation_id'],
                source=str(source), source_sha256=request['input_sha256'], page_count=count,
                selected_pages=request['page_numbers'], images=copy.deepcopy(request['images']),
                destination=DESTINATION, model=MODEL, request_count=1,
                prompt_sha256=hashlib.sha256(PROMPT.encode()).hexdigest(),
                max_output_tokens=10000, max_input_tokens=20000,
                worst_case_usd=.007, reservation_usd=.02,
                cost_basis='Frozen pilot prices: input $0.10/M, output $0.50/M; reserve $0.02. At most 20000 input/10000 output tokens.',
                pending=['exact upload scope', 'coordinator C+D reconciliation', 'remaining original USD1 budget'],
                network_sent=False)


def observations(prediction, request):
    if not isinstance(prediction, dict) or set(prediction)-{'entries','uncertainties'} or not isinstance(prediction.get('entries'), list) or not prediction['entries'] or len(prediction['entries'])>4000:
        raise ValueError('invalid model contents envelope')
    if not isinstance(prediction.get('uncertainties', []), list) or any(not isinstance(v,str) for v in prediction.get('uncertainties', [])):
        raise ValueError('invalid uncertainties')
    allowed = {'title','printed_page','level','parent_index','source_page','toc_group','continuation_of'}
    rows = []
    for index, entry in enumerate(prediction['entries']):
        if not isinstance(entry, dict) or set(entry)-allowed or not {'title','printed_page','level','parent_index'}<=set(entry):
            raise ValueError('unknown model row field')
        if not isinstance(entry.get('title'), str) or not entry['title'].strip() or any(ord(c)<32 for c in entry['title']):
            raise ValueError('invalid title')
        if entry.get('printed_page') is not None and not isinstance(entry['printed_page'], str):
            raise ValueError('invalid printed page')
        if type(entry.get('level')) is not int or not 1<=entry['level']<=32:
            raise ValueError('invalid level')
        for key in ('parent_index','continuation_of'):
            value = entry.get(key)
            if value is not None and (type(value) is not int or not 0<=value<index):
                raise ValueError('cyclic/forward/dangling ' + key)
        e = copy.deepcopy(entry)
        if e.get('source_page') is None:
            if len(request['page_numbers']) != 1:
                raise ValueError('multi-page row requires source page')
            e['source_page'] = request['page_numbers'][0]
            e['source_page_binding'] = 'single-selected-page deterministic binding'
        if type(e['source_page']) is not int or e['source_page'] not in request['page_numbers']:
            raise ValueError('row outside selected contents pages')
        e.setdefault('toc_group', 'main')
        if not isinstance(e['toc_group'],str) or not e['toc_group']:
            raise ValueError('invalid group')
        for key in ('parent_index','continuation_of'):
            if e.get(key) is not None and rows[e[key]]['toc_group'] != e['toc_group']:
                raise ValueError('cross-group relationship')
        e['raw_index'] = index
        e['id'] = 'luna-' + hashlib.sha256((request['input_sha256']+':'+str(index)+':'+json.dumps(entry,sort_keys=True)).encode()).hexdigest()[:20]
        rows.append(e)
    return rows


def candidate_tree(prediction, request):
    rows = observations(prediction, request)
    entries, corrections, stack, row_nodes = [], [], [], {}
    previous_group = None
    closed_groups = set()
    for row in rows:
        idx = row['raw_index']
        group = row['toc_group']
        if group != previous_group:
            if group in closed_groups:
                raise ValueError('interleaved contents groups')
            if previous_group is not None:
                closed_groups.add(previous_group)
            stack = []
            previous_group = group
        continuation = row.get('continuation_of')
        if continuation is not None:
            target = row_nodes[continuation]
            # Never erase raw rows or infer a continuation from a similar title.
            target['title'] += ' ' + row['title']
            target['evidence_ids'].append(row['id'])
            target['source_rows'].append(idx)
            target['review_reasons'].append('continuation_requires_source_review')
            label = row.get('printed_page')
            if label and target['printed_page'] and label != target['printed_page']:
                target['review_reasons'].append('continued_page_reference_conflict')
                target['continued_page_references'].append(label)
            elif label and not target['printed_page']:
                target['printed_page'] = label
                parsed = writer.number(label)
                target['printed_family'], target['printed_value'] = parsed or (None,None)
            row_nodes[idx] = target
            corrections.append(dict(kind='explicit_continuation_candidate', raw_row=idx, target_id=target['id'], accepted=False))
            continue
        level = row['level']-1
        reasons = ['source_title_page_review_required','pagination_uninspected']
        while len(stack)>level:
            stack.pop()
        if level>len(stack):
            # Local flattening is an explicit pending candidate, never model success.
            corrections.append(dict(kind='level_gap_candidate', raw_row=idx, raw_level=row['level'], candidate_level=len(stack)+1, accepted=False))
            level = len(stack)
            reasons.append('hierarchy_level_gap')
        parent = stack[-1] if level else None
        expected_idx = parent['raw_index'] if parent else None
        if row.get('parent_index') != expected_idx:
            corrections.append(dict(kind='parent_level_conflict', raw_row=idx, title=row['title'], raw_parent_index=row.get('parent_index'), parent_from_levels=expected_idx, accepted=False))
            reasons.append('hierarchy_parent_level_conflict')
        parsed = writer.number(row['printed_page']) if row.get('printed_page') else None
        entry = dict(id=row['id'], title=row['title'], parent=parent['id'] if parent else None, level=level,
                     printed_page=row.get('printed_page'), printed_family=parsed[0] if parsed else None,
                     printed_value=parsed[1] if parsed else None, target_pdf_page=None,
                     section_label=None, source_page=row['source_page']-1, source_bbox=None,
                     evidence_ids=[row['id']], source_rows=[idx], toc_group=group,
                     hierarchy_reason='pending model levels candidate; raw parent preserved separately',
                     review_reasons=reasons, state='needs_review', continued_page_references=[], target_evidence=[])
        if any(v['title']==entry['title'] and v['printed_page']==entry['printed_page'] and v['toc_group']==group for v in entries):
            entry['review_reasons'].append('duplicate_candidate_preserved')
        entries.append(entry)
        row_nodes[idx] = entry
        stack.append(row)
    return entries, corrections, rows


def native_targets(source, excluded):
    """Exact publisher labels or measured original native folios, no offsets."""
    results = []
    digest=sha(source)
    with fitz.open(source) as doc:
        for page in doc:
            if page.number in excluded:
                continue
            label = page.get_label()
            parsed = writer.number(label)
            if parsed:
                results.append(dict(family=parsed[0],printed_value=parsed[1],pdf_page=page.number,
                                    kind='publisher_page_label',text=label,evidence_ref=f'pdf-label:{page.number}:{label}',source_sha256=digest))
        # Import existing observed-margin detector without global sys.path changes.
        old = sys.modules.get('bookmarks')
        sys.modules['bookmarks'] = writer
        try:
            pspec=importlib.util.spec_from_file_location('paid_contents_pagination', LOCAL/'pagination.py')
            pagination=importlib.util.module_from_spec(pspec)
            pspec.loader.exec_module(pagination)
        finally:
            if old is None:
                del sys.modules['bookmarks']
            else:
                sys.modules['bookmarks']=old
        candidates=[]
        for page in doc:
            if page.number in excluded:
                continue
            candidates.extend(pagination.candidates_from_words(page.get_text('words'),page.rect.width,page.rect.height,page.number,'native_margin',f'native-margin:{page.number}'))
        # Sequence is used to remove spurious observed numerals, never extrapolate targets.
        model=pagination.build_model(candidates,len(doc),excluded)
        for item in model['observations']:
            results.append(dict(item,source_sha256=digest))
    return results


def checked_scan_targets(request, path):
    """Scan anchors must be real pixel observations with exact image binding.
    No inferred segment/offset records are accepted. AI and human audit stay separate.
    """
    data=load(path)
    if data.get('source_sha256') != request['input_sha256']:
        raise ValueError('scan pagination source mismatch')
    anchors=[]
    for item in data.get('observations',[]):
        if item.get('kind')!='scan_margin_observation' or item.get('review_status') not in ('ai_source_checked','human_source_checked'):
            raise ValueError('scan target needs source-checked pixel observation')
        if sha(item['image_path']) != item['image_sha256'] or not item.get('bbox') or not item.get('reviewer'):
            raise ValueError('scan evidence not image-bound')
        parsed=writer.number(item['text'])
        if not parsed or parsed != (item['family'],item['printed_value']):
            raise ValueError('scan observed numeral mismatch')
        if type(item['pdf_page']) is not int or item['pdf_page'] in [p-1 for p in request['page_numbers']]:
            raise ValueError('contents cannot locate itself')
        if item.get('renderer')!='pymupdf' or item.get('dpi')!=150:
            raise ValueError('scan image renderer/dpi evidence required')
        with fitz.open(request['input_pdf']) as doc:
            if not 0<=item['pdf_page']<len(doc):raise ValueError('scan target outside PDF')
            page=doc[item['pdf_page']]
            rendered=page.get_pixmap(matrix=fitz.Matrix(150/72,150/72),alpha=False).tobytes('png')
            if hashlib.sha256(rendered).hexdigest()!=item['image_sha256']:
                raise ValueError('scan image does not match source PDF page pixels')
            x,y,w,h=item['bbox']
            if w<=0 or h<=0 or x<0 or y<0 or x+w>page.rect.width or y+h>page.rect.height:
                raise ValueError('scan folio bbox outside page')
        anchors.append(dict(item,source_sha256=request['input_sha256']))
    return anchors


def bind_targets(table, anchors):
    for entry in table['entries']:
        matched=[a for a in anchors if (a['family'],a['printed_value'])==(entry['printed_family'],entry['printed_value']) and 0<=a['pdf_page']<table['page_count']]
        targets={a['pdf_page'] for a in matched}
        entry['target_candidates']=matched
        if len(targets)==1:
            entry['target_pdf_page']=next(iter(targets))
            entry['target_evidence']=matched
            entry['review_reasons']=[r for r in entry['review_reasons'] if r!='pagination_uninspected']
            entry['review_reasons'].append('target_title_position_review_required')
        elif len(targets)>1:
            entry['review_reasons'].append('pagination_ambiguous')
    return table


def draft(request, prediction_path, provenance='cached', scan_path=None, guarded=True):
    source,count=validate_request(request)
    if provenance not in ('cached','mock','live'):
        raise ValueError('explicit evidence provenance required')
    root=Path(request['output_directory']).resolve()
    root.mkdir(parents=True,exist_ok=True)
    raw=Path(prediction_path).read_bytes()
    with (root/'raw-model.json').open('xb') as f:
        f.write(raw)
    pred=json.loads(raw)
    try:
        entries,corrections,rows=candidate_tree(pred,request)
    except ValueError as exc:
        put(root/'completion.json',dict(schema_version=1,mode='paid-contents',operation_id=request['operation_id'],status='failed',decision=dict(decision='failed',export_locked=True,alert='TOC_PROTOCOL_FAILURE'),reason=str(exc),saveable_tree=None,artifacts=dict(raw_directory=str(root)),usage=dict(new_requests=0)))
        raise
    table=dict(schema='mpdf-bookmark-table/1',source=str(source),source_sha256=request['input_sha256'],page_count=count,
               entries=entries,revision=0,mode='paid-contents',config_version=CONFIG,operation_id=request['operation_id'],
               raw_model_path=str(root/'raw-model.json'),raw_model_sha256=sha(root/'raw-model.json'),raw_rows=rows,
               corrections=corrections,uncertainties=pred.get('uncertainties',[]),evidence_provenance=provenance,
               images=copy.deepcopy(request['images']),edit_history=[])
    if guarded:
        from .source_coverage import observe
        from .failure_detection import decide, seal_tree
        probe=observe(request)
        put(root/'source-coverage.json',probe)
        admission=seal_tree(decide(rows,probe,request['input_sha256'],table['raw_model_sha256']),table)
        put(root/'admission-r000.json',admission)
        table['admission']=admission
        if admission['decision']=='abstain':
            candidate=put(root/'diagnostic-candidate.json',table)
            envelope=dict(schema_version=1,mode='paid-contents',operation_id=request['operation_id'],status='abstain',decision=admission,review_required=True,artifacts=dict(raw_directory=str(root),diagnostic_candidate=candidate),saveable_tree=None,usage=dict(new_requests=0))
            put(root/'completion.json',envelope)
            return envelope
    anchors=native_targets(source,{p-1 for p in request['page_numbers']})
    if scan_path:
        anchors.extend(checked_scan_targets(request,scan_path))
    bind_targets(table,anchors)
    writer.validate_table(table)
    path=put(root/'draft-r000.json',table)
    envelope=dict(schema_version=1,operation_id=request['operation_id'],mode='paid-contents',status='review_required',
                  input_sha256=request['input_sha256'],output_directory=str(root),artifacts=dict(draft=path,raw_directory=str(root)),
                  page_results=[dict(page_number=p,status='draft',provenance=provenance) for p in request['page_numbers']],review_required=True,
                  timings={},usage=dict(new_requests=0),evidence=dict(provenance=provenance,human_checked=False),decision=table.get('admission'))
    put(root/'completion.json',envelope)
    return envelope


def review(table_path, patch, *, allow_legacy=False):
    table=load(table_path)
    _verify_admission_path(table_path,table,allow_legacy=allow_legacy)
    if patch.get('expected_revision')!=table['revision'] or patch.get('source_sha256')!=table['source_sha256'] or sha(table['source'])!=table['source_sha256']:
        raise ValueError('stale revision or source mismatch')
    if patch.get('raw_model_sha256')!=table['raw_model_sha256']:
        raise ValueError('review ownership mismatch')
    if not isinstance(patch.get('operations'),list) or not patch['operations']:
        raise ValueError('review operations required')
    out=copy.deepcopy(table)
    byid={e['id']:e for e in out['entries']}
    receipts=[]
    for op in patch['operations']:
        entry=byid.get(op.get('id'))
        if entry is None or op.get('member_ids')!=entry['evidence_ids']:
            raise ValueError('unknown entry or stale member ownership')
        if op['op'] not in ('accept','reject','change'):
            raise ValueError('invalid review operation')
        if op['op']=='reject':
            entry['state']='rejected'
            entry['review_reasons']=['explicitly_rejected; retained in recall denominator']
        elif op['op']=='change':
            fields=op.get('fields',{})
            if not fields or set(fields)-{'title','parent','printed_page','target_pdf_page'}:
                raise ValueError('unsupported editable fields')
            entry.update(fields)
            if 'printed_page' in fields:
                parsed=writer.number(fields['printed_page']) if fields['printed_page'] else None
                entry['printed_family'],entry['printed_value']=parsed or (None,None)
                entry['target_pdf_page']=None
                entry['target_evidence']=[]
            if 'target_pdf_page' in fields:
                entry['target_evidence']=[target_proof(op,entry,out)]
            entry['state']='needs_review'
            entry['review_reasons']=['changed_entry_requires_acceptance']
        else:
            proof=op.get('source_review')
            if not isinstance(proof,dict) or proof.get('status') not in ('human_source_checked','ai_source_checked') or not proof.get('reviewer') or not proof.get('evidence_ref'):
                raise ValueError('source title/page/hierarchy review evidence required')
            if entry['target_pdf_page'] is None or not entry['target_evidence']:
                raise ValueError('target remains unknown')
            target_proof(op,entry,out)
            if out.get('admission'):
                image=next((i for i in out['images'] if i['page_number']==entry['source_page']+1),None)
                if not image or proof.get('image_loaded') is not True or proof.get('image_sha256')!=image['sha256'] or sha(image['path'])!=image['sha256']:
                    raise ValueError('actual source image load/hash receipt required')
                if op['target_review'].get('image_loaded') is not True or not op['target_review'].get('target_image_sha256'):
                    raise ValueError('actual target image load/hash receipt required')
            entry['source_review']=proof
            entry['target_review']=op['target_review']
            entry['state']='manually_confirmed' if proof['status']=='human_source_checked' else 'ready'
            entry['review_reasons']=[]
        receipts.append(copy.deepcopy(op))
    # Reparenting candidate has explicit receipt; canonicalize safely, retaining raw tree.
    ordered=[]
    def walk(parent,level):
        for e in out['entries']:
            if e['parent']==parent:
                if e['id'] in {v['id'] for v in ordered}:
                    raise ValueError('cyclic review tree')
                e['level']=level
                ordered.append(e)
                walk(e['id'],level+1)
    walk(None,0)
    if len(ordered)!=len(out['entries']):
        raise ValueError('cyclic/dangling review parent')
    out['entries']=ordered
    out['revision']+=1
    if out.get('admission'):
        from .failure_detection import hash_value, tree_hash
        admission=out['admission'];admission['revision']=out['revision'];admission['candidate_sha256']=tree_hash(out)
        if any(op['op']=='change' for op in patch['operations']):
            admission['decision']='abstain';admission['export_locked']=True
            admission['risks'].append(dict(code='changed_candidate_requires_new_coverage_decision',severity='unknown'))
        admission.pop('decision_sha256',None);admission['decision_sha256']=hash_value(admission)
        put(Path(table_path).parent/f'admission-r{out["revision"]:03d}.json',admission)
    out['edit_history'].append(dict(revision=out['revision'],operations=receipts))
    writer.validate_table(out)
    root=Path(table_path).resolve().parent
    result=put(root/f'draft-r{out["revision"]:03d}.json',out)
    receipt=put(root/f'review-r{out["revision"]:03d}.json',dict(source_sha256=out['source_sha256'],base_revision=table['revision'],revision=out['revision'],operations=receipts,draft_sha256=sha(result)))
    return dict(draft=result,receipt=receipt,revision=out['revision'])


def target_proof(op,entry,table):
    proof=op.get('target_review')
    if not isinstance(proof,dict) or proof.get('source_sha256')!=table['source_sha256'] or proof.get('pdf_page')!=entry['target_pdf_page'] or proof.get('status') not in ('ai_source_checked','human_source_checked') or not proof.get('reviewer') or not proof.get('evidence_ref') or not proof.get('title_position_checked'):
        raise ValueError('target source/title/position proof required')
    if type(proof['pdf_page']) is not int or not 0<=proof['pdf_page']<table['page_count']:
        raise ValueError('target outside source')
    if 'target_image_path' in proof or 'target_image_sha256' in proof:
        if not proof.get('target_image_path') or sha(proof['target_image_path'])!=proof.get('target_image_sha256'):
            raise ValueError('target review image changed')
        with fitz.open(table['source']) as doc:
            pixels=doc[proof['pdf_page']].get_pixmap(matrix=fitz.Matrix(150/72,150/72),alpha=False).tobytes('png')
        if hashlib.sha256(pixels).hexdigest()!=proof['target_image_sha256']:
            raise ValueError('target review pixels do not match actual source page')
    return proof


def _verify_admission_path(table_path,table, *, allow_legacy=False):
    from .failure_detection import verify, hash_value, CONFIG as GUARD_CONFIG
    guard=table.get('admission')
    if not guard and allow_legacy:
        verify(table,allow_legacy=True)
        return
    verify(table)
    root=Path(table_path).resolve().parent
    # Every revision is required; absence never activates legacy compatibility.
    for revision in range(table['revision']+1):
        path=root/f'admission-r{revision:03d}.json'
        if not path.is_file():raise ValueError('complete persisted admission ledger required')
        ledger=load(path)
        check=copy.deepcopy(ledger);saved=check.pop('decision_sha256',None)
        if saved!=hash_value(check) or ledger.get('config')!=GUARD_CONFIG or ledger.get('config_sha256')!=hash_value(GUARD_CONFIG):
            raise ValueError('persisted admission ledger identity changed')
        if ledger.get('revision')!=revision or ledger.get('source_sha256')!=table['source_sha256'] or ledger.get('raw_model_sha256')!=table['raw_model_sha256']:
            raise ValueError('persisted admission ledger revision/source/raw mismatch')
        if ledger.get('probe_sha256')!=guard['probe_sha256'] or ledger.get('export_locked') or ledger.get('decision') in ('abstain','failed'):
            raise ValueError('persisted admission ledger retains export lock')
        if revision==table['revision'] and ledger!=guard:
            raise ValueError('admission ledger mismatch; lock cannot be removed or edited')
    probe_path=root/'source-coverage.json';raw_path=root/'raw-model.json'
    if not probe_path.is_file() or hash_value(load(probe_path))!=guard['probe_sha256']:
        raise ValueError('persisted source coverage proof required')
    if not raw_path.is_file() or sha(raw_path)!=guard['raw_model_sha256']:
        raise ValueError('persisted original raw model proof required')


def save(table_path, request, *, allow_legacy=False):
    table=load(table_path)
    _verify_admission_path(table_path,table,allow_legacy=allow_legacy)
    if request.get('expected_revision')!=table['revision'] or request.get('source_sha256')!=table['source_sha256']:
        raise ValueError('stale save revision/source')
    if table.get('admission',{}).get('decision')=='review' and (request.get('selected_ids') is None or request.get('explicit_partial_export_confirmed') is not True):
        raise ValueError('coverage review requires explicit partial export; uncovered source denominator retained')
    original=copy.deepcopy(table)
    selected=request.get('selected_ids')
    partial=selected is not None
    if partial:
        if not isinstance(selected,list) or not selected or len(set(selected))!=len(selected) or request.get('explicit_partial_export_confirmed') is not True:
            raise ValueError('explicit nonempty unique partial selection and confirmation required')
        known={e['id']:e for e in table['entries']}
        if set(selected)-set(known):raise ValueError('unknown partial selection')
        if any(known[i]['parent'] is not None and known[i]['parent'] not in selected for i in selected):
            raise ValueError('partial selection must include every parent; no implicit promotion')
        table=copy.deepcopy(table)
        table['entries']=[e for e in table['entries'] if e['id'] in selected]
        writer.validate_table(table)
    rejected={e['id'] for e in table['entries'] if e['state']=='rejected'}
    if rejected:
        if request.get('excluded_ids') is None or set(request['excluded_ids'])!=rejected or request.get('explicit_exclusion_confirmed') is not True:
            raise ValueError('rejected rows retained; explicit inclusion decision required before export')
        table=copy.deepcopy(table)
        table['entries']=[e for e in table['entries'] if e['id'] not in rejected]
        writer.validate_table(table)
    for entry in table['entries']:
        if not entry.get('source_review') or not entry.get('target_review') or not entry.get('target_evidence'):
            raise ValueError('unreviewed source-bound target')
    output=Path(request['output_pdf']).resolve()
    from .writer_adapter import export as checked_export
    receipt=checked_export(writer,table,output,cancelled=lambda:check_cancel(Path(table_path).parent))
    remaining=[dict(id=e['id'],state=e['state'],target_pdf_page=e['target_pdf_page'],review_reasons=e['review_reasons']) for e in original['entries'] if e['id'] not in {x['id'] for x in table['entries']}]
    path=put(Path(table_path).parent/f'save-r{table["revision"]:03d}.json',dict(receipt,revision=table['revision'],output=str(output),excluded_ids=sorted(rejected),original_entry_denominator=len(original['entries']),partial_export=partial,selected_ids=[e['id'] for e in table['entries']],remaining_entries=remaining))
    return dict(schema_version=1,operation_id=table['operation_id'],mode='paid-contents',status='draft',input_sha256=table['source_sha256'],output_directory=str(Path(table_path).parent.resolve()),
                artifacts=dict(draft=str(Path(table_path).resolve()),bookmarks_pdf=str(output),receipt=path),page_results=[],review_required=bool(remaining),timings=dict(export_seconds=receipt['seconds']),usage=dict(new_requests=0),evidence=dict(pdf_consumer=receipt,evidence_provenance=table['evidence_provenance'],partial_export=partial,original_entry_denominator=len(original['entries']),exported_entries=len(table['entries']),remaining_entries=remaining,human_approved=all(e['source_review']['status']=='human_source_checked' and e['target_review']['status']=='human_source_checked' for e in table['entries'])))


def check_cancel(root):
    if (Path(root)/'cancel').exists():
        raise InterruptedError('operation_cancelled')


def live(request, approval, ledger_directory):
    """Single bounded request, durable reservation, no retries/ambiguous resume.
    Shared reservation ledger directory must be coordinator-owned and pre-reconciled.
    """
    manifest=preflight(request)
    if approval.get('coordinator_reconciled') is not True or not approval.get('user_authorization_ref'):
        raise ValueError('coordinator C+D budget checkpoint not satisfied')
    if approval.get('destination')!=DESTINATION or approval.get('model')!=MODEL or approval.get('image_sha256s')!=[i['sha256'] for i in request['images']]:
        raise ValueError('exact upload/model/destination authorization mismatch')
    if approval.get('request_cap')!=1 or approval.get('reservation_usd')!=.02 or approval.get('original_total_cap_usd')!=1:
        raise ValueError('bounded original budget required')
    ledger=Path(ledger_directory).resolve()
    # A common C+D lock prevents simultaneous reservations. Existing unknown requests stay reserved.
    ledger.mkdir(parents=True,exist_ok=True)
    lock=ledger/'reservation.lock'
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    os.close(fd)
    body=None
    try:
        identity=hashlib.sha256(json.dumps(dict(source=request['input_sha256'],images=approval['image_sha256s'],model=MODEL,prompt=manifest['prompt_sha256']),sort_keys=True).encode()).hexdigest()
        call=ledger/identity
        if call.exists():
            raise ValueError('request identity already attempted; no duplicate spend or hidden retry')
        history=approval.get('history_reservation_usd')
        cap=approval.get('remaining_reservation_usd')
        if not isinstance(history,(int,float)) or not isinstance(cap,(int,float)) or history<0 or cap<.02 or history+cap>1:
            raise ValueError('original historical reservations unresolved')
        reserved=sum(load(p).get('reserved_usd',0) for p in ledger.glob('*/attempt.json'))
        if reserved+.02>cap:
            raise ValueError('shared budget exhausted')
        # Preflight image dimensions cap token usage at high detail (conservative tiles + text bound).
        content=[dict(type='input_text',text=PROMPT)]
        tokens=2000
        for image in request['images']:
            from PIL import Image
            with Image.open(image['path']) as im:
                w,h=im.size
            tokens+=85+170*((w+511)//512)*((h+511)//512)
            content.append(dict(type='input_text',text=f'Selected PDF page {image["page_number"]}'))
            content.append(dict(type='input_image',image_url='data:image/png;base64,'+base64.b64encode(Path(image['path']).read_bytes()).decode(),detail='high'))
        if tokens>20000:
            raise ValueError('conservative input token cap exceeded; prepare smaller authorized scope')
        check_cancel(request['output_directory'])
        call.mkdir()
        put(call/'attempt.json',dict(request_id=identity,reserved_usd=.02,started_unix=time.time(),manifest=manifest,state='reserved',authorization_ref=approval['user_authorization_ref']))
        body=dict(model=MODEL,reasoning=dict(effort='low'),max_output_tokens=10000,store=False,input=[dict(role='user',content=content)])
        # Request stored as hashes/options, never base64 image or credential.
        put(call/'request.json',dict(manifest=manifest,request_sha256=hashlib.sha256(json.dumps(body).encode()).hexdigest()))
    finally:
        lock.unlink()
    # Read only the established secure local config; never serialize credentials.
    key=None
    for line in (ROOT/'.env').read_text().splitlines():
        match=re.match(r'^\s*(?:export\s+)?OPENAI_API_KEY\s*=\s*(.*?)\s*$',line)
        if match:
            key=match[1].strip('\"\'')
    if not key:
        raise ValueError('credential unavailable; reservation retained conservatively')
    req=urllib.request.Request(DESTINATION,data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+key,'Content-Type':'application/json'})
    started=time.monotonic()
    status=None
    raw=b''
    error=None
    try:
        with urllib.request.urlopen(req,timeout=150) as response:
            raw=response.read()
            status=response.status
    except urllib.error.HTTPError as exc:
        status=exc.code
        raw=exc.read()
        error='HTTP_'+str(status)
    except Exception as exc:
        error=type(exc).__name__
    if raw:
        with (call/'response.json').open('xb') as handle:
            handle.write(raw)
    receipt=dict(status=status,error=error,seconds=time.monotonic()-started,reserved_usd=.02,billing_debit_verified=False,request_id=identity)
    put(call/'receipt.json',receipt)
    if status!=200:
        raise RuntimeError('provider request failed; raw failure retained, no retry')
    response=json.loads(raw)
    if response.get('status')!='completed' or response.get('model')!=MODEL:
        raise ValueError('provider incomplete or model identity mismatch')
    text=''.join(c.get('text','') for item in response.get('output',[]) for c in item.get('content',[]) if c.get('type')=='output_text')
    pred=json.loads(text)
    try:
        observations(pred,request)
    except ValueError as exc:
        put(call/'parsed-but-invalid-prediction.json',pred)
        put(call/'protocol-failure.json',dict(status='failed',error=str(exc),request_id=identity,no_retry=True))
        raise
    path=put(call/'prediction.json',pred)
    check_cancel(request['output_directory'])
    result=draft(request,path,'live')
    result['usage']=dict(new_requests=1,receipt=str(call/'receipt.json'),reserved_usd=.02,provider_usage=response.get('usage',{}))
    # Completion was persisted by draft before response receipt enrichment; write separate final.
    put(Path(request['output_directory'])/'live-completion.json',result)
    return result


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('command',choices=['readiness','preflight','draft','guarded-draft','review','save','live'])
    parser.add_argument('--request')
    parser.add_argument('--prediction')
    parser.add_argument('--provenance',choices=['cached','mock'],default='cached')
    parser.add_argument('--scan-observations')
    parser.add_argument('--table')
    parser.add_argument('--approval')
    parser.add_argument('--ledger-directory')
    args=parser.parse_args()
    try:
        if args.command=='readiness':
            result=readiness()
        elif args.command=='preflight':
            result=preflight(load(args.request))
        elif args.command in ('draft','guarded-draft'):
            result=draft(load(args.request),args.prediction,args.provenance,args.scan_observations,guarded=True)
        elif args.command=='review':
            result=review(args.table,load(args.request))
        elif args.command=='save':
            result=save(args.table,load(args.request))
        else:
            result=live(load(args.request),load(args.approval),args.ledger_directory)
        print(json.dumps(result,ensure_ascii=False))
    except Exception as exc:
        print(json.dumps(dict(schema_version=1,mode='paid-contents',status='cancelled' if isinstance(exc,InterruptedError) else 'failed',error=type(exc).__name__,message=str(exc)),ensure_ascii=False))
        return 1
    return 0

if __name__=='__main__':
    raise SystemExit(main())
