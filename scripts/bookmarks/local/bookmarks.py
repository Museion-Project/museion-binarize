#!/usr/bin/env python3
"""Local bookmark workflow using PDFium and pypdf.
No OCR engine/model installation, network, or source PDF mutation.
All page numbers in JSON are zero-based PDF indices; printed labels are separate.
"""
import argparse, copy, hashlib, json, math, os, re, statistics, subprocess, tempfile, time
from pathlib import Path
import pdf_backend as pdf

SCHEMA='mpdf-bookmark-evidence/1'
def sha(path):
    h=hashlib.sha256()
    with open(path,'rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''): h.update(b)
    return h.hexdigest()
def load(p): return json.loads(Path(p).read_text())
def save(p,d):
    with open(p,'x') as f: json.dump(d,f,ensure_ascii=False,indent=2)
def bbox(a): return dict(zip(('x','y','width','height'),a))
def arr(b): return [b[k] for k in ('x','y','width','height')]
def union(bs):
    x=min(b[0] for b in bs);y=min(b[1] for b in bs)
    return [x,y,max(b[0]+b[2] for b in bs)-x,max(b[1]+b[3] for b in bs)-y]
def number(s):
    if re.fullmatch(r'[0-9]{1,5}',s): return ('arabic',int(s))
    if not re.fullmatch(r'[ivxlcdmIVXLCDM]+',s): return None
    vals={'i':1,'v':5,'x':10,'l':50,'c':100,'d':500,'m':1000};n=0;prev=0
    for c in s.lower()[::-1]:
        v=vals[c];n+=-v if v<prev else v;prev=max(v,prev)
    # Canonical Roman, reject nonsense strings rather than silently repairing OCR.
    def roman(n):
        out=''
        for val,token in [(1000,'m'),(900,'cm'),(500,'d'),(400,'cd'),(100,'c'),(90,'xc'),(50,'l'),(40,'xl'),(10,'x'),(9,'ix'),(5,'v'),(4,'iv'),(1,'i')]:
            q,n=divmod(n,val);out+=token*q
        return out
    return ('roman',n) if 0<n<=5000 and roman(n)==s.lower() else None

def native(source, indices, output):
    """Save PDFium native glyph measurements in visible page coordinates."""
    start=time.perf_counter();digest=sha(source);doc=pdf.Document(source);raw=[]
    for index in indices:
        page=doc[index];glyphs=page.glyphs()
        # Cluster baseline bands using measured center/height, then split large gaps.
        bands=[]
        for g in sorted(glyphs,key=lambda g:(g['bbox'][1]+g['bbox'][3]/2,g['bbox'][0])):
            b=g['bbox']; cy=b[1]+b[3]/2
            matches=[row for row in bands if abs(row['cy']-cy)<min(row['height'],b[3])*.35]
            if matches: row=min(matches,key=lambda r:abs(r['cy']-cy));row['gs'].append(g)
            else: bands.append({'cy':cy,'height':b[3],'gs':[g]})
        observations=[]
        for row in bands:
            parts=[]
            for g in sorted(row['gs'],key=lambda g:g['bbox'][0]):
                if not parts or g['bbox'][0]-(parts[-1][-1]['bbox'][0]+parts[-1][-1]['bbox'][2])>max(12,g['size']*2):parts.append([])
                parts[-1].append(g)
            for part in parts:
                words=[];current=[]
                for g in part:
                    if current and g['bbox'][0]-(current[-1]['bbox'][0]+current[-1]['bbox'][2])>g['size']*.18:
                        words.append(current);current=[]
                    current.append(g)
                if current:words.append(current)
                tokens=[{'text':''.join(g['text'] for g in ws),'bbox':union([g['bbox'] for g in ws])} for ws in words]
                observations.append({'id':f'obs-{len(observations)}','text':' '.join(t['text'] for t in tokens),'bbox':union([g['bbox'] for g in part]),'tokens':tokens,'glyph_ids':[g['id'] for g in part],'size':statistics.median(g['size'] for g in part),'confidence':None})
        raw.append({'schema':'mpdf-native-glyph-raw/2','extractor':pdf.RENDERER,'source':str(Path(source).resolve()),'source_sha256':digest,'page_index':index,'page_count':len(doc),'width':page.rect.width,'height':page.rect.height,'source_kind':'native_text','bbox_convention':'visible_top_left_points_xywh','glyphs':glyphs,'observations':observations,'state':'inspected','rotation':page.rotation,'cropbox':list(page.cropbox)})
    doc.close();save(output,raw);return time.perf_counter()-start

def title_geometry(observation):
    """Use retained token boxes, never estimate glyph widths from string length."""
    tokens=observation.get('tokens',[])
    if not tokens:return None,None
    first=tokens[0]['text']
    count=0
    if re.fullmatch(r'(?:PART|BOOK|CHAPTER|SECTION)',first,re.I):
        if len(tokens)>2 and re.fullmatch(r'[0-9IVXLCDMA-Z]+[.)]?',tokens[1]['text']):count=2
    elif re.fullmatch(r'(?:[0-9lIi]+(?:\.+[0-9lIi]+)*[.)]?|[IVXLCDM]+[.)]?|[A-Z][.)]|[a-z][)])',first):
        count=1
    if count and len(tokens)<=count:return None,None
    return tokens[count]['bbox'][0],bbox(union([t['bbox'] for t in tokens[:count]])) if count else None

def project(raw, raw_path, numeric_reads=None):
    """Generic geometry grouping. Raw OCR and glyph observations stay immutable."""
    w,h=raw['width'],raw['height'];kind=raw.get('source_kind','apple_vision_fast')
    def convert(b):
        return [b[0]*w,(1-b[1]-b[3])*h,b[2]*w,b[3]*h] if raw['bbox_convention']=='normalized_bottom_left_xywh' else b
    obs=[]
    for o in raw['observations']:
        o=copy.deepcopy(o);o['bbox']=convert(o['bbox'])
        for t in o.get('tokens',[]):t['bbox']=convert(t['bbox'])
        obs.append(o)
    if not obs:
        return dict(schema=SCHEMA,source=raw['source'],source_sha256=raw['source_sha256'],page_index=raw['page_index'],page_count=raw['page_count'],width=w,height=h,source_roi=bbox([0,0,w,h]),coordinate_transform=[1,0,0,1,0,0],raw_path=str(raw_path),lines=[],diagnostics=['empty_observations'])
    median=statistics.median(o['bbox'][3] for o in obs)
    # Numeric tokens form repeated right-edge lanes; standalone title years have
    # neither the repetition nor a sizeable gap from the preceding title token.
    numeric=[]
    for o in obs:
        ts=o.get('tokens',[])
        if ts and number(ts[-1]['text']):
            t=ts[-1];b=t['bbox'];gap=b[0]-(ts[-2]['bbox'][0]+ts[-2]['bbox'][2]) if len(ts)>1 else w
            if gap>median*.9:numeric.append((o,t))
    lanes=[]
    for o,t in numeric:
        edge=t['bbox'][0]+t['bbox'][2]
        if sum(abs(edge-(v['bbox'][0]+v['bbox'][2]))<w*.018 for _,v in numeric)>=2 and edge>w*.3:
            if not any(abs(edge-v)<w*.025 for v in lanes):lanes.append(edge)
    lanes.sort()
    def lane_of(o):
        x=o['bbox'][0]
        return next((i for i,e in enumerate(lanes) if x<e+w*.006),len(lanes))
    numbers=[];titles=[]
    for o in obs:
        ts=o.get('tokens',[]);candidate=None
        if ts:
            t=ts[-1];edge=t['bbox'][0]+t['bbox'][2]
            if any(abs(edge-v)<w*.018 for v in lanes) and (number(t['text']) or len(ts)==1 and len(t['text'])<=5):candidate=t
        if candidate:
            numbers.append(dict(text=candidate['text'],bbox=candidate['bbox'],raw_id=o['id'],confidence=o.get('confidence'),lane=lane_of(o)))
            rest=ts[:-1]
            if not rest:continue
            o=copy.deepcopy(o);o['text']=' '.join(t['text'] for t in rest);o['bbox']=union([t['bbox'] for t in rest]);o['tokens']=rest
        titles.append(o)
    # Attach a page token only to a unique vertically aligned title, within lane.
    for o in titles:o.update(page_token=None,number_ids=[],ambiguous=False,lane=lane_of(o))
    for n in numbers:
        cy=n['bbox'][1]+n['bbox'][3]/2
        candidates=[o for o in titles if o['lane']==n['lane'] and o['bbox'][0]<n['bbox'][0] and abs(o['bbox'][1]+o['bbox'][3]/2-cy)<max(o['bbox'][3],n['bbox'][3])*.65]
        candidates.sort(key=lambda o:abs(o['bbox'][1]+o['bbox'][3]/2-cy))
        if candidates:
            o=candidates[0]
            if o['page_token'] or len(candidates)>1 and abs(candidates[1]['bbox'][1]+candidates[1]['bbox'][3]/2-cy)-abs(o['bbox'][1]+o['bbox'][3]/2-cy)<median*.2:o['ambiguous']=True
            o['page_token']=n;o['number_ids'].append(n['raw_id'])
    # Mandatory lane second read is applied before grouping, including rows
    # whose full-page recognition omitted the numeral entirely.
    routed={}
    for origin_id,r in (numeric_reads or {}).items():
        new=r.get('candidate')
        if not new:continue
        nb=new['bbox'];cy=nb[1]+nb[3]/2
        origin=next((o for o in titles if o['id']==origin_id),None)
        if origin is None:continue
        eligible=[o for o in titles if o['lane']==origin['lane'] and o['bbox'][0]<nb[0] and
                  abs(o['bbox'][1]+o['bbox'][3]/2-cy)<max(o['bbox'][3],nb[3])*.75]
        if not eligible:continue
        nearest=min(eligible,key=lambda o:abs(o['bbox'][1]+o['bbox'][3]/2-cy))
        routed.setdefault(nearest['id'],[]).append(r)
    for o in titles:
        o['numeric_read']=(numeric_reads or {}).get(o['id'])
        proposals=routed.get(o['id'],[])
        values={number(r['candidate']['text']) for r in proposals}
        if len(values)>1:o['ambiguous']=True;continue
        if proposals:
            r=max(proposals,key=lambda r:r['candidate']['confidence']);new=r['candidate'];old=o['page_token']
            if old and number(old['text']) and number(new['text'])!=number(old['text']):o['ambiguous']=True
            else:
                o['page_token']=dict(text=new['text'],bbox=new['bbox'],raw_id=r['evidence_ref'],confidence=new['confidence'],lane=o['lane'])
                o['number_ids'].append(r['evidence_ref'])
    visual_rows=copy.deepcopy(titles)
    titles.sort(key=lambda o:(o['lane'],o['bbox'][1],o['bbox'][0]))
    groups=[]
    def section(s):
        return bool(re.match(r'^(?:(?:PART|BOOK|CHAPTER|SECTION)\b|[\dIl]+(?:\.+[\dIl]+)*[.)]?\s|[-.]*\d[\d.\-)]*\s|[A-ZlI]{1,2}[.)]\s|[a-z]{1,2}[)]\s)',s,re.I))
    headers={'contents','table of contents','inhalt','inhaltsverzeichnis','sommaire','table des matières','目录','目次'}
    for o in titles:
        b=o['bbox'];key=o['text'].strip().lower()
        is_header=key in headers and b[1]<h*.3 and not o['page_token']
        furniture=b[1]<h*.13 and (key.startswith('page ') or number(key))
        join=False
        if groups and not is_header and not furniture:
            prev=groups[-1][-1];pb=prev['bbox'];gap=b[1]-pb[1]-pb[3]
            has_number=any(v['page_token'] and number(v['page_token']['text']) for v in groups[-1])
            explicit_named=bool(re.match(r'^(Preface|Introduction|Conclusion|Notes|Bibliography|Index|Indices)\b',o['text'],re.I))
            letters=[c for c in prev['text'] if c.isalpha()]
            author_line=bool(letters) and sum(c.isupper() for c in letters)/len(letters)>.7 and not has_number
            if explicit_named and author_line:explicit_named=False
            aligned=abs(b[0]-groups[-1][0]['bbox'][0])<w*.045
            # A legal page terminates by default. Only clear, short continuation
            # at its text indent (or a hyphenated wrap) may cross that boundary.
            continuation=(not o['page_token'] and not section(o['text']) and not explicit_named and
                (prev['text'].rstrip().endswith('-') or
                 abs(b[0]-groups[-1][0]['bbox'][0])<median*.5 and b[2]<groups[-1][0]['bbox'][2]*.8))
            join=(prev['lane']==o['lane'] and not prev.get('header') and aligned and
                  -.2*median<=gap<median*.9 and not section(o['text']) and not explicit_named and
                  (continuation if has_number else True))
        o['header']=is_header or bool(furniture)
        if join:groups[-1].append(o)
        else:groups.append([o])
    lines=[]
    for i,group in enumerate(groups):
        nums=[o['page_token'] for o in group if o['page_token']];n=nums[0] if nums else None
        text=' '.join(o['text'] for o in group)
        if n:text+=' '+n['text']
        bounds=union([o['bbox'] for o in group]+([n['bbox']] if n else []))
        ambiguous=any(o['ambiguous'] for o in group) or bool(n and not number(n['text']))
        # Unknown glyph-like numeric tokens retained as review, never coerced.
        title_x,label_box=title_geometry(group[0])
        lines.append(dict(title_start_x=title_x,label_bbox=label_box,id=f'p{raw["page_index"]}-g{i}',group_id=f'g{i}',raw_ids=list(dict.fromkeys([o['id'] for o in group]+sum([o['number_ids'] for o in group],[]))),text=text,bbox=bbox(bounds),size_proxy=group[0].get('size',group[0]['bbox'][3]),confidence=min([o['confidence'] for o in group if o.get('confidence') is not None],default=None),source_kind=kind,state='ambiguous' if ambiguous else raw.get('state','locally_recognized'),printed_candidate=n['text'] if n and number(n['text']) else None,number_bbox=bbox(n['bbox']) if n else None,number_confidence=n['confidence'] if n else None,number_raw=n['text'] if n else None))
    diagnostics=[]
    if numbers and sum(o['page_token'] is not None for o in titles)<len(numbers)*.8:diagnostics.append('title_number_lane_unresolved')
    if not lanes:diagnostics.append('no_reliable_number_lane')
    return dict(schema=SCHEMA,source=raw['source'],source_sha256=raw['source_sha256'],page_index=raw['page_index'],page_count=raw['page_count'],width=w,height=h,source_roi=bbox([0,0,w,h]),coordinate_transform=[1,0,0,1,0,0],raw_path=str(Path(raw_path).resolve()),lines=lines,visual_rows=visual_rows,number_lanes=lanes,numeric_read_status='complete' if numeric_reads is not None else 'pending' if kind=='apple_vision_fast' else 'native',diagnostics=diagnostics)

def compile_pages(evidence,binary,output):
    began=time.perf_counter()
    subprocess.run([binary,'compile',str(evidence),str(output)],check=True)
    table=load(output);pages=load(evidence);byid={l['id']:l for p in pages for l in p['lines']}
    for e in table['entries']+table.get('source_entries',[]):
        ls=[byid[i] for i in e['evidence_ids']];ns=[l for l in ls if l.get('number_raw')]
        if ns and ns[0].get('number_confidence') is not None and ns[0]['number_confidence']<.8:e['review_reasons'].append('printed_low_confidence')
        if ns and not number(ns[0]['number_raw']):e['review_reasons'].append('printed_pattern_invalid')
        e['state']='needs_review';e['source_url']=Path(table['source']).as_uri()+f'#page={e["source_page"]+1}&zoom=150,{e["source_bbox"]["x"]:.0f},{e["source_bbox"]["y"]:.0f}'
        # Book-level editable review: do not mark every node wrong merely
        # because the book has not yet been accepted by the user.
        if any(p.get('numeric_read_status')=='pending' for p in pages if p['page_index']==e['source_page']):e['review_reasons'].append('numeric_lane_read_pending')
    table['evidence_path']=str(Path(evidence).resolve());table['compile_process_seconds']=time.perf_counter()-began
    table['fallback']={'requested_pages':[p['page_index'] for p in pages if set(p['diagnostics']) & {'double_column_reading_order','bbox_fragmentation','unstable_physical_line_groups'}], 'executed_pages':0,'seconds':0,'status':'not_run'}
    # This enriches a NEW output created by this invocation, never input evidence.
    Path(output).write_text(json.dumps(table,ensure_ascii=False,indent=2));return table

# Pagination requires independent native page-label evidence, or explicit manual
# anchors. No source outlines, title guesses, or default physical label offsets.
def paginate(table, anchors):
    t=copy.deepcopy(table)
    for a in anchors:
        if a.get('state')!='inspected' or not a.get('evidence_ref'):raise ValueError('mapping anchor requires inspected evidence reference')
        if not 0<=a['pdf_page']<t['page_count']:raise ValueError('anchor outside PDF')
    for e in t['entries']+t.get('source_entries',[]):
        matches=[a for a in anchors if a['family']==e['printed_family'] and a['printed_value']==e['printed_value']]
        # Exact label anchors only. Offset extrapolation needs a separately
        # inspected segment, never inferred from a lone TOC numeral.
        if len({a['pdf_page'] for a in matches})==1:
            e['target_pdf_page']=matches[0]['pdf_page'];e['pagination_evidence']=matches
            e['review_reasons']=[r for r in e['review_reasons'] if r!='pagination_uninspected']
            if any(a.get('kind')=='native_header_candidate' for a in matches):
                e['review_reasons'].append('native_header_mapping_requires_inspection')
        e['state']='needs_review' if e['review_reasons'] else 'ready'
    return t

def native_anchors(source):
    doc=pdf.Document(source);anchors=[]
    for p in doc:
        # Explicit PDF PageLabels are evidence; absent labels return empty.
        label=p.get_label();n=number(label)
        if n:anchors.append(dict(family=n[0],printed_value=n[1],pdf_page=p.number,state='inspected',evidence_ref=f'pdf-page-label:{p.number}:{label}'))
    return anchors

def native_header_anchors(source, excluded_pages=()):
    """Measured native header candidates, with explicit uncertainty on mapping.
    Repeated top-margin number lanes or literal 'Page N'; never OCR a body page.
    These candidates require inspection and are NOT equivalent to PDF PageLabels.
    """
    doc=pdf.Document(source);candidates=[]
    for p in doc:
        if p.number in excluded_pages:continue
        words=p.words()
        for w in words:
            n=number(w[4])
            if not n:continue
            explicit=any(v[4].lower()=='page' and abs(v[1]-w[1])<3 and 0<w[0]-v[2]<20 for v in words)
            if explicit and w[1]<p.rect.height*.15 or w[1]<p.rect.height*.065 and (w[0]<p.rect.width*.1 or w[2]>p.rect.width*.9):
                candidates.append(dict(family=n[0],printed_value=n[1],pdf_page=p.number,state='inspected',evidence_ref=f'native-header:{p.number}:{w[:4]}:{w[4]}',kind='native_header_candidate',lane=round(w[0]/p.rect.width*20),explicit=explicit))
    return [a for a in candidates if a['explicit'] or len({b['pdf_page'] for b in candidates if b['lane']==a['lane']})>=3]

def validate_table(t,ready=False):
    if t['schema']!='mpdf-bookmark-table/1':raise ValueError('unsupported table schema')
    seen={};stack=[]
    for e in t['entries']:
        if not e['id'] or e['id'] in seen or not e['title'].strip() or any(ord(c)<32 for c in e['title']):raise ValueError('invalid id/title')
        parent=e['parent']
        if parent is not None and parent not in seen:raise ValueError('parent must precede child; no dangling/cyclic tree')
        expected=seen[parent]['level']+1 if parent else 0
        if e['level']!=expected:raise ValueError('parent/level mismatch')
        while stack and stack[-1]!=parent:stack.pop()
        if parent and not stack:raise ValueError('tree must use contiguous preorder')
        stack.append(e['id']);seen[e['id']]=e
        target=e['target_pdf_page']
        if target is not None and (type(target)!=int or not 0<=target<t['page_count']):raise ValueError('target outside PDF')
        if ready and (target is None or e.get('state') not in ('ready','manually_confirmed') or e.get('review_reasons')):raise ValueError(f'unresolved entry: {e["id"]}')
    if ready and not t['entries']:raise ValueError('empty bookmark tree')

def edit(t,patch):
    if patch.get('source_sha256')!=t['source_sha256']:raise ValueError('patch source mismatch')
    out=copy.deepcopy(t)
    for op in patch['operations']:
        kind=op['op'];entries=out['entries'];item=next((e for e in entries if e['id']==op.get('id')),None)
        if kind=='add':
            e=copy.deepcopy(op['entry']);e.setdefault('review_reasons',['manual_confirmation_required']);e.setdefault('state','needs_review');entries.insert(op.get('position',len(entries)),e)
        elif item is None:raise ValueError('unknown entry')
        elif kind=='delete':
            # Explicit subtree deletion; no hidden child promotion.
            ids={item['id']}
            for e in entries:
                if e['parent'] in ids:ids.add(e['id'])
            out['entries']=[e for e in entries if e['id'] not in ids]
        elif kind=='update':
            if set(op['fields'])-{'title','parent','target_pdf_page','printed_page','printed_value','printed_family'}:raise ValueError('unsupported edit field')
            item.update(op['fields']);item['state']='needs_review';item['review_reasons']=['manual_confirmation_required']
        elif kind=='confirm':
            if not op.get('evidence_ref'):raise ValueError('confirmation requires evidence reference')
            item['review_reasons']=[];item['state']='manually_confirmed';item['manual_evidence']=op['evidence_ref']
        else:raise ValueError('unknown operation')
    # Canonical preorder permits reparenting; preserve siblings' existing order.
    allentries=out['entries'];byid={e['id']:e for e in allentries}
    if len(byid)!=len(allentries):raise ValueError('duplicate ids')
    ordered=[];visited=set()
    def walk(parent,level):
        for e in allentries:
            if e['parent']==parent:
                if e['id'] in visited:raise ValueError('cycle')
                visited.add(e['id']);e['level']=level;ordered.append(e);walk(e['id'],level+1)
    walk(None,0)
    if len(ordered)!=len(allentries):raise ValueError('cyclic/dangling parent')
    out['entries']=ordered;out.setdefault('edit_history',[]).append(patch);validate_table(out);return out

def original_objects(reader):
    from pypdf.generic import IndirectObject
    refs={(generation,number) for generation,items in reader.xref.items() for number in items if number and generation!=65535}
    refs.update((0,number) for number in reader.xref_objStm)
    return [(generation,number,reader.get_object(IndirectObject(number,generation,reader))) for generation,number in sorted(refs)]


def object_fingerprint(obj, omit=()):
    from pypdf.generic import IndirectObject, DictionaryObject, ArrayObject, StreamObject
    if isinstance(obj,IndirectObject):return ('ref',obj.idnum,obj.generation)
    if isinstance(obj,DictionaryObject):
        return ('dict',tuple(sorted((str(k),object_fingerprint(v)) for k,v in obj.items() if k not in omit)),getattr(obj,'_data',None) if isinstance(obj,StreamObject) else None)
    if isinstance(obj,ArrayObject):return ('array',tuple(object_fingerprint(v) for v in obj))
    import io
    out=io.BytesIO();obj.write_to_stream(out);return out.getvalue()


def contains_signature(obj):
    from pypdf.generic import DictionaryObject, ArrayObject
    if isinstance(obj,DictionaryObject):
        return '/ByteRange' in obj or any(contains_signature(value) for value in obj.values())
    if isinstance(obj,ArrayObject):return any(contains_signature(value) for value in obj)
    return False  # Indirect objects are checked separately by original_objects().


def export(t,output,cancelled=None):
    """Append only outlines; verify original objects and independent PDFium output."""
    from pypdf import PdfReader
    from outline_writer import OutlineWriter
    from pypdf.generic import Fit, IndirectObject
    if cancelled:cancelled()
    began=time.perf_counter();validate_table(t,True);source=t['source']
    if sha(source)!=t['source_sha256']:raise ValueError('source PDF changed')
    output=Path(output).resolve()
    if output.exists() or output==Path(source).resolve():raise ValueError('output exists/source collision')
    reader=PdfReader(source)
    if reader.is_encrypted:raise ValueError('目录处理暂不支持加密 PDF，请先保存不加密副本。')
    objects=original_objects(reader)
    if any(contains_signature(obj) for _,_,obj in objects):raise ValueError('signed PDF requires an explicit signature policy')
    original_page_keys={(g,n):set(obj) for g,n,obj in objects if isinstance(obj,dict) and obj.get('/Type')=='/Page'}
    catalog=reader.trailer.raw_get('/Root')
    # Snapshot before PdfReader.pages resolves inherited page attributes in memory.
    objects=[(g,n,object_fingerprint(obj,('/Outlines',) if (n,g)==(catalog.idnum,catalog.generation) else ())) for g,n,obj in objects]
    if len(reader.pages)!=t['page_count']:raise ValueError('page count changed')
    writer=OutlineWriter(source,incremental=True)
    if '/Outlines' in writer.root_object:del writer.root_object['/Outlines']
    refs={}
    for e in t['entries']:
        refs[e['id']]=writer.add_outline_item(e['title'],e['target_pdf_page'],parent=refs[e['parent']] if e['parent'] else None,fit=Fit.xyz(left=0,top=float(reader.pages[e['target_pdf_page']].mediabox.top),zoom=0))
    # pypdf materializes inherited page attributes when cloning. Restore absent
    # local keys so the incremental revision never rewrites a source page.
    for (generation,number),keys in original_page_keys.items():
        page=writer.get_object(IndirectObject(number,generation,writer))
        for key in ('/Resources','/MediaBox','/CropBox','/Rotate'):
            if key not in keys and key in page:del page[key]
    fd,tmp=tempfile.mkstemp(prefix='.bookmark-',suffix='.pdf',dir=output.parent);os.close(fd)
    try:
        writer.write(tmp);writer.close()
        with open(source,'rb') as original,open(tmp,'rb') as changed:
            while True:
                block=original.read(1024*1024)
                if not block:break
                if changed.read(len(block))!=block:raise ValueError('original PDF bytes changed')
        reopened=PdfReader(tmp)
        for generation,number,obj in objects:
            if cancelled:cancelled()
            omit=('/Outlines',) if (number,generation)==(catalog.idnum,catalog.generation) else ()
            other=reopened.get_object(IndirectObject(number,generation,reopened))
            if obj!=object_fingerprint(other,omit):raise ValueError(f'original object changed: {number}')
        actual=[]
        def walk(items,level=0):
            for item in items:
                if isinstance(item,list):walk(item,level+1)
                else:actual.append((level,item.title,reopened.get_destination_page_number(item)))
        walk(reopened.outline)
        expected=[(e['level'],e['title'],e['target_pdf_page']) for e in t['entries']]
        if actual!=expected:raise ValueError('reopened outline mismatch')
        with pdf.Document(source) as before,pdf.Document(tmp) as after:
            if len(before)!=len(after):raise ValueError('reopened page count differs')
            # PDFium independently resolves destinations from the written outlines.
            independent=[(item.level,item.get_title(),item.get_dest().get_index() if item.get_dest() else None) for item in after.pdf.get_toc()]
            if independent!=expected:raise ValueError('PDFium destination mismatch')
            for a,b in zip(before,after):
                if cancelled:cancelled()
                if (a.rect,a.cropbox,a.rotation)!=(b.rect,b.cropbox,b.rotation):raise ValueError('page geometry changed')
                if a.glyphs()!=b.glyphs():raise ValueError('native glyph/text changed')
                if a.render(alpha=True).tobytes()!=b.render(alpha=True).tobytes():raise ValueError('render changed')
        reopened.close()
        if sha(source)!=t['source_sha256']:raise ValueError('source changed during export')
        if cancelled:cancelled()
        os.link(tmp,output)
        return dict(schema='mpdf-bookmark-only-validation/2',writer='pypdf-incremental',renderer=pdf.RENDERER,source_sha256=t['source_sha256'],output_sha256=sha(output),pages_checked=len(reader.pages),original_objects_checked=len(objects),original_bytes_preserved=True,render_dpi=72,native_glyphs_unchanged=True,streams_unchanged=True,outline_entries=len(expected),actual_destinations_checked=len(expected),seconds=time.perf_counter()-began)
    finally:
        os.unlink(tmp);reader.close()


def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='cmd',required=True)
    q=sub.add_parser('native');q.add_argument('source');q.add_argument('output');q.add_argument('--pages',required=True,help='one-based PDF pages, comma separated')
    q=sub.add_parser('project');q.add_argument('output');q.add_argument('raw',nargs='+')
    q=sub.add_parser('compile');q.add_argument('evidence');q.add_argument('output');q.add_argument('--compiler',default='target/debug/mpdf-bookmarks')
    q=sub.add_parser('map');q.add_argument('table');q.add_argument('output');q.add_argument('--anchors')
    q=sub.add_parser('edit');q.add_argument('table');q.add_argument('patch');q.add_argument('output')
    q=sub.add_parser('export');q.add_argument('table');q.add_argument('output');q.add_argument('receipt')
    a=p.parse_args()
    if a.cmd=='native':print({'native_seconds':native(a.source,[int(i)-1 for i in a.pages.split(',')],a.output)})
    elif a.cmd=='project':
        pages=[]
        for path in a.raw:
            data=load(path)
            for raw in data if isinstance(data,list) else [data]:pages.append(project(raw,path))
        save(a.output,pages)
    elif a.cmd=='compile':compile_pages(a.evidence,a.compiler,a.output)
    elif a.cmd=='map':
        table=load(a.table);save(a.output,paginate(table,load(a.anchors) if a.anchors else native_anchors(table['source'])))
    elif a.cmd=='edit':save(a.output,edit(load(a.table),load(a.patch)))
    elif a.cmd=='export':
        if Path(a.receipt).exists():raise ValueError('receipt exists')
        save(a.receipt,export(load(a.table),a.output))
if __name__=='__main__':main()
