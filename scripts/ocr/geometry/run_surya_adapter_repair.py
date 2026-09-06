"""Bounded adapter replay of two existing independent Surya detector passes."""
import json
from pathlib import Path
import jsonschema
from PIL import Image,ImageDraw
from finalist_adapters import normalize,SURYA_VERSION,validate
from run_dev_geometry_bakeoff import MANIFEST,verify,sha,write
from score_finalists import reference_partition,page_metrics,aggregate,match_boxes

BASE=Path('docs/evidence/geometry-finalists-r2-2026-09-06')
OUT=Path('docs/evidence/surya-apparatus-repair-2026-09-06')
TARGETS={'burnet-platonis-opera-pdf1100','burnet-platonis-opera-pdf1400'}

def main():
    assert not (OUT/'results.json').exists(),'Do not overwrite completed evidence'
    (OUT/'normalized').mkdir(parents=True,exist_ok=True)
    manifest=json.loads(MANIFEST.read_text());verify(manifest)
    schema=json.loads(Path('schemas/museion-geometry-evidence-1.schema.json').read_text())
    snapshot={str(p):sha(p) for p in [Path(__file__),Path(__file__).with_name('finalist_adapters.py'),Path(__file__).with_name('score_finalists.py')]}
    provenance={'adapter_version':SURYA_VERSION,'implementation_sha256':snapshot,'input_manifest_sha256':sha(MANIFEST),
                'scope':'adapter-only replay; two independently inferred R2 raw passes; no new detector inference'}
    passes=[];digests={};checks=[];baseline=[];specific=[];raw_checks=[]
    for attempt in [1,2]:
        pages=[]
        for p in manifest['pages']:
            pid=p['page_id'];rawpath=BASE/'pages/surya'/f'{pid}-pass{attempt}-raw.json'
            raw=json.loads(rawpath.read_text());old=json.loads((BASE/'normalized/surya'/f'{pid}-pass{attempt}.json').read_text())
            assert raw==old['provider_raw']
            image=Image.open(p['image_path']).convert('RGB')
            d=normalize('surya',raw,image,pid,{**provenance,'image_sha256':p['image_sha256'],'detector_provenance':old['provenance']})
            validate(d);jsonschema.validate(d,schema)
            assert d['source_fragments']==old['source_fragments'] and d['provider_raw']==old['provider_raw']
            for key in ['layout_regions','column_bands','column_method','margin_lanes','semantic_status']:
                assert d[key]==old[key],(pid,key)
            assert all(event in d['derivations'] for event in old['derivations'])
            if pid not in TARGETS:
                assert d['logical_lines']==old['logical_lines'] and d['fragments']==old['fragments'],pid
            ref=reference_partition(json.loads(Path(p['reference_path']).read_text()));before=page_metrics(ref,old);after=page_metrics(ref,d)
            for category in ['toc','marginalia']:assert before['specials'][category]==after['specials'][category]
            oldres=before['specials']['apparatus']['residual_fragmentation'];newres=after['specials']['apparatus']['residual_fragmentation']
            assert {r['reference_line_id'] for r in newres}<={r['reference_line_id'] for r in oldres}
            bm={ref[m['gold']]['line_id']:old['logical_lines'][m['candidate']] for m in match_boxes(ref,old['logical_lines'])}
            am={ref[m['gold']]['line_id']:d['logical_lines'][m['candidate']] for m in match_boxes(ref,d['logical_lines'])}
            for r in ref:
                if r['category']=='apparatus':continue
                rid=r['line_id'];assert (rid in bm)==(rid in am)
                if rid in bm:assert bm[rid]==am[rid],(pid,rid)
            if pid in TARGETS:
                app=after['specials']['apparatus'];assert app['matched']==5 and not app['residual_fragmentation'] and not app['contaminated_matches']
                wanted=['0042','0043','0044','0045'] if pid.endswith('1100') else ['0043','0044','0045']
                selected=[am[pid+'-candidate-'+suffix] for suffix in wanted]
                assert len({l['line_id'] for l in selected})==len(wanted)
                if pid.endswith('1100'):assert set(am[pid+'-candidate-0044']['fragment_ids'])=={'d0032-y2','d0034'}
                if attempt==1:
                    specific.append({'page_id':pid,'lines':selected,'before':before['specials']['apparatus'],'after':app,'row_splits':[e for e in d['derivations'] if e['operation']=='ink_valley_row_split']})
                    # Diagnostic visualization, original pixels plus logical envelopes.
                    crop=image.crop((70,1650,1140,1870));draw=ImageDraw.Draw(crop)
                    for r in ref:
                        if r['category']!='apparatus':continue
                        l=am[r['line_id']];x0,y0,x1,y1=l['bbox'];draw.rectangle([x0-70,y0-1650,x1-70,y1-1650],outline='red',width=2)
                        draw.text((x0-68,y0-1648),r['line_id'][-4:],fill='blue')
                    crop.save(OUT/f'{pid}-logical-lines.png')
            write(OUT/'normalized'/f'{pid}-pass{attempt}.json',d)
            if attempt==1:digests[pid]=d['geometry_sha256'];baseline.append(before)
            else:assert digests[pid]==d['geometry_sha256'],pid
            raw_checks.append({'path':str(rawpath),'sha256':sha(rawpath)})
            pages.append(after)
        passes.append({'attempt':attempt,'aggregate':aggregate(pages),'pages':pages})
    assert passes[0]['aggregate']==passes[1]['aggregate']
    verify(manifest)
    assert all(sha(p)==h for p,h in snapshot.items())
    result={'scope':provenance['scope'],'baseline_intent':'intent.md §8.5','alignment':'aligned','implementation':snapshot,
            'baseline':aggregate(baseline),'passes':passes,'target_acceptance':specific,'raw_inputs':raw_checks,
            'checks':{'schema_and_lossless_provenance':'50/50 PASS','deterministic_geometry':'25/25 PASS',
                      'unchanged_other_pages':'23/23 PASS per pass','non_apparatus_matched_lines_unchanged':'PASS',
                      'no_new_apparatus_residual_fragments':'PASS','column_regions_margin_lanes_retained':'50/50 PASS'},
            'limits':['No new detector inference','Existing apparatus residual fragmentation remains on one other line',
                      'Development geometry acceptance only; not semantic D or production acceptance']}
    write(OUT/'results.json',result)
    print(json.dumps({'checks':result['checks'],'before':result['baseline'],'after':passes[0]['aggregate']},indent=2))

if __name__=='__main__':main()
