"""Operational frozen spatial preparation/audit. No credentials or HTTP here.

The caller supplies an original PDF raster and independent stage pins. This
wrapper invokes the already accepted algorithms; it never loads evaluation
references, saved detector predictions or development source-page registries.
"""
import argparse
import base64
import hashlib
import importlib.metadata
import io
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT/'scripts/ocr/geometry'))
from PIL import Image
import finalist_adapters as a
import shared_geometry_adapter as shared
import frozen_local_proposal_adapter as proposal_adapter
import spatial_supports
import local_geometry_redetect as local
from integrate_local_geometry import integrate
from local_component_recovery import recover
import spatial_verification as v
import spatial_batch_strategy as batches
import spatial_batch_audit as audit
import toc_leader_projection as leader
from server import MODEL

RUNTIME = 'frozen-surya-spatial-factory/1'


def save(path, value):
    with Path(path).open('xb') as f:
        f.write(v.canonical(value).encode())


def read(path):
    return v.parse(Path(path).read_bytes())


def digest(path):
    return v.sha(Path(path).read_bytes())


def infer(image_path, output, model_dir, page_index):
    v.require(sys.flags.optimize == 0, 'Frozen geometry assertions must be enabled')
    output = Path(output)
    # The Rust factory already exclusively created this page directory and raster.
    v.require(not (output/'geometry-index.json').exists(), 'Geometry already exists')
    freeze = read(ROOT/'docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json')
    for name, expected in freeze['model_sha256'].items():
        v.require(digest(Path(model_dir)/name) == expected, 'Frozen model mismatch')
    for package in ['surya-ocr','transformers','torch']:
        v.require(importlib.metadata.version(package) == freeze['observed_detector_environment'][package], 'Frozen package mismatch: '+package)
    import torch
    from safetensors.torch import load_file
    from surya.detection import DetectionPredictor
    torch.set_num_threads(4)
    model = DetectionPredictor(checkpoint=str(model_dir), device='cpu', dtype=torch.float32)
    weights = load_file(str(Path(model_dir)/'model.safetensors'))
    loaded = model.model.state_dict()
    v.require(all(torch.equal(value.float(), loaded[key].float()) for key,value in weights.items()), 'Frozen weights not loaded')
    def predict(im):
        return {'detection':model([im],batch_size=1)[0].model_dump(mode='json'),'layout':None}
    image = Image.open(image_path).convert('RGB')
    image_hash = digest(image_path)
    page_id = f'page-{page_index}-{image_hash[:16]}'
    started = time.monotonic()
    raw = predict(image)
    save(output/'detector-raw.json',raw)
    doc,_ = shared.normalize('surya',raw,image,page_id,{'runtime':RUNTIME,'image_sha256':image_hash,
        'implementation_sha256':digest(Path(shared.__file__))})
    save(output/'shared-parent.json',doc)
    # The accepted chain obtained local detections from the frozen candidate1
    # stage, then reused those detections under the later shared adapter. Using
    # the later title grouping to select ROIs changes line-height and pixels.
    # Run both existing stages in their original order; never load old outputs.
    proposal_doc,_ = proposal_adapter.normalize('surya',raw,image,page_id,{'runtime':RUNTIME,'image_sha256':image_hash})
    save(output/'proposal-parent.json',proposal_doc)
    previous_adapter = local.s
    try:
        local.s = proposal_adapter
        proposals = local.propose(image,proposal_doc)
    finally:
        local.s = previous_adapter
    save(output/'local-proposals.json',proposals)
    existing = [f['bbox'] for f in doc['source_fragments']]
    accepted = []
    records = []
    receipt_paths = []
    for index,region in enumerate(proposals['selected']):
        crop = image.crop(tuple(region['bbox']))
        crop = crop.resize((crop.width*2,crop.height*2))
        cp = output/f'local-{index:02d}.png'
        crop.save(cp)
        raw_local = predict(crop)
        receipt = {'page_id':page_id,'provider':'surya','roi':region,'scale':2,'image_sha256':image_hash,
                   'crop_sha256':digest(cp),'raw':raw_local,'mapped':[],'status':'ok'}
        for j,fragment in enumerate(a.raw_boxes('surya',raw_local)):
            polygon = [[x/2+region['bbox'][0],y/2+region['bbox'][1]] for x,y in fragment['polygon']]
            box = a.envelope([[x,y,x,y] for x,y in polygon])
            duplicate = any(a.overlap(box,z)>.5*min(a.area(box),a.area(z)) for z in existing+accepted)
            target = a.overlap(box,region['component_bbox'])>=.5*a.area(region['component_bbox'])
            edge = box[0]<=region['bbox'][0]+1 or box[1]<=region['bbox'][1]+1 or box[2]>=region['bbox'][2]-1 or box[3]>=region['bbox'][3]-1
            ok = target and not duplicate and not edge
            receipt['mapped'].append({'source_local_index':j,'bbox':box,'polygon':polygon,'confidence':fragment['confidence'],
                'accepted':ok,'duplicate':duplicate,'supports_trigger_component':target,'touches_crop_boundary':edge})
            if ok:
                accepted.append(box)
        # Stable content-derived IDs: elapsed time belongs in execution metadata,
        # not geometry. No change to the existing receipt-hash based ID algorithm.
        name = f'local-{index:02d}.json'
        save(output/name,receipt)
        records.append({'path':name,'sha256':digest(output/name),'receipt':receipt})
        receipt_paths.append(name)
    parent,_ = integrate(doc,image,records)
    parent = recover(parent,image,records)
    save(output/'parent.json',parent)
    spatial = spatial_supports.build(parent,image)
    save(output/'spatial.json',spatial)
    contract = {'contract':'mpdf-spatial-transcription/2','page_index':page_index,'image_sha256':image_hash,'spatial':spatial}
    save(output/'contract.json',contract)
    save(output/'geometry-index.json',{'runtime':RUNTIME,'page_index':page_index,'page_id':page_id,
        'image_sha256':image_hash,'parent_geometry_sha256':parent['geometry_sha256'],
        'files':{name:digest(output/name) for name in ['input.png','detector-raw.json','proposal-parent.json','shared-parent.json','local-proposals.json','parent.json','spatial.json','contract.json']+receipt_paths},
        'receipt_paths':receipt_paths,'full_page_inferences':1,'local_inferences':len(records),
        'supports':len(spatial['supports']),'units':len(spatial['units']),
        'multi_units':sum(len(u['support_ids'])>1 for u in spatial['units']),
        'elapsed_seconds':time.monotonic()-started,'cached_detector_output':False})
    print(v.canonical({'stage':'geometry','page_id':page_id,'supports':len(spatial['supports']),'multi_units':sum(len(u['support_ids'])>1 for u in spatial['units'])}),flush=True)


