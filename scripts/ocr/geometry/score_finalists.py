"""Logical-line and D-ready preservation diagnostics, dev references only."""
import json,statistics as st
from collections import Counter,defaultdict
from pathlib import Path
from run_dev_geometry_bakeoff import MANIFEST,verify,sha,write,metrics,summarize,match_boxes
from run_finalist_bakeoff import OUT
from finalist_adapters import area,overlap,validate,digest

def union_area(boxes):
    xs=sorted({b[i] for b in boxes for i in [0,2]});total=0.
    for x0,x1 in zip(xs,xs[1:]):
        spans=sorted((b[1],b[3]) for b in boxes if b[0]<x1 and b[2]>x0)
        length=0.;end=-float('inf')
        for a,b in spans:
            length+=max(0,b-max(a,end));end=max(end,b)
        total+=(x1-x0)*length
    return total

def support(gold,fragments):
    clips=[[max(gold[0],b[0]),max(gold[1],b[1]),min(gold[2],b[2]),min(gold[3],b[3])] for b in fragments]
    clips=[b for b in clips if area(b)>0];inter=union_area(clips)
    total=union_area(fragments);ga=area(gold)
    return {'coverage':inter/ga if ga else 0,'iou':inter/(ga+total-inter) if ga+total-inter else 0}

def reference_partition(record):
    result=[]
    for line in sorted(record['lines'],key=lambda l:l['reading_order']):
        col=line.get('column_id');leaf=line.get('leaf_id') or 'single'
        column=None if col in {'furniture','margin'} else leaf+(':'+col if col in {'left','right'} else ':track')
        category='marginalia' if col=='margin' else 'apparatus' if col=='apparatus' else 'toc' if col=='toc' else None
        result.append({'line_id':line['line_id'],'bbox':line['bbox'],'reading_order':line['reading_order'],
                       'column':column,'category':category,'legacy_column_hint':col,'legacy_leaf_hint':line.get('leaf_id')})
    return result

