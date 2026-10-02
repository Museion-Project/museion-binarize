"""Fixed development-only four-cell comparison. References enter scoring only."""
import argparse, itertools, json, math, statistics, time
from pathlib import Path
import cv2
import numpy as np
from PIL import Image
import shared_geometry_adapter as adapter
from run_dev_geometry_bakeoff import verify, sha, write
from score_finalists import reference_partition, page_metrics, aggregate, match_boxes
import finalist_adapters as a

ROOT=Path('docs/evidence/detector-adapter-challenge-2026-09-08')
RAW=Path('docs/evidence/geometry-finalists-r2-2026-09-06')
BASE=Path('docs/evidence/surya-geometry-root-cause-2026-09-08/repair1-verified/normalized')

def order_delta(ref, before, after):
 def mapping(doc):return {ref[m['gold']]['line_id']:m['candidate'] for m in match_boxes(ref,doc['logical_lines'])}
 bm,am=mapping(before),mapping(after);ids=[g['line_id'] for g in ref];common=set(bm)&set(am)
 result={'common_pairs':0,'before_inversions':0,'after_inversions':0,'regressions':[],'fixes':[],'new_pairs':0,'new_inversions':[],'lost_lines':sorted(set(bm)-set(am)),'recovered_lines':sorted(set(am)-set(bm))}
 for i,j in itertools.combinations(ids,2):
  if i in common and j in common:
   b,c=bm[i]>bm[j],am[i]>am[j];result['common_pairs']+=1;result['before_inversions']+=b;result['after_inversions']+=c
   if c and not b:result['regressions'].append([i,j])
   if b and not c:result['fixes'].append([i,j])
  elif i in am and j in am:
   result['new_pairs']+=1
   if am[i]>am[j]:result['new_inversions'].append([i,j])
 return result

def ink_count(mask,box):
 x0,y0,x1,y1=adapter.bounds(box,mask.shape)
 return int(mask[y0:y1,x0:x1].sum()) if x1>x0 and y1>y0 else 0

def diagnostics(ref,doc,side,image):
 gray=np.asarray(image.convert('L'));_,otsu=cv2.threshold(gray,0,1,cv2.THRESH_BINARY_INV+cv2.THRESH_OTSU)
 local=cv2.adaptiveThreshold(gray,1,cv2.ADAPTIVE_THRESH_GAUSSIAN_C,cv2.THRESH_BINARY_INV,51,9)
 maps={m['gold']:m['candidate'] for m in match_boxes(ref,doc['logical_lines'])};crops={c['line_id']:c['transcription_crop_bbox'] for c in side['crops']};rows=[]
 for i,g in enumerate(ref):
  l=doc['logical_lines'][maps[i]] if i in maps else None
  r={'reference_id':g['line_id'],'matched':l is not None,'logical_line_id':l['line_id'] if l else None,'fragment_ids':l['fragment_ids'] if l else [],'ink':{}}
  for name,mask in [('otsu',otsu),('adaptive',local)]:
   total=ink_count(mask,g['bbox']);v={}
   for layer,b in [('unit',l['bbox'] if l else None),('crop',crops[l['line_id']] if l else None)]:
    inter=[max(g['bbox'][0],b[0]),max(g['bbox'][1],b[1]),min(g['bbox'][2],b[2]),min(g['bbox'][3],b[3])] if b else None
    v[layer]=ink_count(mask,inter)/total if inter and total else (0 if total else None)
   r['ink'][name]=v
  r['residual_units']=[q['line_id'] for q in doc['logical_lines'] if a.overlap(q['bbox'],g['bbox'])>=.5*a.area(q['bbox']) and a.area(q['bbox'])>=.015*a.area(g['bbox'])]
  r['other_reference_overlap']=[q['line_id'] for j,q in enumerate(ref) if j!=i and l and a.overlap(q['bbox'],l['bbox'])>.5*a.area(q['bbox'])]
  r['crop_other_reference_overlap']=[q['line_id'] for j,q in enumerate(ref) if j!=i and l and a.overlap(q['bbox'],crops[l['line_id']])>.5*a.area(q['bbox'])]
  r['large_unit_ratio']=a.area(l['bbox'])/a.area(g['bbox']) if l else None
  rows.append(r)
 return rows