def prepared(directory, pins):
    directory = Path(directory)
    expected = v.ExpectedArtifacts(**{k:pins[k] for k in v.ExpectedArtifacts.__dataclass_fields__})
    receipts = {}
    for name, expected_hash in pins['receipt_sha256'].items():
        # Only explicit local receipt basenames pinned by the Rust caller.
        v.require(name.startswith('local-') and name.endswith('.json') and Path(name).name==name,'Receipt path')
        raw = (directory/name).read_bytes()
        v.hash_matches(raw,expected_hash,'Selected local receipt')
        receipts[name] = raw
    return v.prepare((directory/'contract.json').read_text(),(directory/'parent.json').read_bytes(),
        (directory/'spatial.json').read_bytes(),(directory/'input.png').read_bytes(),expected,receipts)


def prepare(directory, toc):
    directory = Path(directory)
    pins = read(directory/'expected-inputs.json')
    p = prepared(directory,pins)
    geometry = v.parse(p.geometry_json)
    context = {'schema':'mpdf-toc-context/1','page_id':geometry['spatial']['page_id'],
               'image_sha256':p.expected.image_sha256,'scope':'source_reviewed_toc_region' if toc else 'not_a_toc',
               'unit_ids':[u['unit_id'] for u in geometry['spatial']['units']] if toc else [],
               'selection':'explicit document/page scope in caller-pinned factory config'}
    save(directory/'context.json',context)
    save(directory/'plan.json',batches.plan(p,MODEL))
    with (directory/'input-manifest.json').open('x') as f:
        f.write(p.manifest_json)
    save(directory/'preparation.json',{'schema':'mpdf-spatial-factory-preparation/1','expected':pins,
        'input_manifest_sha256':p.manifest_sha256,'plan_sha256':digest(directory/'plan.json'),
        'context_sha256':digest(directory/'context.json'),'model':MODEL})
    print(v.canonical({'stage':'prepared','batches':len(read(directory/'plan.json')['batches'])}),flush=True)


