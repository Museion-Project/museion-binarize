"""Independent bounded local pixels/text observations; no evaluation imports."""
import csv
import hashlib
import io
from PIL import Image
import re
import subprocess
import time
from pathlib import Path
import fitz


def digest(data):
    return hashlib.sha256(data).hexdigest()


def observe_candidate(request, *, layouts, cached_pages=(),document_relations=()):
    """Explicit P5 complete-source candidate; never implicitly changes admission.

    Source layouts/raw receipts are mandatory evidence. This path starts no
    reader, accepts no prediction or reference counts and grants no export.
    """
    from .complete_source import observe_complete
    return observe_complete(request, layouts=layouts, cached_pages=cached_pages,document_relations=document_relations)


def observe(request, timeout=60):
    started=time.monotonic()
    source=Path(request['input_pdf'])
    if digest(source.read_bytes()) != request['input_sha256']:
        raise ValueError('coverage source identity changed')
    results=[]
    with fitz.open(source) as doc:
        for image in request['images']:
            number=image['page_number']; page=doc[number-1]
            raw=Path(image['path']).read_bytes()
            if digest(raw)!=image['sha256']:raise ValueError('coverage image changed')
            dpi=image.get('dpi',150)
            clip=fitz.Rect(image['crop_bbox']) if image['kind']=='crop' else None
            pixels=page.get_pixmap(matrix=fitz.Matrix(dpi/72,dpi/72),clip=clip,alpha=False).tobytes('png')
            verification_dpi=dpi
            if digest(pixels)!=image['sha256']:
                # Historical renderer metadata may disagree with actual image dimensions.
                # Derive only a source rendering scale, then require exact decoded pixels.
                original=Image.open(io.BytesIO(raw)).convert('RGB')
                width=(clip.width if clip else page.rect.width)
                inferred=round(original.width*72/width)
                rendered=page.get_pixmap(matrix=fitz.Matrix(inferred/72,inferred/72),clip=clip,alpha=False)
                if original.size!=(rendered.width,rendered.height) or original.tobytes()!=rendered.samples:
                    raise ValueError('coverage image/source/page pixels differ')
                verification_dpi=inferred
            dpi=verification_dpi
            lines=[]; method='native-source-text'; error=None; elapsed=0
            for block in page.get_text('dict',clip=clip)['blocks']:
                for line in block.get('lines',[]):
                    text=''.join(s['text'] for s in line['spans']).strip()
                    if text:lines.append(dict(text=text,bbox=list(line['bbox']),confidence=None))
            if sum(len(l['text']) for l in lines)<80:
                method='local-tesseract-pixels'; t=time.monotonic()
                try:
                    process=subprocess.run(['tesseract',str(image['path']),'stdout','-l','eng','--psm','6','tsv'],capture_output=True,text=True,timeout=timeout,check=True)
                    grouped={}
                    for item in csv.DictReader(io.StringIO(process.stdout),delimiter='\t'):
                        if not item.get('text','').strip() or float(item['conf'])<0:continue
                        key=tuple(item[k] for k in ('block_num','par_num','line_num'))
                        grouped.setdefault(key,[]).append(item)
                    lines=[]
                    for group in grouped.values():
                        x=min(int(w['left']) for w in group);y=min(int(w['top']) for w in group)
                        right=max(int(w['left'])+int(w['width']) for w in group);bottom=max(int(w['top'])+int(w['height']) for w in group)
                        scale=72/dpi;ox=clip.x0 if clip else 0;oy=clip.y0 if clip else 0
                        lines.append(dict(text=' '.join(w['text'] for w in group),bbox=[ox+x*scale,oy+y*scale,ox+right*scale,oy+bottom*scale],confidence=sum(float(w['conf']) for w in group)/len(group)))
                except (OSError,subprocess.SubprocessError) as exc:
                    lines=[];error=type(exc).__name__
                elapsed=time.monotonic()-t
            # Native extraction may split right-aligned folios into separate blocks.
            # Join only geometrically aligned same-baseline fragments from source.
            if method=='native-source-text':
                combined=[]
                for line in sorted(lines,key=lambda l:(l['bbox'][1],l['bbox'][0])):
                    match=next((l for l in combined if abs(l['bbox'][3]-line['bbox'][3])<2 and l['bbox'][2]<=line['bbox'][0]),None)
                    if match:
                        match['text']+=' '+line['text'];match['bbox'][2]=line['bbox'][2];match['bbox'][0]=min(match['bbox'][0],line['bbox'][0])
                    else:combined.append(dict(line))
                lines=combined
            # Terminal printed labels identify independent TOC regions, not model claims.
            units=[l for l in lines if re.search(r'\s(?:[0-9]+(?:[-–][0-9]+)?|[ivxlcdmIVXLCDM]+)\s*$',l['text']) and len(l['text'].split())>=2 and not re.match(r'^(?:contents|table des mati[eè]res|inhaltsverzeichnis)\b',l['text'].replace('﻿','').casefold())]
            results.append(dict(page_number=number,source_sha256=request['input_sha256'],image_sha256=image['sha256'],method=method,declared_dpi=image.get('dpi'),verified_render_dpi=verification_dpi,readability='observed' if units else 'unknown',regions=units,lines=lines,error=error,probe_seconds=elapsed,limitation='local observation may miss regions; observed is not human verified readability'))
    return dict(schema='toc-independent-coverage/1',source_sha256=request['input_sha256'],pages=results,wall_seconds=time.monotonic()-started,network_sent=False)
