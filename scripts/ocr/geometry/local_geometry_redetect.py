"""Budgeted image-only local detection probe. No references or text labels imported."""
import argparse, json, math, os, statistics, time, traceback
from pathlib import Path
import cv2
import numpy as np
from PIL import Image
import shared_geometry_adapter as s
import finalist_adapters as a
from run_dev_geometry_bakeoff import SURYA,PADDLE,sha,write


def propose(image,doc):
 _,_,_,mask=s.masks(image)
 boxes=[c['transcription_crop_bbox'] for c in s.crops(doc['logical_lines'],mask,doc['column_bands'])]
 hs=[a.height(b) for b in boxes if a.width(b)>4*a.height(b)];h=statistics.median(hs) if hs else 20
 regions=[]
 for c in a.components(mask,[0,0,image.width,image.height]):
  b=c[:4];w,ch=a.width(b),a.height(b)
  if not (.22*h<ch<1.5*h and .08*h<w<2.5*h and c[4]>.025*h*h):continue
  if any(a.overlap(b,z)>.1*a.area(b) for z in boxes):continue
  dist=min((max(0,z[0]-b[2],b[0]-z[2])+2*max(0,z[1]-b[3],b[1]-z[3]))/h for z in boxes) if boxes else 1e9
  if dist>4:continue
  region=s.bounds([b[0]-3*h,b[1]-2*h,b[2]+3*h,b[3]+2*h],mask.shape)
  if a.area(region)>.1*image.width*image.height:continue
  regions.append({'component_bbox':b,'component_area':c[4],'bbox':region,'distance_in_line_heights':dist,'priority':c[4]/(1+dist),'line_height':h})
 selected=[]
 for r in sorted(regions,key=lambda r:(-r['priority'],r['bbox'])):
  if any(a.overlap(r['bbox'],q['bbox'])>0 for q in selected):continue
  if sum(a.area(q['bbox']) for q in selected)+a.area(r['bbox'])>.1*image.width*image.height:continue
  selected.append(r)
  if len(selected)==2:break
 return {'candidates':regions,'selected':selected,'policy':'image-only union-mask components outside bounded crops; <=2 regions/page, total <=10% page area, 2x crop; uncertain until local detector confirms'}


def setup(provider):
 if provider=='surya':
  import torch
  from surya.detection import DetectionPredictor
  from safetensors.torch import load_file
  torch.set_num_threads(4);model=DetectionPredictor(checkpoint=str(SURYA),device='cpu',dtype=torch.float32)
  weights=load_file(str(SURYA/'model.safetensors'));loaded=model.model.state_dict();assert all(torch.equal(v.float(),loaded[k].float()) for k,v in weights.items())
  return lambda im:{'detection':model([im],batch_size=1)[0].model_dump(mode='json'),'layout':None}
 from paddleocr import TextDetection
 model=TextDetection(model_name='PP-OCRv5_server_det',model_dir=str(PADDLE),device='cpu',enable_mkldnn=False,cpu_threads=4)
 return lambda im:{'detection':list(model.predict(np.asarray(im)[:,:,::-1].copy(),thresh=.3,box_thresh=.6,unclip_ratio=1.5))[0].json['res'],'layout':None}


def main(provider,source,out):
 assert not (out/'complete.json').exists()
 out.mkdir(parents=True,exist_ok=True)
 # Intake deliberately strips all reference fields before handing data to detector.
 manifest=json.loads(Path('docs/evidence/detector-adapter-challenge-2026-09-08/reference-manifest.json').read_text())
 inputs=[{k:p[k] for k in ['page_id','image_path','image_sha256']} for p in manifest['pages']]
 plans=[]
 for p in inputs:
  assert sha(p['image_path'])==p['image_sha256']
  doc=json.loads((source/f'{p["page_id"]}-pass1.json').read_text());image=Image.open(p['image_path']).convert('RGB')
  plans.append({**p,**propose(image,doc)})
 write(out/'proposals-before-inference.json',plans)
 predict=setup(provider);receipts=[]
 for p in plans:
  pid=p['page_id'];doc=json.loads((source/f'{pid}-pass1.json').read_text());image=Image.open(p['image_path']).convert('RGB');existing=[f['bbox'] for f in doc['source_fragments']];accepted=[]
  for i,r in enumerate(p['selected']):
   rp=out/f'{pid}-roi{i}.json'
   assert not rp.exists(),'Do not repeat completed inference'
   receipt={'page_id':pid,'provider':provider,'roi':r,'scale':2,'image_sha256':p['image_sha256']};start=time.perf_counter()
   crop=image.crop(tuple(r['bbox']));crop=crop.resize((crop.width*2,crop.height*2));cp=out/f'{pid}-roi{i}.png';crop.save(cp);receipt['crop_sha256']=sha(cp)
   try:
    raw=predict(crop);receipt['raw']=raw;receipt['mapped']=[]
    for j,f in enumerate(a.raw_boxes(provider,raw)):
     poly=[[x/2+r['bbox'][0],y/2+r['bbox'][1]] for x,y in f['polygon']];b=a.envelope([[x,y,x,y] for x,y in poly]);inter=a.overlap(b,r['component_bbox'])
     duplicate=any(a.overlap(b,z)>.5*min(a.area(b),a.area(z)) for z in existing+accepted)
     target=inter>=.5*a.area(r['component_bbox'])
     edge=b[0]<=r['bbox'][0]+1 or b[1]<=r['bbox'][1]+1 or b[2]>=r['bbox'][2]-1 or b[3]>=r['bbox'][3]-1
     ok=target and not duplicate and not edge
     receipt['mapped'].append({'source_local_index':j,'bbox':b,'polygon':poly,'confidence':f['confidence'],'accepted':ok,'duplicate':duplicate,'supports_trigger_component':target,'touches_crop_boundary':edge})
     if ok:accepted.append(b)
    receipt['status']='ok'
   except Exception:receipt.update(status='error',error=traceback.format_exc())
   receipt['seconds']=time.perf_counter()-start;write(rp,receipt);receipts.append({'path':str(rp),'sha256':sha(rp),'status':receipt['status']});print(pid,i,receipt['status'],len(accepted),flush=True)
 write(out/'complete.json',{'provider':provider,'receipts':receipts,'inference_count':len(receipts),'failures':sum(r['status']!='ok' for r in receipts),'new_full_page_inferences':0,'retries':0,'implementation_sha256':sha(__file__)})

if __name__=='__main__':
 os.environ.update(OMP_NUM_THREADS='4',OPENBLAS_NUM_THREADS='4',VECLIB_MAXIMUM_THREADS='4',TOKENIZERS_PARALLELISM='false',MPLCONFIGDIR='/private/tmp/mpdf-challenge-mpl',PADDLE_PDX_CACHE_HOME='/private/tmp/mpdf-geometry-dev-2026-09-06/paddlex',PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK='True')
 ap=argparse.ArgumentParser();ap.add_argument('provider',choices=['surya','paddle']);ap.add_argument('--source',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);args=ap.parse_args();main(args.provider,args.source,args.out)