def verify(directory):
    directory = Path(directory)
    pins = read(directory/'expected-inputs.json')
    p = prepared(directory,pins)
    dispatch = read(directory/'dispatch-pins.json')
    v.hash_matches((directory/'input-manifest.json').read_bytes(),dispatch['input_manifest_sha256'],'Selected manifest')
    v.require(p.manifest_json==(directory/'input-manifest.json').read_text(),'Prepared manifest changed')
    context = (directory/'context.json').read_bytes()
    v.hash_matches(context,dispatch['context_sha256'],'Selected context')
    receipt_raw = (directory/'broker-response.json').read_bytes()
    result_pin = read(directory/'result-pins.json')
    v.hash_matches(receipt_raw,result_pin['broker_response_sha256'],'Selected broker response')
    result = v.parse(receipt_raw)
    v.require(result['schema']=='mpdf-spatial-broker-result/1' and result['complete_page'] is True
              and result['plan_sha256']==dispatch['plan_sha256'],'Broker identity')
    response, aggregation = audit.verify_completed_run(p,directory/'execution',dispatch['plan_sha256'],result['aggregation_sha256'])
    v.require(result['response_sha256']==aggregation['response_sha256'],'Broker response selection')
    e, projection = leader.build(p,response,aggregation['response_sha256'],context,dispatch['context_sha256'])
    for name,value in [('leader-evidence.json',e),('projection.json',projection)]:
        if (directory/name).exists():
            v.require(read(directory/name)==value,'Saved semantic evidence changed')
        else:
            save(directory/name,value)
    leader.verify(p,response,aggregation['response_sha256'],context,dispatch['context_sha256'],e,projection)
    summary = {'schema':'mpdf-spatial-factory-audit/1','response_sha256':aggregation['response_sha256'],
        'aggregation_sha256':result['aggregation_sha256'],'projection_sha256':digest(directory/'projection.json'),
        'context_sha256':dispatch['context_sha256'],'plan_sha256':dispatch['plan_sha256'],
        'input_manifest_sha256':p.manifest_sha256,'complete_page':True,'usage':aggregation['usage'],
        'confirmed_leaders':sum(s['leader_present'] for s in projection['supports']),
        'nfc_changes':sum(len(read(folder/'normalization.json')['changes']) for folder in (directory/'execution').glob('batch-*'))}
    if (directory/'audit.json').exists():
        v.require(read(directory/'audit.json')==summary,'Saved audit changed')
    else:
        save(directory/'audit.json',summary)
    print(v.canonical({'stage':'audited','complete':True,'confirmed_leaders':summary['confirmed_leaders'],'nfc_changes':summary['nfc_changes']}),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('mode',choices=['geometry','prepare','verify'])
    parser.add_argument('--directory',type=Path,required=True)
    parser.add_argument('--model-dir',type=Path)
    parser.add_argument('--page-index',type=int,default=0)
    parser.add_argument('--toc',action='store_true')
    args=parser.parse_args()
    if args.mode=='geometry':
        infer(args.directory/'input.png',args.directory,args.model_dir,args.page_index)
    elif args.mode=='prepare':
        prepare(args.directory,args.toc)
    else:
        verify(args.directory)
