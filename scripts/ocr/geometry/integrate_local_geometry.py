"""Integrate verified local detector evidence into development units, all pages."""
import argparse,copy,json
from pathlib import Path
from PIL import Image
import shared_geometry_adapter as s
import finalist_adapters as a
from run_dev_geometry_bakeoff import write,sha,verify
from run_detector_adapter_challenge import ROOT,BASE,diagnostics,diag_summary,order_delta
from score_finalists import reference_partition,page_metrics,aggregate

def integrate(doc,image,receipts):
 d=copy.deepcopy(doc);_,t,_,mask=s.masks(image);events=[];extras=[]
 for record in receipts:
  receipt=record['receipt']
  for f in receipt.get('mapped',[]):
   if not f['accepted']:continue
   fid='local-'+record['sha256'][:12]+'-'+str(f['source_local_index'])
   b=f['bbox'];hints=[{'kind':'local_detector_recovered','receipt_sha256':record['sha256']}]
   # A recovered marginal number must inherit an already evidenced margin lane,
   # including its ordering policy. Never infer this from evaluation labels.
   for lane in d['margin_lanes']:
    z=lane['bbox']
    if z[0]<=a.xc(b)<=z[2] and z[1]<=a.yc(b)<=z[3] and a.width(b)<2*a.height(b):
     hints.append({'kind':'recurring_margin_lane','lane_id':lane['lane_id'],'support':'existing full-page lane'})
   extras.append({'fragment_id':fid,'source_box_index':f['source_local_index'],'bbox':b,'polygon':f['polygon'],'confidence':f['confidence'],'source_pointer':record['path']+'#/mapped/'+str(receipt['mapped'].index(f)),'region_ids':[],'hints':hints,'features':{'width':a.width(b),'height':a.height(b),'aspect':a.width(b)/a.height(b)}})
   events.append({'operation':'local_detector_recovery','output_fragment_ids':[fid],'receipt_path':record['path'],'receipt_sha256':record['sha256'],'scale':receipt['scale'],'roi':receipt['roi']['bbox'],'source_local_index':f['source_local_index']})
 d['fragments']+=extras
 bands=a.adaptive_bands(d['fragments']);a.assign_columns(d['fragments'],bands)
 d['column_bands']=bands;d['logical_lines']=s.organize(d['fragments'],mask,bands,d['page_id']);d['derivations']+=events
 # Supplemental evidence belongs to the evidence provenance, never the protected product schema.
 d['provenance']['local_redetection']={'source_fragments':extras,'receipts':[{k:r[k] for k in ['path','sha256']} for r in receipts],'parent_geometry_sha256':doc['geometry_sha256']}
 d['adapter_version']+='-local-evidence'
 d['geometry_sha256']=a.digest({k:v for k,v in d.items() if k!='geometry_sha256'})
 side={'version':s.VERSION,'crops':s.crops(d['logical_lines'],mask,bands)}
 assert d['provider_raw']==doc['provider_raw'] and d['source_fragments']==doc['source_fragments']
 seen=[fid for l in d['logical_lines'] for fid in l['fragment_ids']]
 assert sorted(seen)==sorted(f['fragment_id'] for f in d['fragments']) and len(seen)==len(set(seen))
 for l in d['logical_lines']:assert l['bbox']==a.envelope([f['bbox'] for f in d['fragments'] if f['fragment_id'] in l['fragment_ids']])
 return d,side

def main(source,out):
 out.mkdir(exist_ok=True);assert not (out/'results.json').exists()
 m=json.loads((ROOT/'reference-manifest.json').read_text());verify(m)
 snap={str(p):sha(p) for p in [Path(__file__),Path(s.__file__),Path(a.__file__),Path('scripts/ocr/geometry/local_geometry_redetect.py')]};write(out/'implementation-before-scoring.json',snap)
 results={}
 for provider in ['surya','paddle']:
  local=ROOT/f'local-{provider}';complete=json.loads((local/'complete.json').read_text());receipts=[]
  for r in complete['receipts']:
   assert sha(r['path'])==r['sha256'];receipts.append({**r,'receipt':json.loads(Path(r['path']).read_text())})
  folder=out/provider;folder.mkdir(exist_ok=True);passes=[];allrows=[];deltas=[];unit_deltas=[];hashes={};materialized=[]
  for attempt in [1,2]:
   pages=[]
   for p in m['pages']:
    pid=p['page_id'];doc=json.loads((source/f'{provider}-shared'/f'{pid}-pass{attempt}.json').read_text());image=Image.open(p['image_path']).convert('RGB')
    relevant=[r for r in receipts if r['receipt']['page_id']==pid];d,side=integrate(doc,image,relevant)
    write(folder/f'{pid}-pass{attempt}.json',d);write(folder/f'{pid}-pass{attempt}-crops.json',side)
    ref=reference_partition(json.loads(Path(p['reference_path']).read_text()));pages.append(page_metrics(ref,d))
    if attempt==1:
     hashes[pid]=a.digest({'lines':d['logical_lines'],'side':side});rows=diagnostics(ref,d,side,image);allrows+=rows;write(folder/f'{pid}-diagnostics.json',rows)
     base=json.loads((BASE/f'{pid}-pass1.json').read_text());deltas.append({'page_id':pid,**order_delta(ref,base,d)});unit_deltas.append({'page_id':pid,**order_delta(ref,doc,d)})
     # Materialize every crop used for crop scoring: proves concrete recognition input, not recognition accuracy.
     cdir=folder/'transcription-crops'/pid;cdir.mkdir(parents=True,exist_ok=True)
     for c in side['crops']:
      box=s.bounds(c['transcription_crop_bbox'],(image.height,image.width));cp=cdir/(c['line_id']+'.png');image.crop(tuple(box)).save(cp)
      materialized.append({'page_id':pid,'line_id':c['line_id'],'path':str(cp),'sha256':sha(cp),'bbox':box,'recognizer_called':False})
    else:assert hashes[pid]==a.digest({'lines':d['logical_lines'],'side':side})
    print(provider,attempt,pid,pages[-1]['matched'],flush=True)
   passes.append({'aggregate':aggregate(pages),'pages':pages})
  results[provider]={'passes':passes,'diagnostics':diag_summary(allrows),'order_vs_previous':deltas,'order_vs_shared_only':unit_deltas,'local_execution':complete,'accepted_local_boxes':sum(f['accepted'] for r in receipts for f in r['receipt'].get('mapped',[])),'deterministic_replay_pages':25,'crop_manifest':str(folder/'crop-manifest.json')};write(folder/'crop-manifest.json',materialized)
 verify(m);assert all(sha(p)==h for p,h in snap.items());write(out/'results.json',results)
if __name__=='__main__':
 ap=argparse.ArgumentParser();ap.add_argument('--source',type=Path,default=ROOT/'final');ap.add_argument('--out',type=Path,default=ROOT/'local-integrated');args=ap.parse_args();main(args.source,args.out)
