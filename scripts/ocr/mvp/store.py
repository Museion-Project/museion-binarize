"""Immutable revision snapshots with single-writer CAS and persisted review receipts."""
import copy
from collections import Counter
import fcntl
import html
import json
import os
import uuid
from pathlib import Path
import fitz
from scripts.ocr.free_local import pipeline as old
from .core import digest, sha, invariant, ownership, reading_rows, CONSUMER_POLICY

EXPORTER_VERSION = 'row-sequence-glyph-v1'

def read(p):return json.loads(Path(p).read_text())
def write(p,x):
    with Path(p).open('w') as f:
        json.dump(x,f,ensure_ascii=False,indent=2,allow_nan=False);f.flush();os.fsync(f.fileno())

def load_snapshot(root):
    root=Path(root).resolve();pointer=read(root/'CURRENT.json')
    folder=(root/pointer['folder']).resolve()
    if not folder.is_relative_to(root):raise ValueError('POINTER_SCOPE')
    manifest=read(folder/'manifest.json')
    if sha(folder/'manifest.json')!=pointer['manifest_sha256']:raise ValueError('MANIFEST_CORRUPT')
    for name,h in manifest['files'].items():
        p=(folder/name).resolve()
        if not p.is_relative_to(folder) or sha(p)!=h:raise ValueError('ARTIFACT_CORRUPT')
    snapshot=read(folder/'snapshot.json');invariant(snapshot)
    if pointer['revision']!=snapshot['revision']:raise ValueError('MIXED_REVISION')
    for page in snapshot['pages']:
        for relative,h in page.get('raw_files',{}).items():
            raw=(root/relative).resolve()
            if not raw.is_relative_to(root) or sha(raw)!=h:raise ValueError('RAW_EVIDENCE_CORRUPT')
    return snapshot,folder

def reader_alternatives(snapshot):
    """Read-only saved readings, never new output members or source ground truth.

    Match final members by exact reader ID and engine, not text or proximity.
    Duplicate/missing IDs retain every raw record with an ambiguous identity.
    The raw boxes are reader observations, not validated glyph geometry.
    """
    rows=[];coverage=[]
    for page in snapshot['pages']:
        streams=('independent_reader','residual_reader')
        raw=[(stream,index,word) for stream in streams
             if isinstance(page.get(stream),list)
             for index,word in enumerate(page[stream])]
        ids=Counter(word.get('id') for _,_,word in raw
                    if isinstance(word,dict) and isinstance(word.get('id'),str) and word['id'])
        final={(word.get('id'),word.get('engine')) for word in page.get('words',[])}
        adopted=0
        for stream,index,original in raw:
            word=original if isinstance(original,dict) else {}
            member=word.get('id');unique=isinstance(member,str) and bool(member) and ids[member]==1
            engine=word.get('engine')
            if unique and isinstance(engine,str) and (member,engine) in final:
                adopted+=1;continue
            # Bind immutable evidence separately from the current review revision.
            evidence=digest(original)
            binding=dict(source_sha256=snapshot['input_sha256'],page=page.get('page'),
                         image_sha256=page.get('image_sha256'),stream=stream,
                         raw_index=index,raw_member_id=member,evidence_sha256=evidence)
            decisions=[copy.deepcopy(decision) for decision in page.get('decisions',[])
                       if unique and (decision.get('reader_id')==member or member in decision.get('reader_ids',[]))]
            actions=[dict(revision=receipt.get('revision'),action=action.get('action'))
                     for receipt in snapshot.get('receipts',[])
                     if unique and receipt.get('source_hash')==snapshot['input_sha256']
                     for action in receipt.get('actions',[])
                     if action.get('page')==page.get('page') and action.get('member_id')==member]
            rows.append(dict(binding,alternative_id='reader-alternative:'+digest(binding),
                             revision=snapshot['revision'],text=word.get('text',''),
                             raw_record=copy.deepcopy(original),
                             identity_status='UNIQUE' if unique else 'AMBIGUOUS',
                             geometry_status='READER_OBSERVATION_UNVERIFIED',
                             decision_status='RECORDED' if decisions else 'NOT_RECORDED',
                             decisions=decisions,review_actions=actions,read_only=True))
        coverage.append(dict(page=page.get('page'),raw_records=len(raw),adopted_records=adopted,
                             alternative_records=len(raw)-adopted,
                             unavailable_streams=[stream for stream in streams if not isinstance(page.get(stream),list)]))
    return dict(schema='saved-reader-alternatives/1',source_sha256=snapshot['input_sha256'],
                revision=snapshot['revision'],rows=rows,coverage=coverage,read_only=True)

