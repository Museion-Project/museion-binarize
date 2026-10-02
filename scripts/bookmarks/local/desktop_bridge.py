#!/usr/bin/env python3
"""Desktop adapter for the existing local evidence/mapping/writer helpers.
Rust calls compile_local directly between prepare and finish. This adapter never
calls a CLI/compiler, installs a model, or uses a network provider.
"""
import copy, hashlib, json, os, subprocess, sys, time
from pathlib import Path
import fitz
from bookmarks import native, load, save, project, paginate, native_anchors, native_header_anchors, validate_table, export, sha
from numeric_lane import run as read_numeric_lanes


def check_cancel(root):
    if (Path(root)/'cancel').exists():raise InterruptedError('operation_cancelled')


def stage(root,name,**extra):
    check_cancel(root)
    payload=dict(stage=name,**extra)
    tmp=Path(root)/'progress.tmp';tmp.write_text(json.dumps(payload));tmp.replace(Path(root)/'progress.json')


def vision_worker(root,runtime):
    source=Path(__file__).with_name('vision_fast.swift')
    if sys.platform!='darwin':raise RuntimeError('图像目录识别目前需要 macOS；含可用文字的 PDF 可直接读取。')
    digest=hashlib.sha256(source.read_bytes()).hexdigest()[:16]
    runtime=Path(runtime);runtime.mkdir(parents=True,exist_ok=True)
    binary=runtime/f'vision-fast-{digest}'
    if not binary.is_file():
        stage(root,'preparing_local_recognizer')
        modules=runtime/'modules';modules.mkdir(exist_ok=True)
        tmp=binary.with_suffix(f'.{os.getpid()}.building')
        subprocess.run(['/usr/bin/swiftc','-O','-module-cache-path',str(modules),str(source),'-o',str(tmp)],check=True,timeout=120)
        tmp.replace(binary)
    return str(binary)


def prepare(request,root):
    root=Path(root);source=str(Path(request['source']).resolve());mode=request.get('mode','auto')
    if mode not in ('auto','image'):raise ValueError('invalid extraction mode')
    with fitz.open(source) as doc:
        if doc.is_encrypted:raise ValueError('目录生成暂不支持加密 PDF，请先保存不加密副本。')
        pages=request['pages']
        if not pages or len(pages)>40 or any(type(p)!=int or not 1<=p<=len(doc) for p in pages):raise ValueError('目录页必须是 PDF 中有效的页码，最多40页。')
        if pages!=sorted(set(pages)):raise ValueError('目录页需要按文档顺序排列且不可重复。')
        indices=[p-1 for p in pages];count=len(doc)
    began=time.perf_counter();digest=sha(source)
    stage(root,'reading_contents',pages=pages)
    raw=[]
    if mode=='auto':
        native(source,indices,root/'native-raw.json');raw=load(root/'native-raw.json')
    native_by_page={r['page_index']:r for r in raw}
    evidence=[];requests=[];routes=[]
    with fitz.open(source) as doc:
        for index in indices:
            check_cancel(root);r=native_by_page.get(index)
            # A few running-header glyphs are not a usable contents page.
            usable=r is not None and len(r['glyphs'])>=30 and sum(c['text'].isalpha() for c in r['glyphs'])>=15
            if usable:
                evidence.append(project(r,root/'native-raw.json'))
                routes.append(dict(page=index+1,path='native_text'))
            else:
                rendered=request['rendered_pages'][str(index+1)]
                if rendered['renderer']!='pdfium' or rendered['dpi']!=150:raise ValueError('unexpected contents raster contract')
                image=Path(rendered['image_path'])
                if not image.is_file():raise ValueError('missing contents raster')
                page=doc[index]
                requests.append(dict(id=f'p{index}',source=source,source_sha256=digest,page_index=index,page_count=count,width=page.rect.width,height=page.rect.height,image_path=str(image),render_seconds=rendered['render_seconds']))
                routes.append(dict(page=index+1,path='apple_vision_fast'))
    if requests:
        worker=vision_worker(root,request['runtime_cache']);stage(root,'recognizing_contents')
        save(root/'vision-requests.json',requests)
        subprocess.run([worker,str(root/'vision-requests.json'),str(root/'vision')],check=True,timeout=120)
        raw_paths=sorted((root/'vision').glob('*.json'))
        for raw_path in raw_paths:
            recognition=load(raw_path)
            if recognition.get('state')=='failed':
                raise RuntimeError('本机图像识别失败：'+str(recognition.get('error','unknown error')))
        stage(root,'reading_page_numbers')
        evidence.extend(read_numeric_lanes(raw_paths,root/'numeric-lane',worker,max_crops=4000))
    evidence.sort(key=lambda p:p['page_index']);save(root/'evidence.json',evidence)
    stage(root,'mapping_pages')
    if request.get('pagination_path'):
        model=load(request['pagination_path'])
        if model['source_sha256']!=digest:raise ValueError('pagination source mismatch')
        save(root/'pagination.json',model)
        anchors=[]
    else:
        anchors=native_anchors(source) or native_header_anchors(source,indices)
    save(root/'anchors.json',anchors)
    if sha(source)!=digest:raise ValueError('source PDF changed during generation')
    save(root/'intake.json',dict(source_sha256=digest,routes=routes,selected_pages=pages,mode=mode,prepare_seconds=time.perf_counter()-began,full_page_renderer="pdfium",full_page_dpi=150,ocr_tool_enabled=False,surya_pages=0))
    return dict(evidence_path=str(root/'evidence.json'),routes=routes)


