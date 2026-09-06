"""Finalize each independently inferred pass with the common lossless adapter."""
import json,time
from pathlib import Path
from PIL import Image
from run_dev_geometry_bakeoff import MANIFEST,verify,sha,write
from run_finalist_bakeoff import OUT
from finalist_adapters import normalize,VERSION

def main():
    m=json.loads(MANIFEST.read_text());verify(m)
    for name in ['paddle','surya']:
        info=json.loads((OUT/f'{name}-run.json').read_text());assert len(info['runs'])==50 and info['inputs_unchanged']
        assert all(x['status']=='ok' for x in info['runs'])
        prov={**info['provenance'],'adapter_version':VERSION,'adapter_sha256':sha(Path(__file__).with_name('finalist_adapters.py')),
              'normalization_runner_sha256':sha(__file__),'inference_manifest_sha256':sha(OUT/f'{name}-run.json')}
        final={'provider':name,'provenance':prov,'runs':[]};folder=OUT/'normalized'/name;folder.mkdir(parents=True,exist_ok=True)
        assert not (OUT/f'{name}-normalized.json').exists()
        for attempt in [1,2]:
            for p in m['pages']:
                rawpath=OUT/'pages'/name/f'{p["page_id"]}-pass{attempt}-raw.json';raw=json.loads(rawpath.read_text());t=time.perf_counter()
                with Image.open(p['image_path']) as image:d=normalize(name,raw,image.convert('RGB'),p['page_id'],{**prov,'image_sha256':p['image_sha256']})
                path=folder/f'{p["page_id"]}-pass{attempt}.json';write(path,d)
                final['runs'].append({'page_id':p['page_id'],'attempt':attempt,'file':str(path.relative_to(OUT)),
                    'sha256':sha(path),'raw_file_sha256':sha(rawpath),'geometry_digest':d['geometry_sha256'],'seconds':time.perf_counter()-t})
        write(OUT/f'{name}-normalized.json',final)
    verify(m);print('PASS: 100 independently inferred page outputs normalized with r2.2, raw evidence retained')

if __name__=='__main__':main()
