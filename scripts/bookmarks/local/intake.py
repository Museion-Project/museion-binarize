#!/usr/bin/env python3
"""Single local intake command: selected TOC pages -> native or Vision -> editor.
Explicit TOC selection is the current MVP intake; no full-book OCR or cloud.
"""
import argparse,time,subprocess
from pathlib import Path
import fitz
from bookmarks import native,load,save,project,compile_pages,paginate,native_anchors,native_header_anchors
from editor import write
p=argparse.ArgumentParser();p.add_argument('source');p.add_argument('output');p.add_argument('--pages',required=True,help='1-based comma-separated TOC pages');p.add_argument('--vision-worker',required=True);p.add_argument('--compiler',default='target/debug/mpdf-bookmarks');a=p.parse_args()
start=time.perf_counter();out=Path(a.output).resolve();out.mkdir();source=str(Path(a.source).resolve());indices=[int(n)-1 for n in a.pages.split(',')]
native(source,indices,out/'native-raw.json');raw=load(out/'native-raw.json');evidence=[];requests=[];doc=fitz.open(source)
for r in raw:
    if r['glyphs']:evidence.append(project(r,out/'native-raw.json'))
    else:
        t=time.perf_counter();page=doc[r['page_index']];image=out/f'p{page.number}.png';page.get_pixmap(matrix=fitz.Matrix(150/72,150/72)).save(image)
        requests.append(dict(id=f'p{page.number}',source=source,source_sha256=r['source_sha256'],page_index=page.number,page_count=len(doc),width=page.rect.width,height=page.rect.height,image_path=str(image),render_seconds=time.perf_counter()-t))
if requests:
    save(out/'vision-requests.json',requests);subprocess.run([a.vision_worker,str(out/'vision-requests.json'),str(out/'vision')],check=True,timeout=120)
    from numeric_lane import run as read_numeric_lanes
    evidence.extend(read_numeric_lanes(sorted((out/'vision').glob('*.json')),out/'numeric-lane',a.vision_worker))
evidence.sort(key=lambda p:p['page_index']);save(out/'evidence.json',evidence)
table=compile_pages(out/'evidence.json',a.compiler,out/'unmapped-table.json');anchors=native_anchors(source) or native_header_anchors(source,indices);save(out/'anchors.json',anchors);table=paginate(table,anchors);save(out/'table.json',table);write(out/'table.json',out/'editor.html')
save(out/'intake.json',dict(total_seconds=time.perf_counter()-start,native_pages=len(raw)-len(requests),vision_pages=len(requests),surya_pages=0,status='editable_needs_review',toc_selection='explicit_user_page_list'))