def review_html(snapshot):
    cards=[]
    alternatives=reader_alternatives(snapshot)
    for page in snapshot['pages']:
        words=page.get('words',[])
        controls=''.join('<label>'+html.escape(w['id'])+' ['+html.escape(w.get('export_status','UNCHECKED'))+'] '+' <select data-action><option>accept</option><option>reject</option><option>change</option></select><input data-text value="'+html.escape(w['text'],quote=True)+'" data-id="'+html.escape(w['id'],quote=True)+'"></label><br>' for w in words)
        readings=''.join('<li data-alternative="'+html.escape(row['alternative_id'],quote=True)+'"><strong>'+html.escape(str(row['text']))+'</strong><pre>'+html.escape(json.dumps(row,ensure_ascii=False,indent=2))+'</pre></li>' for row in alternatives['rows'] if row['page']==page['page'])
        coverage=next(item for item in alternatives['coverage'] if item['page']==page['page'])
        readonly='<details><summary>未采用的识别读法 · Saved readings ('+str(coverage['alternative_records'])+')</summary><p>只读原始记录。可能是同一文字的其他读法；不代表漏词或正确文字。查看原页不会接受它们。Reader boxes are unverified.</p><p>'+html.escape(json.dumps(coverage,ensure_ascii=False))+'</p><ul>'+readings+'</ul></details>'
        cards.append(f'<section data-page="{page["page"]}"><h2>Page {page["page"]}: {html.escape(page["status"])}</h2><img src="{html.escape(page.get("image_path", ""))}"><div>{controls}{readonly}</div></section>')
    context=json.dumps(dict(expected_revision=snapshot['revision'],input_sha256=snapshot['input_sha256']),ensure_ascii=False)
    return '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>OCR source review</title><style>body{font:16px system-ui;margin:20px}img{width:48%;vertical-align:top}section>div{display:inline-block;width:48%;max-height:800px;overflow:auto}input{width:60%}button{padding:12px}</style><h1>机器草稿 · Source review</h1><p>每项绑定实际词框。Accept / Reject / Change 后导出补丁，使用 review-save 保存新版本 PDF。保存前原图和原始观察保留；下载补丁不是已保存 PDF。</p><button onclick="patch()">Export review patch</button>'+''.join(cards)+'''<script>const context='''+context+''';function patch(){const actions=[];document.querySelectorAll('section').forEach(s=>s.querySelectorAll('input').forEach(i=>actions.push({page:Number(s.dataset.page),member_id:i.dataset.id,action:i.previousElementSibling.value,text:i.value})));const a=document.createElement('a');a.href=URL.createObjectURL(new Blob([JSON.stringify({...context,actions},null,2)],{type:'application/json'}));a.download='review-patch.json';a.click();URL.revokeObjectURL(a.href)}</script>'''

def export_fonts(snapshot):
    """Existing fonts only; system fallback is a local dependency, not a bundled license."""
    paths=[str(snapshot.get('font_path',old.FONT))]+snapshot.get('fallback_font_paths',['/System/Library/Fonts/Times.ttc'])
    return [(path,fitz.Font(fontfile=path)) for path in dict.fromkeys(paths) if Path(path).is_file()]

def noncharacter(code):
    return 0xFDD0<=code<=0xFDEF or code&0xFFFF in (0xFFFE,0xFFFF)

def prepare_export(snapshot):
    snapshot['exporter_version']=EXPORTER_VERSION
    snapshot['consumer_policy']=CONSUMER_POLICY
    with fitz.open(snapshot['source_pdf']) as source:count=len(source)
    selected=[page['page'] for page in snapshot['pages']]
    if len(selected)!=len(set(selected)) or any(type(number) is not int or not 1<=number<=count for number in selected):raise ValueError('INVALID_PHYSICAL_PAGE_MAPPING')
    snapshot['source_page_count']=count;snapshot['exported_page_count']=count
    snapshot['untouched_page_numbers']=[number for number in range(1,count+1) if number not in selected]
    snapshot['pdf_page_mapping']=[dict(source_page=number,exported_page=number,selected=number in selected,result_index=selected.index(number) if number in selected else None) for number in range(1,count+1)]
    for page in snapshot['pages']:page['exported_page']=page['page']
    fonts=export_fonts(snapshot)
    snapshot['export_font_dependencies']=[dict(path=path,sha256=sha(path),origin='macOS installed system font' if path.startswith('/System/') else 'configured font',distribution_closed=False) for path,_ in fonts]
    pending=[]
    for page in snapshot['pages']:
        issues=[]
        if page.get('route')!='ocr':continue
        for word in page.get('words',[]):
            text=word['text'];bad=[f'U+{ord(c):04X}' for c in text if noncharacter(ord(c)) or ord(c)<32 or 0xD800<=ord(c)<=0xDFFF]
            chosen=None if bad else next((path for path,font in fonts if all(font.has_glyph(ord(c)) for c in text)),None)
            word['export_status']='EXPORTED' if chosen else 'EXPORT_REVIEW'
            word['export_font_path']=chosen
            word['export_issues']=bad if bad else ([] if chosen else ['UNSUPPORTED_GLYPH'])
            if not chosen:
                word['review']=True;issues.append(dict(member_id=word['id'],text=text,issues=word['export_issues']))
        page['export_review']=issues
        page['export_coverage']=dict(total_words=len(page.get('words',[])),exported_words=len(page.get('words',[]))-len(issues),pending_words=len(issues))
        if issues:
            if page['status']!='EXPORT_REVIEW':page['pre_export_status']=page['status']
            page['status']='EXPORT_REVIEW';pending.extend(dict(page=page['page'],**issue) for issue in issues)
        elif page['status']=='EXPORT_REVIEW':page['status']=page.pop('pre_export_status','OCR_DRAFT')
    snapshot['export_review']=pending

