"""Report raw-box and final geometry compatibility; no adapter mutation."""
import json,csv
from pathlib import Path
import jsonschema
from run_dev_geometry_bakeoff import MANIFEST,verify,sha,metrics,summarize
from score_finalists import reference_partition,page_metrics,aggregate
from finalist_adapters import validate,digest
from run_surya_upgrade_probe import OUT,write


def main():
    assert not (OUT/'results.json').exists()
    manifest=json.loads(MANIFEST.read_text());verify(manifest)
    versions=['0.17.0','0.22.1'];runs={v:json.loads((OUT/v/'run.json').read_text()) for v in versions}
    assert all(r['complete'] and len(r['runs'])==25 for r in runs.values())
    assert len({r['adapter_sha256'] for r in runs.values()})==1
    assert runs['0.17.0']['model_sha256']==runs['0.22.1']['model_sha256']
    schema=json.loads(Path('schemas/museion-geometry-evidence-1.schema.json').read_text())
    arrays={v:{'raw':[],'logical':[]} for v in versions};comparison=[]
    fields=['source_fragments','fragments','layout_regions','column_bands','column_method','margin_lanes','derivations','logical_lines','features','semantic_status']
    def without_confidence(x):
        if isinstance(x,dict):return {k:without_confidence(v) for k,v in x.items() if k!='confidence'}
        if isinstance(x,list):return [without_confidence(v) for v in x]
        return x
    for p in manifest['pages']:
        pid=p['page_id'];ref=reference_partition(json.loads(Path(p['reference_path']).read_text()));docs={}
        for v in versions:
            d=json.loads((OUT/v/(pid+'.json')).read_text());docs[v]=d;validate(d);jsonschema.validate(d,schema)
            arrays[v]['raw'].append(metrics(ref,d['source_fragments']));arrays[v]['logical'].append(page_metrics(ref,d))
        a,b=[docs[v] for v in versions]
        ar=a['provider_raw']['detection']['bboxes'];br=b['provider_raw']['detection']['bboxes']
        coord=lambda raw:[{k:x[k] for k in ['bbox','polygon']} for x in raw]
        samecoords=coord(ar)==coord(br)
        delta=max([abs(x['confidence']-y['confidence']) for x,y in zip(ar,br) if x.get('confidence') is not None and y.get('confidence') is not None],default=0) if samecoords else None
        geom_a={k:a[k] for k in fields};geom_b={k:b[k] for k in fields}
        old=json.loads((Path('docs/evidence/surya-apparatus-repair-2026-09-06/normalized')/f'{pid}-pass1.json').read_text())
        comparison.append({'page_id':pid,'raw_boxes_old':len(ar),'raw_boxes_new':len(br),'raw_coordinates_polygons_equal':samecoords,
            'full_raw_payload_equal':a['provider_raw']==b['provider_raw'],'max_confidence_delta_for_equal_boxes':delta,
            'final_geometry_fields_equal':geom_a==geom_b,'final_geometry_except_confidence_equal':without_confidence(geom_a)==without_confidence(geom_b),
            'logical_lines_equal':a['logical_lines']==b['logical_lines'],
            'old_arm_reproduces_repaired_baseline':all(a[k]==old[k] for k in fields),
            'changed_geometry_fields':[k for k in fields if a[k]!=b[k]]})
    result={'scope':'One actual detector pass per release on 25 existing dev pages; fixed adapter and rules',
       'runs':runs,'summary':{v:{'raw':summarize(arrays[v]['raw']),'logical':aggregate(arrays[v]['logical'])} for v in versions},
       'pages':arrays,'comparisons':comparison,'checks':{'schema_provenance':'50/50 PASS','adapter_unchanged':True,'model_sha256_equal':True},
       'equality_counts':{k:sum(c[k] for c in comparison) for k in ['raw_coordinates_polygons_equal','full_raw_payload_equal','final_geometry_fields_equal','final_geometry_except_confidence_equal','logical_lines_equal','old_arm_reproduces_repaired_baseline']}}
    assert sha(Path(__file__).with_name('finalist_adapters.py'))==runs['0.17.0']['adapter_sha256']
    verify(manifest);write(OUT/'results.json',result)
    with (OUT/'pages.csv').open('w') as f:
        w=csv.DictWriter(f,fieldnames=comparison[0].keys());w.writeheader();w.writerows(comparison)
    print(json.dumps({'equality_counts':result['equality_counts'],'summary':result['summary']},indent=2))
if __name__=='__main__':main()
