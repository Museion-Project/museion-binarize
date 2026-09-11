"""Document-open printed pagination from visible native folios or Vision margins.

Observations, piecewise offset rules and inferred targets remain distinct. No
TOC values, title guesses, source outlines or evaluation labels seed the rules.
All physical page indices are zero based; printed Arabic/Roman values are not.
"""
import copy
import json
import random
import re
import time
from collections import Counter, defaultdict
from pathlib import Path

import pdf_backend as pdf
from bookmarks import number, save, sha

SCHEMA = 'mpdf-printed-pagination/1'
MARGIN = .14
MAX_VISION_PAGES = 18  # ceiling, never a required number of exact folios


def parsed_folio(text):
    text = text.strip().strip('–—-·[] ')
    text = re.sub(r'^(?:page|p\.?|seite)\s+', '', text, flags=re.I)
    parsed = number(text)
    return parsed if parsed and parsed[1] > 0 else None


def candidates_from_words(words, width, height, page_index, kind, evidence_ref):
    """Read only margin numerals, isolated lines or separated running folios."""
    if any(w[4].lower().strip(':') in ('contents','inhaltsverzeichnis','sommaire') and w[1]<height*.25 for w in words):
        return []
    lines = defaultdict(list)
    for word in words:
        lines[(word[5], word[6])].append(word)
    result = []
    for line in lines.values():
        line.sort(key=lambda w: w[0])
        for index, word in enumerate(line):
            n = parsed_folio(word[4])
            if not n:
                continue
            x0, y0, x1, y1 = word[:4]
            cy = (y0 + y1) / 2
            if not (cy < height * MARGIN or cy > height * (1-MARGIN)):
                continue
            explicit = index > 0 and line[index-1][4].lower().rstrip('.') in ('page', 'p', 'seite')
            nearby = [v for v in words if v is not word and abs((v[1]+v[3])/2-cy)<max(y1-y0,v[3]-v[1])*.55 and min(abs(v[0]-x1),abs(x0-v[2]))<12]
            isolated = len(line) == 1 and not nearby
            separated = ((index == 0 and x0 < width*.22 and len(line)>1 and line[1][0]-x1 > 10)
                         or (index == len(line)-1 and x1 > width*.78 and index>0 and x0-line[index-1][2] > 10))
            if not (isolated or explicit or separated):
                continue
            result.append(dict(pdf_page=page_index, family=n[0], printed_value=n[1], text=word[4],
                               bbox=[x0,y0,x1-x0,y1-y0], band='top' if cy<height/2 else 'bottom',
                               kind=kind, confidence=word[8] if len(word)>8 else 1.0, evidence_ref=evidence_ref, explicit=explicit, side=('left' if (x0+x1)/2<width/2 else 'right') if width>height*1.1 else 'single'))
    return result


