#!/usr/bin/env python3
"""Two full local inference + normalization passes, no reference data in adapters."""
import argparse,importlib.metadata,json,os,platform,time,traceback
from pathlib import Path
from PIL import Image
from run_dev_geometry_bakeoff import ROOT,MANIFEST,SURYA,PADDLE,LAYOUT,verify,sha,write,model_hashes
from finalist_adapters import normalize,digest,VERSION

OUT=ROOT/'docs/evidence/geometry-finalists-r2-2026-09-06'

def setup(name):
    if name=='surya':
        import torch
        from surya.detection import DetectionPredictor
        from safetensors.torch import load_file
        torch.set_num_threads(4)
        model=DetectionPredictor(checkpoint=str(SURYA),device='cpu',dtype=torch.float32)
        weights=load_file(str(SURYA/'model.safetensors'));loaded=model.model.state_dict()
        assert all(torch.equal(v.float(),loaded[k].float()) for k,v in weights.items()),'checkpoint load mismatch'
        def predict(p):
            with Image.open(p) as im:r=model([im.convert('RGB')],batch_size=1)[0]
            return {'detection':r.model_dump(mode='json'),'layout':None}
        return predict
    from paddleocr import TextDetection,LayoutDetection
    dt=TextDetection(model_dir=str(PADDLE),device='cpu',enable_mkldnn=False,cpu_threads=4)
    lr=LayoutDetection(model_dir=str(LAYOUT),device='cpu',enable_mkldnn=False,cpu_threads=4)
    return lambda p:{'detection':list(dt.predict(p))[0].json['res'],'layout':list(lr.predict(p))[0].json['res']}

def main(name):
    os.environ.update(OMP_NUM_THREADS='4',OPENBLAS_NUM_THREADS='4',VECLIB_MAXIMUM_THREADS='4',TOKENIZERS_PARALLELISM='false',
      MPLCONFIGDIR='/private/tmp/mpdf-geometry-dev-2026-09-06/mpl',PADDLE_PDX_CACHE_HOME='/private/tmp/mpdf-geometry-dev-2026-09-06/paddlex')
    m=json.loads(MANIFEST.read_text());verify(m)
    assert not (OUT/f'{name}-run.json').exists()
    folder=OUT/'pages'/name;folder.mkdir(parents=True,exist_ok=True)
    prov={'model_sha256':model_hashes(name),'adapter_sha256':sha(Path(__file__).with_name('finalist_adapters.py')),
          'adapter_version':VERSION,'protocol_sha256':sha(OUT/'protocol.md'),'runner_sha256':sha(__file__),
          'versions':{p:importlib.metadata.version(p) for p in ['surya-ocr','paddleocr','paddlepaddle','torch','transformers']}}
    info={'provider':name,'platform':platform.platform(),'provenance':prov,'manifest_sha256':sha(MANIFEST),'runs':[]}
    t=time.perf_counter();predict=setup(name);info['initialization_seconds']=time.perf_counter()-t
    for attempt in [1,2]:
        for p in m['pages']:
            t=time.perf_counter();entry={'page_id':p['page_id'],'attempt':attempt}
            try:
                raw=predict(p['image_path']);tinfer=time.perf_counter()-t
                # Raw payload is persisted even when normalization fails.
                rawpath=folder/f'{p["page_id"]}-pass{attempt}-raw.json';write(rawpath,raw)
                with Image.open(p['image_path']) as image:
                    doc=normalize(name,raw,image.convert('RGB'),p['page_id'],{**prov,'image_sha256':p['image_sha256']})
                path=folder/f'{p["page_id"]}-pass{attempt}.json';write(path,doc)
                entry.update(status='ok',file=str(path.relative_to(OUT)),sha256=sha(path),raw_digest=digest(raw),
                             geometry_digest=doc['geometry_sha256'],inference_seconds=tinfer,
                             line_count=len(doc['logical_lines']),fragment_count=len(doc['fragments']),columns=len(doc['column_bands']),margin_splits=len(doc['derivations']))
            except Exception:entry.update(status='error',error=traceback.format_exc())
            entry['seconds']=time.perf_counter()-t;info['runs'].append(entry);write(OUT/f'{name}-run.json',info)
            print(name,attempt,p['page_id'],entry['status'],entry.get('line_count'),entry.get('columns'),entry.get('margin_splits'),round(entry['seconds'],2),flush=True)
    verify(m);info['inputs_unchanged']=True;write(OUT/f'{name}-run.json',info)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('provider',choices=['paddle','surya']);main(parser.parse_args().provider)