def diag_summary(rows):
 return {**{f'{n}_{layer}_mean':statistics.mean(r['ink'][n][layer] for r in rows if r['ink'][n][layer] is not None) for n in ['otsu','adaptive'] for layer in ['unit','crop']},**{f'{n}_{layer}_below98':sum(r['ink'][n][layer] is not None and r['ink'][n][layer]<.98 for r in rows) for n in ['otsu','adaptive'] for layer in ['unit','crop']},'residual_split_proxy':sum(len(r['residual_units'])>1 for r in rows),'contaminated_match_proxy':sum(bool(r['other_reference_overlap']) for r in rows),'contaminated_crop_proxy':sum(bool(r['crop_other_reference_overlap']) for r in rows),'unit_area_over_2x_reference':sum((r['large_unit_ratio'] or 0)>2 for r in rows)}

def main(out,passes):
 assert not (out/'results.json').exists(),'Do not overwrite completed evidence'
 out.mkdir(parents=True,exist_ok=True)
 manifest=json.loads((ROOT/'reference-manifest.json').read_text());verify(manifest)
 snapshot={str(p):sha(p) for p in [Path(__file__),Path(adapter.__file__),Path(a.__file__),ROOT/'protocol.md',ROOT/'reference-manifest.json',Path('scripts/ocr/geometry/score_finalists.py'),Path('scripts/ocr/geometry/run_geometry_bakeoff.py')]}
 write(out/'implementation-before-scoring.json',snapshot)
 result={'scope':'25 full development pages; replay retained raw; no inference or production proof','candidates':{},'baseline':[]}
 basepages=[]
 for provider in ['surya','paddle']:
  for mode in ['minimal','shared']:
   key=provider+'-'+mode;folder=out/key;folder.mkdir(exist_ok=True);runs=[];allrows=[];deltas=[];hashes={};rawrecords=[]
   for attempt in range(1,passes+1):
    pages=[]
    for p in manifest['pages']:
     pid=p['page_id'];rawpath=RAW/'pages'/provider/f'{pid}-pass{attempt}-raw.json';raw=json.loads(rawpath.read_text());image=Image.open(p['image_path']).convert('RGB')
     doc,side=adapter.normalize(provider,raw,image,pid,{'image_sha256':p['image_sha256'],'implementation':snapshot},mode)
     ref=reference_partition(json.loads(Path(p['reference_path']).read_text()))
     pages.append(page_metrics(ref,doc));write(folder/f'{pid}-pass{attempt}.json',doc);write(folder/f'{pid}-pass{attempt}-crops.json',side)
     rawrecords.append({'path':str(rawpath),'sha256':sha(rawpath),'image_sha256':p['image_sha256']})
     if attempt==1:
      rows=diagnostics(ref,doc,side,image);allrows.extend(rows);write(folder/f'{pid}-diagnostics.json',rows)
      baseline=json.loads((BASE/f'{pid}-pass1.json').read_text())
      deltas.append({'page_id':pid,**order_delta(ref,baseline,doc)})
      if key=='surya-minimal':basepages.append(page_metrics(ref,baseline))
      hashes[pid]=a.digest({'lines':doc['logical_lines'],'fragments':doc['fragments'],'side':side})
     else:assert hashes[pid]==a.digest({'lines':doc['logical_lines'],'fragments':doc['fragments'],'side':side}),pid
     print(key,attempt,pid,pages[-1]['matched'],flush=True)
    runs.append({'aggregate':aggregate(pages),'pages':pages})
   result['candidates'][key]={'passes':runs,'diagnostics':diag_summary(allrows),'order_vs_previous':deltas,'raw_inputs':rawrecords,'deterministic_pages':25 if passes==2 else None}
 result['baseline']=aggregate(basepages)
 verify(manifest);assert all(sha(p)==h for p,h in snapshot.items())
 write(out/'results.json',result)
 print(json.dumps({k:{'recall':v['passes'][0]['aggregate']['recall'],'coverage':v['passes'][0]['aggregate']['all_gold_coverage'],**v['diagnostics']} for k,v in result['candidates'].items()},indent=2))

if __name__=='__main__':
 ap=argparse.ArgumentParser();ap.add_argument('--out',type=Path,required=True);ap.add_argument('--passes',type=int,choices=[1,2],default=2);args=ap.parse_args();main(args.out,args.passes)
