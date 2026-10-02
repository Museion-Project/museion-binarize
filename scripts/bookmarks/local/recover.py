#!/usr/bin/env python3
"""PDF-only explicit TOC recovery. No pagination, body-heading model or PDF writer.

Every page participates in discovery. Only explicit navigation spans are compiled.
All raw recognition and decision evidence is preserved under the output directory.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

import fitz

from bookmarks import compile_pages, load, native, save, sha
from numeric_lane import collect, run as numeric_run
from recovery_layout import features, restrict_region, source_projection, spans, visible_prefix


def native_summary(page, source, digest, page_count):
    observations = []
    for block in page.get_text('dict')['blocks']:
        for line in block.get('lines', []):
            text = ''.join(s['text'] for s in line['spans']).strip()
            rect = fitz.Rect(line['bbox'])*page.rotation_matrix
            if text:
                observations.append(dict(id=f'obs-{len(observations)}', text=text,
                                         bbox=[rect.x0,rect.y0,rect.width,rect.height]))
    return dict(schema='mpdf-native-discovery/1',source=source,source_sha256=digest,
                page_index=page.number,page_count=page_count,width=page.rect.width,
                height=page.rect.height,bbox_convention='visible_top_left_points_xywh',
                observations=observations,source_kind='native_text',state='inspected')


def request(page, source, digest, count, image, dpi):
    started=time.monotonic()
    page.get_pixmap(matrix=fitz.Matrix(dpi/72,dpi/72),alpha=False).save(image,jpg_quality=90)
    return dict(id=f'p{page.number}',source=source,source_sha256=digest,
                page_index=page.number,page_count=count,width=page.rect.width,height=page.rect.height,
                image_path=str(image.resolve()),render_seconds=time.monotonic()-started,
                raster_dpi=dpi)


def recover(source, output, worker, compiler):
    started=time.monotonic(); source=str(Path(source).resolve()); output=Path(output).resolve()
    output.mkdir(parents=True,exist_ok=True)
    rawdir=output/'discovery';rawdir.mkdir();digest=sha(source)
    doc=fitz.open(source);decisions=[None]*len(doc);native_pages=set();pending=[]

    def flush():
        if not pending:return
        batch=rawdir/f'batch-{pending[0]["page_index"]}';batch.mkdir()
        save(batch/'requests.json',pending)
        subprocess.run([worker,str(batch/'requests.json'),str(batch/'vision')],check=True,timeout=180)
        for req in pending:
            raw=load(batch/'vision'/(req['id']+'.json'))
            decisions[req['page_index']]=features(raw)|{'source_kind':'apple_vision_fast','raw_path':str(batch/'vision'/(req['id']+'.json')),'recognition_state':raw['state']}
        pending.clear()

    for page in doc:
        summary=native_summary(page,source,digest,len(doc))
        text=' '.join(r['text'] for r in summary['observations'])
        if sum(c.isalpha() for c in text)>=15 and sum(not c.isspace() for c in text)>=30:
            path=rawdir/f'native-{page.number}.json';save(path,summary)
            decisions[page.number]=features(summary)|{'source_kind':'native_text','raw_path':str(path),'recognition_state':'inspected'}
            native_pages.add(page.number)
        else:
            pending.append(request(page,source,digest,len(doc),rawdir/f'p{page.number}.jpg',150))
            if len(pending)==16:flush()
    flush();save(output/'discovery.json',decisions)
    groups=spans(decisions);save(output/'discovered-spans.json',groups)
    predictions=[];errors=[]
    for gi,indices in enumerate(groups):
        gout=output/f'group-{gi}';gout.mkdir();evidence=[];refs={}
        for index in indices:
            pout=gout/f'p{index}';pout.mkdir()
            try:
                if index in native_pages:
                    native(source,[index],pout/'native-raw.json');raw=load(pout/'native-raw.json')[0]
                    region=features(raw)['region_y'];save(pout/'selected-region.json',dict(region_y=region))
                    projected,full=source_projection(restrict_region(raw,region),pout/'native-raw.json')
                else:
                    req=request(doc[index],source,digest,len(doc),pout/'page.jpg',150)
                    save(pout/'requests.json',[req])
                    subprocess.run([worker,str(pout/'requests.json'),str(pout/'vision')],check=True,timeout=120)
                    rawpath=pout/'vision'/f'p{index}.json';raw=load(rawpath)
                    if raw['state']=='failed':raise RuntimeError('selected-page recognition failed')
                    numeric_run([rawpath],pout/'numeric',worker,max_crops=4000)
                    reads,_=collect(pout/'numeric/vision')
                    region=features(raw)['region_y'];save(pout/'selected-region.json',dict(region_y=region))
                    projected,full=source_projection(restrict_region(raw,region),rawpath,reads.get(str(rawpath.resolve()),{}))
                evidence.append(projected);refs.update(full)
            except Exception as e:
                errors.append(dict(group=gi,page_index=index,error=str(e)))
        if not evidence:continue
        save(gout/'evidence.json',evidence);save(gout/'printed-references.json',refs)
        try:
            table=compile_pages(gout/'evidence.json',compiler,gout/'unmapped-table.json')
            entries=[]
            for i,e in enumerate(table['source_entries']):
                full=next((refs[v] for v in e['evidence_ids'] if refs.get(v) is not None),None)
                # Source numbering is literal. Compiler canonical labels remain
                # in the retained table, not in the source-faithful output.
                label,title=visible_prefix(e['title'])
                entries.append(dict(entry_id=f'g{gi}-'+e['id'],order=i+1,toc_pdf_page=e['source_page']+1,
                                    level=e['level'],parent_entry_id=f'g{gi}-'+e['parent'] if e['parent'] else None,numbering=label,
                                    title=title,printed_page_ref=full,
                                    evidence_ids=e['evidence_ids'],review_reasons=e['review_reasons']))
            predictions.append(dict(toc_group_id=f'toc-{gi}',physical_pages=[i+1 for i in indices],entries=entries))
        except Exception as e:
            errors.append(dict(group=gi,error=str(e)))
    doc.close()
    if sha(source)!=digest:raise RuntimeError('source changed')
    result=dict(schema='mpdf-explicit-toc-recovery/1',status='completed_with_errors' if errors else 'completed',
                source_sha256=digest,toc_groups=predictions,errors=errors,
                total_seconds=time.monotonic()-started,discovery_page_count=len(decisions),
                discovery_mode='all_pages_native_or_vision_fast_150dpi',selected_page_dpi=150,
                method='explicit_navigation_only_no_body_heading_inference',requires_review=True)
    save(output/'prediction.json',result)
    print(json.dumps({'status':result['status'],'groups':len(predictions),'errors':len(errors),'seconds':result['total_seconds']}),flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('source');p.add_argument('output');p.add_argument('--vision-worker',required=True)
    p.add_argument('--compiler',default='target/debug/mpdf-bookmarks');a=p.parse_args()
    recover(a.source,a.output,a.vision_worker,a.compiler)
