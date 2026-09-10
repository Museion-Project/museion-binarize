#!/usr/bin/env python3
"""Bounded same-Vision number retry: <=8 ROIs/page, one 300dpi page render.
Prepare requests, run the existing fast worker, then apply as a versioned projection.
"""
import copy,sys,time
from pathlib import Path
import fitz
from bookmarks import load,save,arr,bbox,number

def prepare(evidence,table,directory,limit=8):
    directory=Path(directory);directory.mkdir();pages=load(evidence);t=load(table);requests=[]
    for p in pages:
        ls={l['id']:l for l in p['lines']};entries=[e for e in t['entries'] if e['source_page']==p['page_index']]
        eligible=[e for e in entries if any(r in e['review_reasons'] for r in ['printed_missing','printed_low_confidence','printed_pattern_invalid','printed_nonmonotonic','printed_out_of_range','printed_large_neighbor_gap'])]
        eligible.sort(key=lambda e:(e['printed_value'] is not None,e['id']))
        # Ignore isolated centered chapter headings: no title-number lane claim.
        nums=[l['number_bbox'] for l in p['lines'] if l.get('number_bbox')]
        if not nums:continue
        edge=sorted(b['x']+b['width'] for b in nums)[len(nums)//2]
        selected=[]
        raw=load(p['raw_path']);rawobs={o['id']:o for o in raw['observations']}
        for e in eligible:
            l=ls[e['evidence_ids'][0]]
            if l['bbox']['x']>p['width']*.63 and l['bbox']['width']<p['width']*.25 and l['bbox']['height']>20:continue
            selected.append((e,l))
            if len(selected)>=min(limit,8):break
        if not selected:continue
        began=time.perf_counter();doc=fitz.open(p['source']);page=doc[p['page_index']];pix=page.get_pixmap(matrix=fitz.Matrix(300/72,300/72));render=time.perf_counter()-began
        from PIL import Image
        im=Image.frombytes('RGB',[pix.width,pix.height],pix.samples)
        for e,l in selected:
            if l.get('number_bbox'):b=arr(l['number_bbox'])
            else:
                # Last visual title line, not the union height of author+title.
                originals=[rawobs[i] for i in l['raw_ids'] if i in rawobs]
                last=min(originals,key=lambda o:o['bbox'][1]) # raw bottom-left
                rb=last['bbox'];b=[edge-p['width']*.055,(1-rb[1]-rb[3])*p['height'],p['width']*.065,rb[3]*p['height']]
            roi=[max(0,b[0]-6),max(0,b[1]-4),min(p['width'],b[0]+b[2]+6),min(p['height'],b[1]+b[3]+4)]
            ident=f'p{p["page_index"]}-{e["id"]}';path=directory/f'{ident}.png';im.crop(tuple(round(v*300/72) for v in roi)).save(path)
            requests.append(dict(id=ident,image_path=str(path.resolve()),source=p['source'],source_sha256=p['source_sha256'],page_index=p['page_index'],page_count=p['page_count'],width=roi[2]-roi[0],height=roi[3]-roi[1],source_roi=roi,line_id=l['id'],entry_id=e['id'],render_seconds=render/len(selected),retry_round=1))
    save(directory/'requests.json',requests);return len(requests)

def apply(evidence,results,output):
    pages=load(evidence);byid={l['id']:l for p in pages for l in p['lines']};receipts=[]
    for path in sorted(Path(results).glob('*.json')):
        r=load(path);l=byid[r['line_id']];tokens=[o for o in r['observations'] if number(o['text'].strip())]
        accepted=False;reason='no_unique_numeric_observation'
        if len(tokens)==1:
            o=tokens[0];candidate=o['text'].strip();old=l.get('number_raw')
            # Agreement, or filling a missing/invalid label; conflicts stay review.
            if old and number(old) and number(candidate)!=number(old):reason='retry_conflicts_with_fullpage';l['state']='ambiguous'
            else:
                if old and l['text'].endswith(' '+old):l['text']=l['text'][:-len(old)-1]
                l['text']+=' '+candidate;l['printed_candidate']=candidate;l['number_raw']=candidate;l['number_confidence']=o['confidence'];l['state']='locally_recognized';accepted=True;reason='retry_agreement_or_missing_label'
                roi=r['source_roi'];bb=o['bbox'];l['number_bbox']=bbox([roi[0]+bb[0]*r['width'],roi[1]+(1-bb[1]-bb[3])*r['height'],bb[2]*r['width'],bb[3]*r['height']])
        l.setdefault('retry_evidence',[]).append(str(path.resolve()));receipts.append(dict(line_id=l['id'],accepted=accepted,reason=reason,ocr_seconds=r['ocr_seconds'],render_seconds=r['render_seconds']))
    save(output,pages);save(str(output)+'.retry.json',receipts)
if __name__=='__main__':
    if sys.argv[1]=='prepare':print(prepare(*sys.argv[2:]))
    elif sys.argv[1]=='apply':apply(*sys.argv[2:])
