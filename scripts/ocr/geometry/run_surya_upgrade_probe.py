"""One 25-page detector pass per release; fixed committed adapter and references."""
import os
os.environ.update(OMP_NUM_THREADS='4',OPENBLAS_NUM_THREADS='4',VECLIB_MAXIMUM_THREADS='4',TOKENIZERS_PARALLELISM='false')
import json,sys,time,importlib.metadata as metadata
from pathlib import Path
from PIL import Image
import torch
from safetensors.torch import load_file
from surya.detection import DetectionPredictor
from surya.settings import settings
from finalist_adapters import normalize,digest
from run_dev_geometry_bakeoff import MANIFEST,verify,sha,SURYA,model_hashes

OUT=Path('docs/evidence/surya-upgrade-probe-2026-09-06')
def write(p,d):p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n')
def main(version):
    assert metadata.version('surya-ocr')==version
    folder=OUT/version;folder.mkdir(parents=True,exist_ok=True)
    assert not (folder/'run.json').exists()
    manifest=json.loads(MANIFEST.read_text());verify(manifest)
    adapter=Path(__file__).with_name('finalist_adapters.py');adapter_hash=sha(adapter)
    info={'version':version,'adapter_sha256':adapter_hash,'runner_sha256':sha(__file__),
          'versions':{p:metadata.version(p) for p in ['surya-ocr','transformers','huggingface-hub','torch','tokenizers','numpy','pillow','opencv-python']},
          'default_detector':settings.DETECTOR_MODEL_CHECKPOINT,'model_sha256':model_hashes('surya'),
          'device':'cpu','dtype':'float32','threads':4,'batch_size':1,'runs':[]}
    assert settings.DETECTOR_MODEL_CHECKPOINT=='s3://text_detection/2025_05_07'
    torch.set_num_threads(4)
    t=time.perf_counter()
    # New release's official local API avoids persistent shared server execution.
    model=DetectionPredictor.local(device='cpu',dtype=torch.float32) if version=='0.22.1' else DetectionPredictor(checkpoint=str(SURYA),device='cpu',dtype=torch.float32)
    weights=load_file(str(SURYA/'model.safetensors'));loaded=model.model.state_dict()
    bad=[k for k,v in weights.items() if k not in loaded or not torch.equal(v.float(),loaded[k].float())]
    assert not bad,('checkpoint load mismatch',bad[:10])
    info['loaded_tensor_equality']=len(weights);info['initialization_seconds']=time.perf_counter()-t
    write(folder/'run.json',info)
    for p in manifest['pages']:
        t=time.perf_counter()
        with Image.open(p['image_path']) as image:
            image=image.convert('RGB');raw={'detection':model([image],batch_size=1)[0].model_dump(mode='json'),'layout':None}
            secs=time.perf_counter()-t
            d=normalize('surya',raw,image,p['page_id'],{'version':version,'adapter_sha256':adapter_hash,'image_sha256':p['image_sha256'],'model_sha256':info['model_sha256']})
        write(folder/(p['page_id']+'-raw.json'),raw);write(folder/(p['page_id']+'.json'),d)
        info['runs'].append({'page_id':p['page_id'],'inference_seconds':secs,'raw_digest':digest(raw),'boxes':len(d['source_fragments']),'logical_lines':len(d['logical_lines'])})
        write(folder/'run.json',info);print(version,p['page_id'],len(d['source_fragments']),len(d['logical_lines']),round(secs,2),flush=True)
    verify(manifest);assert sha(adapter)==adapter_hash
    info['complete']=True;write(folder/'run.json',info)
if __name__=='__main__':main(sys.argv[1])