def insert_exportable(page,words,width,height,fonts):
    sx,sy=page.rect.width/width,page.rect.height/height
    import statistics
    for row in reading_rows(words):
        supported=[w for w in row if w['export_status']=='EXPORTED']
        if not supported:continue
        baselines=[]
        for word in supported:
            font=fonts[word['export_font_path']];x,y,r,b=word['bbox']
            fs=max(1,(b-y)*sy/(font.ascender-font.descender))
            baselines.append(y*sy+font.ascender*fs)
        baseline=statistics.median(baselines)
        for word in supported:
            font=fonts[word['export_font_path']];x,y,r,b=word['bbox'];rect=fitz.Rect(x*sx,y*sy,r*sx,b*sy)
            fs=max(1,rect.height/(font.ascender-font.descender));pt=fitz.Point(rect.x0,baseline)
            writer=fitz.TextWriter(page.rect);writer.append(pt,word['text']+' ',font=font,fontsize=fs)
            writer.write_text(page,render_mode=3,morph=(pt,fitz.Matrix(rect.width/max(.001,font.text_length(word['text'],fontsize=fs)),1)))

def export_pdf(snapshot,path):
    return _export_pdf_with_inserter(snapshot,path,insert_exportable)

def _export_pdf_with_inserter(snapshot,path,inserter):
    """Share source preservation with isolated output candidates; formal default unchanged."""
    if snapshot.get('font_path'):old.FONT=Path(snapshot['font_path'])
    source=Path(snapshot['source_pdf'])
    if sha(source)!=snapshot['input_sha256']:raise ValueError('SOURCE_CHANGED')
    out=fitz.open(source);source_toc=out.get_toc(simple=False);fonts=dict(export_fonts(snapshot))
    for p in snapshot['pages']:
        idx=p['page']-1
        if p['route']=='ocr' and p.get('words'):
            target=out[idx]
            if p.get('text_layer_policy')=='replace' and target.get_text().strip():
                # Preserve image/graphics bytes and visible pixels; remove only an
                # explicitly selected hidden-only layer. Rich visible text is blocked upstream.
                if any(t['type']!=3 for t in target.get_texttrace()):raise ValueError('VISIBLE_LAYER_REPLACEMENT_UNSUPPORTED')
                target.add_redact_annot(target.rect,fill=False,cross_out=False)
                target.apply_redactions(images=0,graphics=0,text=0)
            if target.rotation:
                # Lossless page normalization using original PDF graphics rather than
                # a second raster resample. Pixel checks cover this real path.
                rotation=target.rotation;dimensions=target.rect
                temporary=fitz.open();temporary.insert_pdf(out,from_page=idx,to_page=idx)
                temporary[0].set_rotation(0)
                replacement=fitz.open();normalized=replacement.new_page(width=dimensions.width,height=dimensions.height)
                normalized.show_pdf_page(normalized.rect,temporary,0,rotate=-rotation)
                out.delete_page(idx);out.insert_pdf(replacement,start_at=idx);target=out[idx];temporary.close();replacement.close()
            inserter(target,p['words'],p['width'],p['height'],fonts)
    if source_toc:out.set_toc(source_toc)
    out.save(path,deflate=True,garbage=3);out.close()