def build_model(observations, page_count, excluded_pages=()):
    excluded = set(excluded_pages)
    observations = [o for o in observations if o['pdf_page'] not in excluded and o.get('confidence',1)>=.2]
    # A spread needs paired consecutive folios on opposite halves of multiple
    # real pages; landscape orientation alone is not evidence of two-up pages.
    by_page = defaultdict(list)
    for o in observations:by_page[o['pdf_page']].append(o)
    paired=set()
    for page,items in by_page.items():
        if any(a.get('side')=='left' and b.get('side')=='right' and a['family']==b['family'] and b['printed_value']==a['printed_value']+1 for a in items for b in items):paired.add(page)
    spread=len(paired)>=2
    def key(o):
        step=2 if spread and o.get('side') in ('left','right') and o['family']=='arabic' else 1
        # Printed = step * physical_index + base (+1 for the right folio).
        base=o['printed_value']-step*o['pdf_page']-(1 if step==2 and o['side']=='right' else 0)
        return o['family'],step,base
    groups=defaultdict(set)
    for o in observations:groups[key(o)].add(o['pdf_page'])
    selected=[];conflicts=[]
    for page,items in sorted(by_page.items()):
        def score(o):return (bool(o.get('explicit')),len(groups[key(o)]),o.get('confidence',1))
        items.sort(key=score,reverse=True);best=items[0]
        if len(groups[key(best)])<2 and not best.get('explicit'):continue
        if any(score(c)==score(best) and key(c)!=key(best) for c in items[1:]):
            conflicts.append(page);continue
        selected.append(best)
    all_runs=[]
    for o in selected:
        if not all_runs or all_runs[-1]['key']!=key(o):all_runs.append(dict(key=key(o),anchors=[]))
        all_runs[-1]['anchors'].append(o)
    runs=[]
    for run in all_runs:
        if len({o['pdf_page'] for o in run['anchors']})<2:continue
        if runs and runs[-1]['key']==run['key']:runs[-1]['anchors'].extend(run['anchors'])
        else:runs.append(run)
    rules=[]
    for i,run in enumerate(runs):
        family,step,base=run['key'];first=run['anchors'][0]['pdf_page'];last=run['anchors'][-1]['pdf_page']
        start=max(0,(1-base)//step);end=page_count-1
        before=[o for o in selected if o['pdf_page']<first and key(o)!=run['key']]
        after=[o for o in selected if o['pdf_page']>last and key(o)!=run['key']]
        if before and not before[-1]['pdf_page']<start<=first:start=first
        if after:
            next_o=after[0];_,next_step,next_base=key(next_o);next_start=max(0,(1-next_base)//next_step)
            end=next_start-1 if last<next_start<=next_o['pdf_page'] else last
        rules.append(dict(id=f'segment-{i+1}',family=family,printed_per_pdf_page=step,printed_base=base,
                          start_pdf_page=start,end_pdf_page=end,first_observed=first,last_observed=last,
                          anchor_count=len({o['pdf_page'] for o in run['anchors']}),anchor_pages=sorted({o['pdf_page'] for o in run['anchors']}),
                          boundary_kind='inferred_from_consistent_folios',state='supported'))
    gaps=[]
    for left,right in zip(rules,rules[1:]):
        if left['end_pdf_page']+1<right['start_pdf_page']:gaps.append([left['end_pdf_page']+1,right['start_pdf_page']-1])
    # Keep exact visible folios outside regular regions (e.g. a reflowed book)
    # but drop unrelated header/footnote numerals when another strong sequence
    # owns that physical page. Paired left/right folios are both retained.
    selected_keys={o['pdf_page']:key(o) for o in selected}
    reliable=[o for o in observations if o.get('explicit') or key(o)==selected_keys.get(o['pdf_page'])]
    sequence_count=0;previous=None
    for run in runs:
        first=run['anchors'][0];last=run['anchors'][-1]
        if previous is None or first['family']!=previous['family'] or first['printed_value']<previous['printed_value']:
            sequence_count+=1
        previous=last
    return dict(schema=SCHEMA,page_count=page_count,observations=reliable,raw_observations=observations,rules=rules,
                sequence_count=sequence_count,spread=spread,conflicts=conflicts,unresolved_boundaries=gaps,
                status='ready' if rules and not conflicts and not gaps else 'partial' if rules or reliable else 'unavailable')


def target_candidates(model, family, value):
    if family is None or value is None:
        return []
    observed=[o for o in model['observations'] if o['family']==family and o['printed_value']==value]
    # Exact original folios outrank extrapolation in nonuniform reflowed PDFs.
    if observed:return [dict(pdf_page=p,kind='observed_folio',evidence_ref=next(o['evidence_ref'] for o in observed if o['pdf_page']==p)) for p in sorted({o['pdf_page'] for o in observed})]
    candidates={}
    for rule in model['rules']:
        if rule['family']!=family:
            continue
        page=(value-rule['printed_base'])//rule['printed_per_pdf_page']
        if rule['start_pdf_page']<=page<=rule['end_pdf_page']:
            candidates[page]=dict(pdf_page=page,kind='inferred_segment',rule_id=rule['id'],anchor_pages=rule['anchor_pages'])
    # Exact observed folios can map a short singleton segment without pretending
    # one observation proved a rule. Multiple candidates remain explicit.
    for o in model['observations']:
        if o['family']==family and o['printed_value']==value:
            candidates[o['pdf_page']]=dict(pdf_page=o['pdf_page'],kind='observed_folio',evidence_ref=o['evidence_ref'])
    return [candidates[p] for p in sorted(candidates)]


def map_table(table, model):
    if model.get('source_sha256')!=table.get('source_sha256') or model['page_count']!=table['page_count']:
        raise ValueError('pagination source does not match contents')
    t=copy.deepcopy(table)
    # A contents numeral cannot be its own independent pagination witness.
    excluded={e['source_page'] for e in t['entries']+t.get('source_entries',[])}
    applicable=build_model(model.get('raw_observations',model['observations']),model['page_count'],excluded)
    for entries in [t['entries'],t.get('source_entries',[])]:
        candidate_sets=[target_candidates(applicable,e.get('printed_family'),e.get('printed_value')) for e in entries]
        positions=[i for i,c in enumerate(candidate_sets) if c]
        # Forward/backward feasibility across the complete TOC, rather than a
        # greedy "first offset wins" when Arabic numbering restarts.
        forward=[]
        for k,i in enumerate(positions):
            pages={c['pdf_page'] for c in candidate_sets[i]}
            forward.append(pages if k==0 else {p for p in pages if any(q<=p for q in forward[-1])})
        backward=[set() for _ in positions]
        for k in range(len(positions)-1,-1,-1):
            pages={c['pdf_page'] for c in candidate_sets[positions[k]]}
            backward[k]=pages if k==len(positions)-1 else {p for p in pages if any(p<=q for q in backward[k+1])}
        feasible={i:forward[k]&backward[k] for k,i in enumerate(positions)} if forward and forward[-1] else {}
        for i,(entry,candidates) in enumerate(zip(entries,candidate_sets)):
            entry['target_pdf_page']=None
            entry['review_reasons']=[r for r in entry.get('review_reasons',[]) if r not in ('pagination_uninspected','pagination_ambiguous','native_header_mapping_requires_inspection')]
            entry['pagination_candidates']=candidates
            pages={c['pdf_page'] for c in candidates}
            if len(pages)>1 and i in feasible:
                pages=feasible[i]
            if len(pages)==1:
                page=next(iter(pages));entry['target_pdf_page']=page
                entry['pagination_evidence']=[c for c in candidates if c['pdf_page']==page]
            else:
                entry['review_reasons'].append('pagination_ambiguous' if pages else 'pagination_uninspected')
            entry['state']='needs_review' if entry['review_reasons'] else 'ready'
    t['pagination_model']=dict(schema=SCHEMA,status=applicable['status'],rules=applicable['rules'],
                              source_sha256=model['source_sha256'],evidence_path=model.get('evidence_path'),
                              excluded_contents_pages=sorted(excluded))
    return t


def sample_pages(page_count, seed):
    if page_count<=8:
        return list(range(page_count))
    rng=random.Random(seed);front=min(18,page_count)
    intervals=[(1,max(1,front//3)),(max(1,front//3),max(2,front*2//3)),(max(2,front*2//3),front-1)]
    intervals += [(int(page_count*a),min(page_count-1,int(page_count*b))) for a,b in [(.2,.3),(.43,.55),(.65,.75),(.83,.93),(.96,.995)]]
    return sorted({rng.randint(min(a,b),max(a,b)) for a,b in intervals})


def scan_margins(doc, indices, root, batch, worker, digest, check_cancel):
    import subprocess
    output=root/f'sample-{batch}';output.mkdir();requests=[]
    for index in indices:
        check_cancel(root);page=doc[index];w,h=page.rect.width,page.rect.height
        for band,rect in [(f'{edge}-{tile}',pdf.Rect(w*tile/3,y0,w*(tile+1)/3,y1)) for edge,y0,y1 in [('top',0,h*MARGIN),('bottom',h*(1-MARGIN),h)] for tile in range(3)]:
            image=output/f'p{index}-{band}.png';began=time.perf_counter()
            page.render(dpi=220,clip=rect).save(image)
            requests.append(dict(id=f'p{index}-{band}',image_path=str(image),source=str(Path(doc.name).resolve()),source_sha256=digest,
                                 page_index=index,page_count=len(doc),width=rect.width,height=rect.height,source_roi=list(rect),
                                 page_width=w,page_height=h,band=band,render_seconds=time.perf_counter()-began,stage='pagination_margin',renderer=pdf.RENDERER,dpi=220))
    save(output/'requests.json',requests)
    child=subprocess.Popen([worker,str(output/'requests.json'),str(output/'vision')],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    started=time.perf_counter()
    try:
        while child.poll() is None:
            check_cancel(root)
            if time.perf_counter()-started>15:raise TimeoutError("Apple Vision 页码抽样超时。")
            time.sleep(.03)
        if child.returncode:
            raise RuntimeError('Apple Vision 页码读取失败：'+child.stderr.read().decode(errors='replace')[-500:])
    finally:
        if child.poll() is None:child.kill();child.wait()
    observations=[]; page_words=defaultdict(list);page_meta={}
    for path in sorted((output/'vision').glob('*.json')):
        raw=json.loads(path.read_text())
        if raw['state']=='failed':raise RuntimeError('Apple Vision 页码读取失败：'+raw.get('error',''))
        roi=raw['source_roi'];words=[]
        for i,row in enumerate(raw['observations']):
            for k,token in enumerate(row.get('tokens',[])):
                b=token['bbox'];x=roi[0]+b[0]*raw['width'];y=roi[1]+(1-b[1]-b[3])*raw['height']
                words.append((x,y,x+b[2]*raw['width'],y+b[3]*raw['height'],token['text'],i,0,k,row.get('confidence',0)))
        page_words[raw['page_index']].extend([(*word[:5],str(path)+':'+str(word[5]),*word[6:]) for word in words])
        page_meta[raw['page_index']]=raw
    for index,words in page_words.items():
        raw=page_meta[index]
        observations.extend(candidates_from_words(words,raw['page_width'],raw['page_height'],index,'apple_vision_margin',str(output/'vision')))
    return observations


def analyze(request,root,stage,check_cancel,vision_worker):
    root=Path(root);source=str(Path(request['source']).resolve());digest=sha(source);began=time.perf_counter()
    stage(root,'checking_text_layer',text_layer='checking',pagination_status='running')
    if request.get('source_sha256') and request['source_sha256']!=digest:raise ValueError('原 PDF 已改变，请重新打开。')
    observations=[];text_pages=0;native_folio_pages=[]
    with pdf.Document(source) as doc:
        if doc.is_encrypted:raise ValueError('加密 PDF 暂不支持自动重建页码，请先保存不加密副本。')
        for index,page in enumerate(doc):
            check_cancel(root);words=page.words()
            if sum(sum(c.isalnum() for c in w[4]) for w in words)>=30:text_pages+=1
            visible=words
            found=candidates_from_words(visible,page.rect.width,page.rect.height,index,'native_margin',f'{root}/native-folios.json#page={index+1}')
            observations.extend(found)
            if found:native_folio_pages.append(index)
        text_layer='absent' if not text_pages else 'present' if text_pages>=len(doc)*.8 else 'mixed'
        save(root/'native-folios.json',dict(source_sha256=digest,page_count=len(doc),text_pages=text_pages,observations=observations))
        model=build_model(observations,len(doc));sampled=[]
        stage(root,'rebuilding_pagination',text_layer=text_layer,pagination_status='running',rule_count=len(model['rules']))
        # Native folios are sufficient even on otherwise scanned pages. An
        # absent text layer still receives a small visual sample as requested.
        need_vision=text_layer!='present' or not model['rules']
        if need_vision:
            worker=vision_worker(root,request['runtime_cache'])
            deadline=time.perf_counter()+12  # OCR sampling budget; compilation is separately visible
            initial=sample_pages(len(doc),int(digest[:16],16))
            for batch in range(5):
                if batch==0:pages=initial
                else:
                    choices=[]
                    if batch==1 and model['rules'] and not any(r['family']=='roman' for r in model['rules']):
                        front_end=min(r['start_pdf_page'] for r in model['rules'])
                        if front_end>=6:choices.extend(range(front_end-2,max(0,front_end-12),-2))
                    for left,right in model['unresolved_boundaries']:choices.append((left+right)//2)
                    # A singleton family/offset deserves one nearby check, not
                    # a fixed requirement to harvest fifteen exact page labels.
                    owned={o['pdf_page'] for o in model['observations']}
                    pending=[o for o in observations if o['pdf_page'] not in owned or o in model['observations']]
                    counts=Counter((o['family'],o['pdf_page']-o['printed_value']) for o in pending)
                    for o in pending:
                        if counts[(o['family'],o['pdf_page']-o['printed_value'])]==1:
                            choices.extend(o['pdf_page']+delta for delta in (1,-1,2,-2,3,-3))
                    if not model['rules']:
                        choices.extend(range(2,min(len(doc),24),3))
                    pages=list(dict.fromkeys(p for p in choices if 0<=p<len(doc) and p not in sampled))[:4]
                pages=[p for p in pages if p not in sampled][:MAX_VISION_PAGES-len(sampled)]
                if not pages or time.perf_counter()>deadline:break
                stage(root,'sampling_page_numbers',text_layer=text_layer,pagination_status='running',sampled_pages=len(sampled),rule_count=len(model['rules']))
                observations.extend(scan_margins(doc,pages,root,batch,worker,digest,check_cancel));sampled.extend(pages)
                model=build_model(observations,len(doc))
                if model['status']=='ready':
                    owned={o['pdf_page'] for o in model['observations']}
                    keys={(o['family'],o['pdf_page']-o['printed_value']) for o in observations if o['pdf_page'] not in owned or o in model['observations']}
                    supported={(r['family'],-r['printed_base']) for r in model['rules'] if r['printed_per_pdf_page']==1}
                    front_unchecked=batch==0 and model['rules'] and min(r['start_pdf_page'] for r in model['rules'])>=6 and not any(r['family']=='roman' for r in model['rules'])
                    if keys<=supported and not front_unchecked:break
        if sha(source)!=digest:raise ValueError('重建期间原 PDF 已改变。')
        # A complete native scan supplies the exact map even if original print
        # pages were reflowed across a variable number of PDF pages.
        if text_layer=='present' and len({o['pdf_page'] for o in model['observations']})>=max(2,len(doc)*.2) and not model['conflicts']:
            model['status']='ready'
        model.update(source=source,source_sha256=digest,text_layer=text_layer,text_pages=text_pages,
                     sampled_pages=sampled,native_folio_pages=native_folio_pages,elapsed_seconds=time.perf_counter()-began,
                     evidence_path=str(root/'pagination.json'),method='native_and_vision_margins' if sampled else 'native_margins',
                     coverage_note='规则基于已观察页码；未逐页视觉验证无插页或未抽中的短编号段。')
        save(root/'pagination.json',model)
        stage(root,'pagination_finished',text_layer=text_layer,pagination_status=model['status'],rule_count=len(model['rules']),sampled_pages=len(sampled))
        return model