def finish(root):
    root=Path(root);stage(root,'building_tree')
    table=load(root/'compiled.json');pages=load(root/'evidence.json')
    byid={line['id']:line for page in pages for line in page['lines']}
    for e in table['entries']+table.get('source_entries',[]):
        lines=[byid[i] for i in e['evidence_ids']]
        if any(l.get('number_confidence') is not None and l['number_confidence']<.8 for l in lines):
            e['review_reasons'].append('printed_low_confidence')
        e['state']='needs_review'
    if (root/'pagination.json').is_file():
        from pagination import map_table
        table=map_table(table,load(root/'pagination.json'))
    else:
        table=paginate(table,load(root/'anchors.json'))
    table['evidence_path']=str(root/'evidence.json')
    validate_table(table)
    if table.get('source_entries'):validate_table(dict(table,entries=table['source_entries']))
    save(root/'table.json',table)
    return dict(table=table,intake=load(root/'intake.json'))


def reviewed_table(base,entries,review_accepted,projection):
    """Accept editable fields only. Evidence, source identity and anchors are trusted base data."""
    if review_accepted is not True:raise ValueError('请先核对整份目录，再确认保存。')
    if projection not in ('navigation','source'):raise ValueError('invalid projection')
    if not isinstance(entries,list) or not entries or len(entries)>4000:raise ValueError('目录需要包含有效条目。')
    originals={e['id']:e for e in base.get('source_entries',base['entries'])}
    t=copy.deepcopy(base);t['entries']=[];seen=set()
    for item in entries:
        if set(item)-{'id','title','parent','target_pdf_page'}:raise ValueError('unsupported edited field')
        ident=item['id']
        if not isinstance(ident,str) or ident in seen or len(ident)>128:raise ValueError('invalid or duplicate entry id')
        seen.add(ident)
        if ident in originals:e=copy.deepcopy(originals[ident])
        elif ident.startswith('manual-'):
            e=dict(id=ident,title='',parent=None,level=0,section_label=None,printed_page=None,printed_value=None,printed_family=None,source_page=0,source_bbox=dict(x=0,y=0,width=1,height=1),evidence_ids=[],hierarchy_reason='manual',target_pdf_page=None,review_reasons=[])
        else:raise ValueError('unknown entry id')
        e.update(item);e['state']='manually_confirmed';e['review_reasons']=[]
        e['manual_evidence']='desktop:explicit-book-review';t['entries'].append(e)
    ordered=[];visited=set()
    def walk(parent,level):
        for e in t['entries']:
            if e['parent']==parent:
                if e['id'] in visited:raise ValueError('cyclic tree')
                visited.add(e['id']);e['level']=level;ordered.append(e);walk(e['id'],level+1)
    walk(None,0)
    if len(ordered)!=len(t['entries']):raise ValueError('目录包含循环或不存在的父节点。')
    t['entries']=ordered
    t['desktop_review']=dict(accepted=True,projection=projection,submitted_entries=entries,base_source_sha256=base['source_sha256'])
    validate_table(t,True)
    return t


def save_review(request,root):
    root=Path(root);stage(root,'checking_bookmarks')
    base=load(request['table_path'])
    if sha(base['source'])!=base['source_sha256']:raise ValueError('source PDF changed')
    t=reviewed_table(base,request['entries'],request['review_accepted'],request['projection'])
    save(root/'reviewed-original.json',t)
    processed=request.get('processed_source')
    if processed:
        # The original table is retained above. The writer verifies its own
        # input, which is the independently validated binarization result.
        with fitz.open(processed) as pdf:
            if len(pdf)!=base['page_count']:raise ValueError('processed page count changed')
        t['source']=processed;t['source_sha256']=sha(processed)
        t['original_source_sha256']=base['source_sha256']
    stage(root,'writing_bookmarks')
    receipt=export(t,request['output'],cancelled=lambda:check_cancel(root))
    save(root/'writer-validation.json',receipt)
    return receipt


def main():
    command,request_path=sys.argv[1:];request=load(request_path);root=Path(request['work_dir'])
    if command=='prepare':result=prepare(request,root)
    elif command=='analyze':
        from pagination import analyze
        result=analyze(request,root,stage,check_cancel,vision_worker)
    elif command=='finish':result=finish(root)
    elif command=='save':result=save_review(request,root)
    else:raise ValueError('unknown desktop operation')
    save(root/f'{command}-result.json',result)

if __name__=='__main__':main()