def publish(root,snapshot,expected_revision=None):
    root=Path(root).resolve();root.mkdir(parents=True,exist_ok=True)
    with (root/'.writer.lock').open('a+') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        previous=load_snapshot(root)[0] if (root/'CURRENT.json').exists() else None
        if (previous['revision'] if previous else None)!=expected_revision:raise ValueError('STALE_REVISION')
        if previous:
            if previous['input_sha256']!=snapshot['input_sha256'] or snapshot['revision']!=previous['revision']+1:raise ValueError('SOURCE_OR_REVISION_REPLAY')
        prepare_export(snapshot)
        snapshot['pages_hash']=digest(snapshot['pages']);invariant(snapshot)
        tmp=root/('.prepare-'+uuid.uuid4().hex);tmp.mkdir()
        write(tmp/'snapshot.json',snapshot);write(tmp/'pages.json',snapshot['pages']);write(tmp/'receipts.json',snapshot['receipts'])
        write(tmp/'page-map.json',{key:snapshot[key] for key in ('source_page_count','exported_page_count','untouched_page_numbers','pdf_page_mapping')})
        export_pdf(snapshot,tmp/'searchable.pdf')
        (tmp/'text.txt').write_text('\n\n'.join(p.get('native_text','') if p['route']!='ocr' else '\n'.join(' '.join(w['text'] for w in row) for row in reading_rows(p.get('words',[]))) for p in snapshot['pages']))
        (tmp/'review.html').write_text(review_html(snapshot))
        write(tmp/'manifest.json',dict(schema_version=1,exporter_version=EXPORTER_VERSION,files={p.name:sha(p) for p in tmp.iterdir()}))
        folder=root/f'revision-{snapshot["revision"]}-{uuid.uuid4().hex[:12]}'
        os.rename(tmp,folder)
        pointer=root/('.CURRENT-'+uuid.uuid4().hex)
        write(pointer,dict(folder=folder.name,revision=snapshot['revision'],manifest_sha256=sha(folder/'manifest.json')))
        os.replace(pointer,root/'CURRENT.json')
        fd=os.open(root,os.O_RDONLY);os.fsync(fd);os.close(fd)
        return folder

def review_save(root,patch):
    """Patch: expected_revision,input_sha256,actions[{page,member_id,action,text}].
    Reject retires contribution and restores its original members only when safe.
    Text changes retain actual word support; geometry and ownership cannot be edited.
    """
    current,_=load_snapshot(root)
    if patch['expected_revision']!=current['revision']:raise ValueError('STALE_REVISION')
    if patch['input_sha256']!=current['input_sha256'] or sha(current['source_pdf'])!=current['input_sha256']:raise ValueError('SOURCE_CHANGED')
    updated=copy.deepcopy(current);seen=set()
    for action in patch['actions']:
        key=(action['page'],action['member_id'])
        if key in seen:raise ValueError('DUPLICATE_REVIEW_MEMBER')
        seen.add(key)
        p=next((p for p in updated['pages'] if p['page']==key[0]),None)
        if p is None:raise ValueError('UNKNOWN_PAGE')
        w=next((w for w in p.get('words',[]) if w['id']==key[1]),None)
        if w is None:raise ValueError('UNKNOWN_MEMBER')
        if action['action'] not in ('accept','reject','change'):raise ValueError('UNKNOWN_ACTION')
        if action['action']=='reject':
            p['words'].remove(w)
            for a in p.get('original_apple',[]):
                if a['id'] in w.get('source_members',[]) and not any(a['id'] in v.get('source_members',[]) for v in p['words']) and not any(old.intersection(a['bbox'],v['bbox']) for v in p['words']):
                    p['words'].append(dict(a,review=True,source_members=[a['id']]))
        else:
            if action['action']=='change':
                text=action['text']
                if not isinstance(text,str) or not text.strip() or '\n' in text or len(text)>200:raise ValueError('INVALID_WORD_CHANGE')
                w['text']=__import__('unicodedata').normalize('NFC',text)
            w['review']=False;w['review_status']='user_action_recorded'
    for page in updated['pages']:page['contributions']=ownership(page.get('words',[]))
    updated['revision']+=1
    updated['receipts'].append(dict(receipt_id=uuid.uuid4().hex,revision=updated['revision'],source_hash=updated['input_sha256'],patch_hash=digest(patch),actions=patch['actions'],human_approval_claimed=False))
    folder=publish(root,updated,current['revision'])
    receipt=dict(schema_version=1,revision=updated['revision'],artifacts={n:str(folder/n) for n in ('searchable.pdf','text.txt','pages.json','review.html','receipts.json','page-map.json')},receipt=updated['receipts'][-1])
    completion_path=Path(root)/'completion.json'
    if completion_path.exists():
        completion=read(completion_path);completion['revision']=updated['revision'];completion['page_results']=updated['pages'];completion['export_review']=updated['export_review']
        for key in ('source_page_count','exported_page_count','untouched_page_numbers','pdf_page_mapping'):completion[key]=updated[key]
        completion['artifacts']['page_mapping_json']=str(folder/'page-map.json')
        for key,name in [('searchable_pdf','searchable.pdf'),('text','text.txt'),('pages_json','pages.json'),('review_html','review.html')]:completion['artifacts'][key]=str(folder/name)
        temporary=completion_path.with_name('.completion-'+uuid.uuid4().hex);write(temporary,completion);os.replace(temporary,completion_path)
    return receipt
