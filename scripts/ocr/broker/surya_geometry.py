#!/usr/bin/env python3
"""Frozen Surya runtime entrypoint: image -> lossless evidence + core geometry."""
import argparse,hashlib,importlib.metadata,json,os,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/ocr/geometry'))
from finalist_adapters import normalize,SURYA_VERSION

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
    p=argparse.ArgumentParser();p.add_argument('image',type=Path);p.add_argument('output',type=Path);p.add_argument('--page-index',type=int,default=0);p.add_argument('--model-dir',type=Path,required=True);a=p.parse_args()
    if a.output.exists():raise ValueError('Output already exists')
    freeze=json.loads((ROOT/'docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json').read_text())
    assert sha(ROOT/freeze['adapter_path'])==freeze['adapter_sha256'],'Frozen adapter changed'
    for name,h in freeze['model_sha256'].items():assert sha(a.model_dir/name)==h,'Frozen detector changed'
    for package in ['surya-ocr','transformers','torch']:
        assert importlib.metadata.version(package)==freeze['observed_detector_environment'][package],f'Wrong {package} version'
    import torch
    from safetensors.torch import load_file
    from PIL import Image
    from surya.detection import DetectionPredictor
    torch.set_num_threads(4)
    model=DetectionPredictor(checkpoint=str(a.model_dir),device='cpu',dtype=torch.float32)
    weights=load_file(str(a.model_dir/'model.safetensors'));loaded=model.model.state_dict()
    assert all(torch.equal(v.float(),loaded[k].float()) for k,v in weights.items()),'Detector weights not loaded correctly'
    image=Image.open(a.image).convert('RGB');image_hash=sha(a.image)
    raw={'detection':model([image],batch_size=1)[0].model_dump(mode='json'),'layout':None}
    evidence=normalize('surya',raw,image,f'page-{a.page_index}-{image_hash[:16]}',{'freeze_sha256':sha(ROOT/'docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json'),'image_sha256':image_hash,'adapter_sha256':freeze['adapter_sha256']})
    geometry={'page_index':a.page_index,'width':image.width,'height':image.height,'image_sha256':image_hash,
              'provider_id':'surya-line-geometry','provider_version':'surya-0.17.0/text_detection-2025_05_07/r3.0-apparatus-repair',
              'validation_status':'historical_material_not_validated','lines':[]}
    for line in evidence['logical_lines']:
        x,y,z,w=line['bbox'];geometry['lines'].append({'line_id':line['line_id'],'reading_order':line['reading_order'],'bbox':{'x':x,'y':y,'width':z-x,'height':w-y}})
    with a.output.open('x') as f:json.dump({'geometry':geometry,'evidence':evidence},f,ensure_ascii=False)
    print('Surya geometry ready; source and split evidence retained')
if __name__=='__main__':main()