def page_metrics(ref,doc):
    lines=doc['logical_lines'];matches=match_boxes(ref,lines);matched={m['gold']:m for m in matches}
    base=summarize([metrics(ref,lines)]);frags={f['fragment_id']:f for f in doc['fragments']}
    sup={m['gold']:support(ref[m['gold']]['bbox'],[frags[f]['bbox'] for f in lines[m['candidate']]['fragment_ids']]) for m in matches}
    base['support_union_coverage_all_reference']=sum(v['coverage'] for v in sup.values())/len(ref)
    base['support_union_iou_matched']=st.fmean(v['iou'] for v in sup.values()) if sup else 0
    base['support_union_coverage_matched']=st.fmean(v['coverage'] for v in sup.values()) if sup else 0
    cat=[];run=[]
    for i,g in enumerate(ref):
        if i not in matched:run.append(g['line_id'])
        else:
            if len(run)>=5:cat.append({'kind':'consecutive_omission','line_ids':run})
            run=[]
    if len(run)>=5:cat.append({'kind':'consecutive_omission','line_ids':run})
    tracks=defaultdict(list)
    for i,g in enumerate(ref):
        if g['column']:tracks[g['column']].append(i)
    for c,indices in [('whole_page',list(range(len(ref)))),*tracks.items()]:
        misses=[ref[i]['line_id'] for i in indices if i not in matched];recall=1-len(misses)/len(indices)
        if len(indices)>=5 and recall<.8:cat.append({'kind':'low_page_or_column_recall','track':c,'recall':recall,'line_ids':misses})
    eligible=[i for i,g in enumerate(ref) if g['column']];common=[i for i in eligible if i in matched]
    within=cross=within_inv=cross_inv=partition_correct=0
    for a,i in enumerate(common):
        for j in common[a+1:]:
            same=ref[i]['column']==ref[j]['column'];pi=matched[i]['candidate'];pj=matched[j]['candidate'];inv=pi>pj
            if same:within+=1;within_inv+=inv
            else:cross+=1;cross_inv+=inv
            partition_correct+=(same==(lines[pi]['column_id']==lines[pj]['column_id']))
    pairs=within+cross;allpairs=len(eligible)*(len(eligible)-1)//2
    pred_cols={lines[matched[i]['candidate']]['column_id'] for i in common}
    exact=partition_correct==pairs and len(common)>0
    column={'eligible_reference_lines':len(eligible),'matched_reference_lines':len(common),'pairs':pairs,'all_reference_pairs':allpairs,
        'correct_partition_pairs':partition_correct,'pair_accuracy':partition_correct/pairs if pairs else None,'pair_coverage':pairs/allpairs if allpairs else None,
        'reference_track_count':len(tracks),'matched_predicted_track_count':len(pred_cols),'count_agreement':len(tracks)==len(pred_cols),
        'exact_partition_on_matched':exact,'complete_exact_partition':exact and len(common)==len(eligible),
        'within_pairs':within,'within_inverted':within_inv,'within_accuracy':1-within_inv/within if within else None,
        'cross_pairs':cross,'cross_inverted':cross_inv,'cross_accuracy':1-cross_inv/cross if cross else None}
    specials={}
    for catname in ['toc','apparatus','marginalia']:
        inds=[i for i,g in enumerate(ref) if g['category']==catname];good=[i for i in inds if i in matched]
        residual=[];contaminated=[];multiple=[]
        for i in inds:
            g=ref[i]['bbox']
            overlapping=[j for j,l in enumerate(lines) if overlap(l['bbox'],g)>=.5*area(l['bbox']) and area(l['bbox'])>=.015*area(g)]
            if len(overlapping)>1:residual.append({'reference_line_id':ref[i]['line_id'],'logical_line_ids':[lines[j]['line_id'] for j in overlapping]})
            if i not in matched:continue
            l=lines[matched[i]['candidate']]
            if len(l['fragment_ids'])>1:multiple.append(ref[i]['line_id'])
            other=[r['line_id'] for k,r in enumerate(ref) if k!=i and overlap(r['bbox'],l['bbox'])>.5*area(r['bbox'])]
            if other:contaminated.append({'reference_line_id':ref[i]['line_id'],'other_reference_lines':other})
        specials[catname]={'reference_lines':len(inds),'matched':len(good),'missed':[ref[i]['line_id'] for i in inds if i not in matched],
            'multi_fragment_logical_realizations':multiple,'residual_fragmentation':residual,'contaminated_matches':contaminated,
            'union_coverage_all_reference':sum(sup[i]['coverage'] for i in good)/len(inds) if inds else None}
    return {**base,'page_id':doc['page_id'],'column':column,'catastrophic_omissions':cat,'specials':specials,
            'missing_line_ids':[g['line_id'] for i,g in enumerate(ref) if i not in matched],
            'inverted_pairs':base['pairs']-base['correct_pairs'],
            'normalization':{'source_boxes':len(doc['source_fragments']),'fragments':len(doc['fragments']),'logical_lines':len(lines),
                            'column_bands':len(doc['column_bands']),'column_method':doc['column_method'],
                            'margin_lanes':len(doc['margin_lanes']),'margin_splits':len(doc['derivations']),
                            'dotted_row_hints':sum(any(h['kind']=='dotted_row_candidate' for h in l['hints']) for l in lines)}}

def aggregate(pages):
    countkeys=['gold','predicted','matched','matched_at_50','iou_sum','coverage_sum','pairs','correct_pairs','gold_pairs']
    result=summarize([{k:p[k] for k in countkeys} for p in pages])
    result['inverted_pairs']=result['pairs']-result['correct_pairs'];result['macro_page_f1']=st.fmean(p['f1'] for p in pages)
    result['union_coverage_all_reference']=sum(p['support_union_coverage_all_reference']*p['gold'] for p in pages)/result['gold']
    result['union_iou_matched']=sum(p['support_union_iou_matched']*p['matched'] for p in pages)/result['matched'] if result['matched'] else 0
    result['catastrophic_page_ids']=[p['page_id'] for p in pages if p['catastrophic_omissions']]
    cols={k:sum(p['column'][k] for p in pages) for k in ['eligible_reference_lines','matched_reference_lines','pairs','all_reference_pairs','correct_partition_pairs','within_pairs','within_inverted','cross_pairs','cross_inverted']}
    for label,total,correct in [('pair_accuracy','pairs','correct_partition_pairs')]:cols[label]=cols[correct]/cols[total] if cols[total] else None
    cols['pair_coverage']=cols['pairs']/cols['all_reference_pairs']
    for kind in ['within','cross']:cols[kind+'_accuracy']=1-cols[kind+'_inverted']/cols[kind+'_pairs'] if cols[kind+'_pairs'] else None
    for key in ['count_agreement','exact_partition_on_matched','complete_exact_partition']:cols[key+'_pages']=sum(p['column'][key] for p in pages)
    result['columns']=cols;specials={}
    for cat in ['toc','apparatus','marginalia']:
        records=[p['specials'][cat] for p in pages];total=sum(r['reference_lines'] for r in records);mt=sum(r['matched'] for r in records)
        specials[cat]={'reference_lines':total,'matched':mt,'missed':total-mt,'recall':mt/total if total else None,
            'union_coverage_all_reference':sum((r['union_coverage_all_reference'] or 0)*r['reference_lines'] for r in records)/total if total else None,
            **{key:sum(len(r[key]) for r in records) for key in ['multi_fragment_logical_realizations','residual_fragmentation','contaminated_matches']}}
    result['specials']=specials
    return result

