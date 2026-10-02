#!/usr/bin/env python3
"""Normal numeric-lane second read, with the unchanged Apple Vision fast worker.
One crop per title-bearing physical row; no fallback trigger or low-confidence cap.
Numbers are never inferred from the hierarchy grammar or assessment data.
"""
import argparse,subprocess,time
from pathlib import Path
import fitz
from PIL import Image
from bookmarks import load,save,project,number,sha

def prepare(raw_paths,directory,max_crops=160):
    out=Path(directory);out.mkdir();requests=[]
    for raw_path in raw_paths:
        raw=load(raw_path)
        if raw['bbox_convention']!='normalized_bottom_left_xywh':raise ValueError('numeric reread expects full-page Vision raw')
        if sha(raw['source'])!=raw['source_sha256']:raise ValueError('source changed')
        p=project(raw,raw_path);lanes=p.get('number_lanes',[]);rows=p.get('visual_rows',[]);selected=[]
        for row in rows:
            b=row['bbox'];lane=row['lane']
            if lane>=len(lanes):continue
            # No title to the left of this lane => this is header furniture.
            if b[0]+b[2]>lanes[lane]+p['width']*.01:continue
            if b[1]<p['height']*.13 and row['text'].lower().startswith('page '):continue
            if row['text'].strip().lower() in {'contents','table of contents','inhalt','inhaltsverzeichnis','sommaire','table des matières','目录','目次'}:continue
            selected.append(row)
        start=time.perf_counter();doc=fitz.open(raw['source']);pix=doc[raw['page_index']].get_pixmap(matrix=fitz.Matrix(300/72,300/72));render=time.perf_counter()-start
        im=Image.frombytes('RGB',(pix.width,pix.height),pix.samples)
        for row in selected:
            b=row['bbox'];edge=lanes[row['lane']]
            # Uniform fixed lane window, generous white context for short tokens;
            # retain the complete source ROI transform in every observation.
            roi=[max(0,edge-max(36,p['width']*.05)),max(0,b[1]-b[3]*.5),min(p['width'],edge+10),min(p['height'],b[1]+b[3]*1.5)]
            ident=f'{raw["id"]}-{row["id"]}';image=out/f'{ident}.png';im.crop(tuple(round(v*300/72) for v in roi)).save(image)
            requests.append(dict(id=ident,image_path=str(image.resolve()),source=raw['source'],source_sha256=raw['source_sha256'],page_index=raw['page_index'],page_count=raw['page_count'],width=roi[2]-roi[0],height=roi[3]-roi[1],source_roi=roi,row_id=row['id'],raw_path=str(Path(raw_path).resolve()),render_seconds=render/len(selected),stage='mandatory_numeric_lane'))
    if len(requests)>max_crops:raise ValueError('numeric lane crop limit exceeded')
    save(out/'requests.json',requests);return requests

def collect(result_dir):
    by_raw={};receipts=[]
    for path in sorted(Path(result_dir).glob('*.json')):
        if path.name.startswith('._'):continue  # macOS sidecar, not a recognition record
        r=load(path);tokens=[o for o in r.get('observations',[]) if number(o['text'].strip())]
        candidate=None
        if len(tokens)==1:
            t=tokens[0];b=t['bbox'];roi=r['source_roi']
            candidate=dict(text=t['text'].strip(),confidence=t['confidence'],bbox=[roi[0]+b[0]*r['width'],roi[1]+(1-b[1]-b[3])*r['height'],b[2]*r['width'],b[3]*r['height']])
        by_raw.setdefault(r['raw_path'],{})[r['row_id']]=dict(candidate=candidate,evidence_ref=str(path.resolve()),state=r['state'],ocr_seconds=r['ocr_seconds'])
        receipts.append(dict(id=r['id'],source_page=r['page_index'],candidate=candidate['text'] if candidate else None,ocr_seconds=r['ocr_seconds'],render_seconds=r['render_seconds']))
    return by_raw,receipts

def run(raw_paths,directory,worker,max_crops=160):
    directory=Path(directory);directory.mkdir();start=time.perf_counter();requests=prepare(raw_paths,directory/'input',max_crops=max_crops)
    subprocess.run([worker,str(directory/'input/requests.json'),str(directory/'vision')],check=True,timeout=120)
    by_raw,receipts=collect(directory/'vision');pages=[project(load(p),p,by_raw.get(str(Path(p).resolve()),{})) for p in raw_paths]
    save(directory/'evidence.json',pages);save(directory/'receipt.json',dict(stage='mandatory_numeric_lane',crops=len(requests),process_seconds=time.perf_counter()-start,rows=receipts));return pages
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('output');p.add_argument('raw',nargs='+');p.add_argument('--worker',required=True);a=p.parse_args();run(a.raw,a.output,a.worker)
