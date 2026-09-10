#!/usr/bin/env python3
"""Explicit diagnostic-gated Surya detection ONLY, using the unchanged adapter.
No imports of recognition, foundation or layout model code. Offline cached weights.
Geometry is supplemental evidence; it cannot repair missing characters/numbers.
"""
import os,sys,time,copy
from pathlib import Path
from bookmarks import load,save,sha,project

def run(raw_path,diagnostic_path,output):
    process_started=time.perf_counter()
    raw=load(raw_path);diagnostic=load(diagnostic_path)
    allowed={'double_column_reading_order','multiline_grouping_unresolved','title_number_lane_unresolved','indentation_unresolved','degenerate_vision_bbox'}
    if diagnostic.get('source_sha256')!=raw['source_sha256'] or diagnostic.get('page_index')!=raw['page_index'] or not set(diagnostic.get('reasons',[]))&allowed:raise ValueError('explicit geometry diagnostic required')
    out=Path(output);out.mkdir();os.environ.update(HF_HUB_OFFLINE='1',TRANSFORMERS_OFFLINE='1')
    sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'ocr/geometry'))
    import finalist_adapters as adapter
    import torch
    from PIL import Image
    from surya.detection import DetectionPredictor
    from safetensors.torch import load_file
    checkpoint=Path('/Users/theo/Library/Caches/datalab/models/text_detection/2025_05_07')
    if not (checkpoint/'model.safetensors').is_file():raise ValueError('cached detector absent; downloading forbidden')
    torch.set_num_threads(4);start=time.perf_counter();model=DetectionPredictor(checkpoint=str(checkpoint),device='cpu',dtype=torch.float32)
    weights=load_file(str(checkpoint/'model.safetensors'));state=model.model.state_dict()
    if not all(torch.equal(v.float(),state[k].float()) for k,v in weights.items()):raise ValueError('cached checkpoint mismatch')
    load_seconds=time.perf_counter()-start;im=Image.open(raw['image_path']).convert('RGB');start=time.perf_counter()
    detection={'detection':model([im],batch_size=1)[0].model_dump(mode='json'),'layout':None};detect_seconds=time.perf_counter()-start
    save(out/'raw.json',detection);start=time.perf_counter()
    geometry=adapter.normalize('surya',detection,im,raw['id'],{'source_sha256':raw['source_sha256'],'image_sha256':sha(raw['image_path']),'diagnostic_path':str(Path(diagnostic_path).resolve())})
    save(out/'geometry.json',geometry);adapter_seconds=time.perf_counter()-start
    # Supplemental measured supports mapped to the same visible-page coordinate
    # convention. Preserve Vision strings/token boxes, attach overlap links only.
    p=project(raw,raw_path);sx=raw['width']/im.width;sy=raw['height']/im.height
    supports=[]
    for line in geometry['logical_lines']:
        b=line['bbox'];supports.append({'id':line['line_id'],'bbox':{'x':b[0]*sx,'y':b[1]*sy,'width':(b[2]-b[0])*sx,'height':(b[3]-b[1])*sy},'source_kind':'surya_geometry','state':'inspected','reading_order':line['reading_order']})
    p['geometry_supports']=supports;p['geometry_path']=str((out/'geometry.json').resolve())
    for l in p['lines']:
        b=l['bbox'];links=[]
        for s in supports:
            q=s['bbox'];area=max(0,min(b['x']+b['width'],q['x']+q['width'])-max(b['x'],q['x']))*max(0,min(b['y']+b['height'],q['y']+q['height'])-max(b['y'],q['y']))
            if area>min(b['width']*b['height'],q['width']*q['height'])*.5:links.append(s['id'])
        l['geometry_support_ids']=links
        if len(links)!=1:l['state']='ambiguous'
    p['diagnostics']=diagnostic['reasons'];save(out/'evidence.json',[p])
    receipt=dict(load_seconds=load_seconds,detect_seconds=detect_seconds,adapter_seconds=adapter_seconds,recognition_calls=0,pages=1,geometry_lines=len(supports),ambiguous_groups=sum(l['state']=='ambiguous' for l in p['lines']),status='supplemental_geometry_needs_review',limitation='Support linking does not resegment mixed Vision observations or recover missing text. Ambiguous groups remain review.')
    receipt['process_seconds']=time.perf_counter()-process_started
    save(out/'receipt.json',receipt);print(receipt)
if __name__=='__main__':run(*sys.argv[1:])