def main():
    m=json.loads(MANIFEST.read_text());verify(m)
    refs={p['page_id']:reference_partition(json.loads(Path(p['reference_path']).read_text())) for p in m['pages']}
    write(OUT/'evaluation-partition.json',{'status':'evaluation-only geometric partition using existing dev hints; not human D Gold',
         'excluded_from_adapter':True,'source_manifest_sha256':sha(MANIFEST),'pages':refs})
    report={'schema':'museion-geometry-finalists-comparison/1','candidates':{}}
    snapshot=json.loads((OUT/'implementation-snapshot-final.json').read_text())
    for f,h in snapshot['files'].items():assert sha(f)==h
    for name in ['paddle','surya']:
        info=json.loads((OUT/f'{name}-run.json').read_text());assert len(info['runs'])==50 and info['inputs_unchanged']
        final=json.loads((OUT/f'{name}-normalized.json').read_text());assert len(final['runs'])==50
        passes=[];rawpasses=[];docs={};failures=[]
        for attempt in [1,2]:
            pages=[];rawmetrics=[]
            for p in m['pages']:
                entry=next(x for x in info['runs'] if x['page_id']==p['page_id'] and x['attempt']==attempt)
                if entry['status']!='ok':failures.append(entry);raise RuntimeError('Candidate inference/adapter failure retained; resolve before scoring')
                assert sha(OUT/entry['file'])==entry['sha256']
                norm=next(x for x in final['runs'] if x['page_id']==p['page_id'] and x['attempt']==attempt)
                assert sha(OUT/norm['file'])==norm['sha256']
                doc=json.loads((OUT/norm['file']).read_text());validate(doc)
                assert doc['provider_raw_sha256']==entry['raw_digest']
                assert digest({k:v for k,v in doc.items() if k!='geometry_sha256'})==doc['geometry_sha256']
                docs[p['page_id'],attempt]=doc
                pages.append(page_metrics(refs[p['page_id']],doc))
                rawmetrics.append(metrics(refs[p['page_id']],doc['source_fragments']))
            passes.append({'aggregate':aggregate(pages),'pages':pages});rawpasses.append(summarize(rawmetrics))
        same_raw=sum(docs[p['page_id'],1]['provider_raw_sha256']==docs[p['page_id'],2]['provider_raw_sha256'] for p in m['pages'])
        same_geo=sum(docs[p['page_id'],1]['geometry_sha256']==docs[p['page_id'],2]['geometry_sha256'] for p in m['pages'])
        report['candidates'][name]={'passes':passes,'raw_detector_metrics':rawpasses,'raw_deterministic_pages':same_raw,'geometry_deterministic_pages':same_geo,
             'metrics_deterministic':passes[0]==passes[1],'failures':failures,
             'median_seconds':st.median(x['inference_seconds']+y['seconds'] for x in info['runs'] for y in final['runs'] if x['page_id']==y['page_id'] and x['attempt']==y['attempt']),
             'execution_note':'Two independent model passes followed by two normalizations of the respective retained full raw outputs; r2.2 fixes split child polygon provenance before scoring.'}
    report['ranking']=sorted(report['candidates'],key=lambda n:(-report['candidates'][n]['passes'][0]['aggregate']['f1'],-report['candidates'][n]['passes'][0]['aggregate']['f1_at_50'],-report['candidates'][n]['passes'][0]['aggregate']['order_accuracy']))
    write(OUT/'results.json',report)
    print(json.dumps({n:{'aggregate':c['passes'][0]['aggregate'],'raw':c['raw_detector_metrics'][0],'raw_repeat':c['raw_deterministic_pages'],'geometry_repeat':c['geometry_deterministic_pages']} for n,c in report['candidates'].items()},indent=2))

if __name__=='__main__':main()
