"""D-ready, text-free geometry normalization for the two local finalists."""
from __future__ import annotations
import hashlib
import json
import math
import statistics as st
from collections import defaultdict

import cv2
import numpy as np

SCHEMA = 'museion-geometry-evidence/1'
VERSION = 'r2.1'


def digest(x):
    return hashlib.sha256(json.dumps(x,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()


def envelope(boxes):
    return [min(b[0] for b in boxes),min(b[1] for b in boxes),max(b[2] for b in boxes),max(b[3] for b in boxes)]


def area(b):return max(0,b[2]-b[0])*max(0,b[3]-b[1])
def overlap(a,b):return max(0,min(a[2],b[2])-max(a[0],b[0]))*max(0,min(a[3],b[3])-max(a[1],b[1]))
def yc(b):return (b[1]+b[3])/2
def xc(b):return (b[0]+b[2])/2
def height(b):return b[3]-b[1]
def width(b):return b[2]-b[0]


def raw_boxes(provider,payload):
    if provider=='paddle':
        return [{'bbox':envelope([[float(p[0]),float(p[1]),float(p[0]),float(p[1])] for p in poly]),
                 'polygon':poly,'confidence':payload['detection'].get('dt_scores',[None]*len(payload['detection']['dt_polys']))[i]}
                for i,poly in enumerate(payload['detection']['dt_polys'])]
    return [{'bbox':[float(v) for v in x.get('bbox',envelope([[p[0],p[1],p[0],p[1]] for p in x['polygon']]))],
             'polygon':x['polygon'],'confidence':x.get('confidence')} for x in payload['detection']['bboxes']]


def components(mask,bbox):
    x0,y0,x1,y1=map(int,[math.floor(bbox[0]),math.floor(bbox[1]),math.ceil(bbox[2]),math.ceil(bbox[3])])
    crop=mask[max(0,y0):y1,max(0,x0):x1]
    n,_,stats,_=cv2.connectedComponentsWithStats(crop,8)
    return [[float(x+x0),float(y+y0),float(x+x0+w),float(y+y0+h),int(a)]
            for x,y,w,h,a in stats[1:] if a>=2]


def modal_edge(values,scale):
    return min(values,key=lambda v:(-sum(abs(v-w)<=.35*scale for w in values),v))


def margin_split(fragments,mask,page_width):
    """Split only when a whitespace-separated end component recurs in a margin lane."""
    if len(fragments)<8:return fragments,[],[]
    hs=[height(f['bbox']) for f in fragments if width(f['bbox'])>3*height(f['bbox'])]
    h=st.median(hs) if hs else 1
    span=envelope([f['bbox'] for f in fragments]); wide=[f for f in fragments if width(f['bbox'])>.55*width(span) and height(f['bbox'])<1.8*h]
    if len(wide)<8:return fragments,[],[]
    left=modal_edge([f['bbox'][0] for f in wide],h);right=modal_edge([f['bbox'][2] for f in wide],h)
    # A single recurring full-width text track is needed. Spread/column pages are not treated as a margin lane.
    if right-left<.5*width(span):return fragments,[],[]
    proposals=[];short=[]
    for f in fragments:
        b=f['bbox']
        # Repeated leader dots make this a row-grouping cue, not a margin-lane observation.
        row_extent=[span[0],b[1],span[2],b[3]]
        if dotted_run(mask,row_extent,row_extent,h):continue
        if width(b)<1.6*h and (xc(b)<left or xc(b)>right):
            short.append({'source':f['fragment_id'],'bbox':b,'center':xc(b),'y':yc(b),'side':'left' if xc(b)<left else 'right'})
        if width(b)<.55*(right-left) or height(b)>1.8*h:continue
        cc=components(mask,b)
        if not cc:continue
        for side,edge in [('left',left),('right',right)]:
            outer=[c for c in cc if (xc(c)>edge+.10*h if side=='right' else xc(c)<edge-.10*h) and c[4]>=max(2,.01*h*h)]
            if not outer:continue
            ob=envelope([c[:4] for c in outer])
            if width(ob)>1.4*h or height(ob)<.17*h:continue
            inner=[c for c in cc if c not in outer]
            if not inner:continue
            ib=envelope([c[:4] for c in inner])
            gap=ob[0]-ib[2] if side=='right' else ib[0]-ob[2]
            if gap<.18*h or width(ib)<.5*(right-left):continue
            cut=(ob[0]+ib[2])/2 if side=='right' else (ib[0]+ob[2])/2
            proposals.append({'source':f['fragment_id'],'bbox':ob,'center':xc(ob),'y':yc(ob),'side':side,
                              'cut':cut,'valley_width':gap,'ink_component_boxes':[c[:4] for c in outer]})
    lanes=[];events=[];accepted={};lane_sources={}
    for side in ['left','right']:
        candidates=[p for p in proposals+short if p['side']==side]
        while candidates:
            seed=max(candidates,key=lambda a:sum(abs(a['center']-b['center'])<=.7*h for b in candidates))
            group=[p for p in candidates if abs(p['center']-seed['center'])<=.7*h]
            candidates=[p for p in candidates if p not in group]
            ys=sorted(set(round(p['y']/h,1) for p in group))
            if len({p['source'] for p in group})<3 or len(ys)<3 or max(ys)-min(ys)<4:continue
            lid=f'lane-{len(lanes):03d}'
            lanes.append({'lane_id':lid,'side':side,'bbox':envelope([p['bbox'] for p in group]),
                          'support_fragment_ids':sorted({p['source'] for p in group}),'modal_body_edge':left if side=='left' else right,
                          'features':{'support_count':len(group),'vertical_span_in_line_heights':max(ys)-min(ys)}})
            for p in group:
                lane_sources[p['source']]=lid
                if 'cut' in p:accepted[p['source']]={**p,'lane_id':lid}
    output=[]
    for f in fragments:
        fid=f['fragment_id'];p=accepted.get(fid)
        if not p:
            if fid in lane_sources:f['hints'].append({'kind':'recurring_margin_lane','lane_id':lane_sources[fid]})
            output.append(f);continue
        b=f['bbox'];cut=p['cut'];parts=([b[0],b[1],cut,b[3]],[cut,b[1],b[2],b[3]])
        ids=[]
        for i,box in enumerate(parts):
            nf={**f,'fragment_id':fid+f'-s{i}','bbox':box,'hints':list(f['hints']),
                'source_fragment_id':fid,'derivation':{'operation':'whitespace_margin_split','cut_x':cut,'lane_id':p['lane_id']}}
            if (i==1)==(p['side']=='right'):nf['hints'].append({'kind':'recurring_margin_lane','lane_id':p['lane_id']})
            ids.append(nf['fragment_id']);output.append(nf)
        events.append({'operation':'split','source_fragment_id':fid,'output_fragment_ids':ids,**p})
    return output,lanes,events


def adaptive_bands(fragments):
    """Search scale-free x supports and persistent y gaps between crossing lines."""
    if len(fragments)<6:return []
    bb=envelope([f['bbox'] for f in fragments]); span=width(bb)
    h=st.median(height(f['bbox']) for f in fragments)
    broad=[f for f in fragments if width(f['bbox'])>=.27*span and height(f['bbox'])<2*h]
    if len(broad)<6:return []
    candidates=[]
    # Quantile-relative scan samples, not fixed pixel gutter thresholds.
    for cut in np.linspace(bb[0]+.22*span,bb[2]-.22*span,161):
        left=[f for f in broad if f['bbox'][2]<cut];right=[f for f in broad if f['bbox'][0]>cut]
        if min(len(left),len(right))<3:continue
        barriers=sorted((f['bbox'][1],f['bbox'][3]) for f in broad if f not in left and f not in right)
        intervals=[];start=bb[1]
        for a,b in barriers:
            if a>start:intervals.append((start,a))
            start=max(start,b)
        if start<bb[3]:intervals.append((start,bb[3]))
        for y0,y1 in intervals:
            ls=[f for f in left if y0<=yc(f['bbox'])<=y1];rs=[f for f in right if y0<=yc(f['bbox'])<=y1]
            if min(len(ls),len(rs))<3:continue
            lb=envelope([f['bbox'] for f in ls]);rb=envelope([f['bbox'] for f in rs])
            shared=max(0,min(lb[3],rb[3])-max(lb[1],rb[1]))
            if shared<2*h or shared<.35*min(height(lb),height(rb)):continue
            # Prevent fragmented tiny-font rows from masquerading as columns.
            bh=st.median(height(f['bbox']) for f in ls+rs)
            if bh<.7*h:continue
            gutter=[lb[2],rb[0]]
            if gutter[1]<=gutter[0]:continue
            candidates.append({'bbox':envelope([lb,rb]),'cut_x':sum(gutter)/2,'column_boxes':[lb,rb],
              'support_ids':[f['fragment_id'] for f in ls+rs],'evidence':'adaptive_x_y_support',
              'features':{'gutter':gutter,'gutter_fraction':(gutter[1]-gutter[0])/span,'shared_vertical_support':shared,
                          'left_support':len(ls),'right_support':len(rs),'crossing_barriers':len(barriers)},
              'score':len(ls)+len(rs)+min(len(ls),len(rs))})
    return choose_bands(candidates,h)


def choose_bands(candidates,h):
    selected=[]
    for c in sorted(candidates,key=lambda c:(-c['score'],-height(c['bbox']),c['cut_x'])):
        if any(max(0,min(c['bbox'][3],b['bbox'][3])-max(c['bbox'][1],b['bbox'][1]))>.25*min(height(c['bbox']),height(b['bbox'])) for b in selected):continue
        selected.append(c)
    return sorted(selected,key=lambda c:c['bbox'][1])


def paddle_bands(fragments,regions):
    if not fragments:return []
    bb=envelope([f['bbox'] for f in fragments]);h=st.median(height(f['bbox']) for f in fragments);span=width(bb)
    candidates=[]
    for i,a in enumerate(regions):
        ab=a['bbox']
        if not (.23*span<width(ab)<.68*span and height(ab)>3*h):continue
        for b in regions[i+1:]:
            ab,bbx=a['bbox'],b['bbox']
            if not (.23*span<width(bbx)<.68*span and height(bbx)>3*h):continue
            left,right=(a,b) if xc(ab)<xc(bbx) else (b,a);lb,rb=left['bbox'],right['bbox']
            shared=max(0,min(lb[3],rb[3])-max(lb[1],rb[1]))
            if lb[2]>=rb[0] or shared<.45*min(height(lb),height(rb)):continue
            ls=[f for f in fragments if overlap(f['bbox'],lb)>.5*area(f['bbox']) and width(f['bbox'])>.22*span]
            rs=[f for f in fragments if overlap(f['bbox'],rb)>.5*area(f['bbox']) and width(f['bbox'])>.22*span]
            if min(len(ls),len(rs))<3:continue
            candidates.append({'bbox':envelope([lb,rb]),'cut_x':(lb[2]+rb[0])/2,'column_boxes':[lb,rb],
              'support_ids':[f['fragment_id'] for f in ls+rs],'source_region_ids':[left['region_id'],right['region_id']],
              'evidence':'provider_layout_containment','features':{'gutter':[lb[2],rb[0]],'gutter_fraction':(rb[0]-lb[2])/span,
                          'shared_vertical_support':shared,'left_support':len(ls),'right_support':len(rs)},
              'score':len(ls)+len(rs)+min(len(ls),len(rs))})
    return choose_bands(candidates,h)


def assign_columns(fragments,bands):
    if not fragments:return
    full=envelope([f['bbox'] for f in fragments]);h=st.median(height(f['bbox']) for f in fragments)
    for i,b in enumerate(bands):
        b['band_id']=f'band-{i:02d}';b['column_ids']=[f'band-{i:02d}-c0',f'band-{i:02d}-c1']
        if b['features']['gutter_fraction']>.08 and height(b['bbox'])>.55*height(full):
            b['bbox'][1]=full[1];b['bbox'][3]=full[3];b['features']['wide_gutter_full_height_extension']=True
    for f in fragments:
        f['column_id']='single-track';f['band_id']=None
        for b in bands:
            box=f['bbox'];cut=b['cut_x']
            if b['bbox'][1]-.35*h<=yc(box)<=b['bbox'][3]+.35*h and not(box[0]<cut<box[2]):
                side=0 if xc(box)<cut else 1
                f['column_id']=b['column_ids'][side];f['band_id']=b['band_id'];break


def dotted_run(mask,a,b,h):
    y0=max(0,int(min(a[1],b[1])));y1=int(max(a[3],b[3]));x0=int(min(a[0],b[0]));x1=int(max(a[2],b[2]))
    if x1-x0<3*h:return None
    cc=components(mask,[x0,y0,x1,y1]);dots=[c for c in cc if height(c)<.35*h and width(c)<.35*h and c[4]>=2]
    if len(dots)<5:return None
    for seed in dots:
        aligned=sorted([c for c in dots if abs(yc(c)-yc(seed))<.12*h],key=xc)
        for i in range(len(aligned)-4):
            run=aligned[i:i+7]
            xs=[xc(c) for c in run];gaps=np.diff(xs)
            if len(run)<5 or xs[-1]-xs[0]<2*h or np.std(gaps)/max(1,np.mean(gaps))>.35:continue
            return {'kind':'dotted_row_candidate','dot_component_boxes':[c[:4] for c in run],
                    'dot_count':len(run),'y_dispersion':st.pstdev(yc(c) for c in run),'x_span':xs[-1]-xs[0]}
    return None


def group_rows(fragments,mask,bands,page_id):
    if not fragments:return []
    hs=[height(f['bbox']) for f in fragments if width(f['bbox'])>3*height(f['bbox'])]
    bodyh=float(np.quantile(hs,.75)) if hs else 1
    rows=[]
    for f in sorted(fragments,key=lambda f:(yc(f['bbox']),f['bbox'][0],f['fragment_id'])):
        b=f['bbox'];margin=any(h['kind']=='recurring_margin_lane' for h in f['hints'])
        best=None
        if not margin:
            for row in reversed(rows):
                rb=row['bbox']
                if yc(b)-yc(rb)>2*bodyh:break
                if row['column_id']!=f['column_id'] or row['margin']:continue
                ov=max(0,min(b[3],rb[3])-max(b[1],rb[1]))/min(height(b),height(rb))
                if ov<.5 or abs(yc(b)-yc(rb))>.55*min(height(b),height(rb)):continue
                gap=max(0,b[0]-rb[2],rb[0]-b[2])
                dots=dotted_run(mask,rb,b,bodyh)
                near_small=[q for q in fragments if q['fragment_id']!=f['fragment_id'] and .6*bodyh<abs(yc(q['bbox'])-yc(b))<3*bodyh and height(q['bbox'])<.83*bodyh and q['column_id']==f['column_id']]
                dense=height(b)<.83*bodyh and height(rb)<.9*bodyh and len(near_small)>=2
                if gap<=1.2*bodyh or dots or dense:
                    best=row
                    if dots:row['hints'].append(dots)
                    if dense:row['hints'].append({'kind':'small_dense_row_candidate','neighbor_support':len(near_small),'relative_height':height(b)/bodyh})
                    break
        if best:
            best['fragment_ids'].append(f['fragment_id']);best['bbox']=envelope([best['bbox'],b]);best['hints'].append({'kind':'aligned_fragments','vertical_overlap':ov})
        else:rows.append({'bbox':b[:],'fragment_ids':[f['fragment_id']],'column_id':f['column_id'],'band_id':f['band_id'],'margin':margin,'hints':list(f['hints'])})
    def key(r):
        b=next((b for b in bands if b['band_id']==r['band_id']),None)
        if b:return (b['bbox'][1],1,b['column_ids'].index(r['column_id']),r['bbox'][1],r['bbox'][0])
        return (r['bbox'][1],0,0,r['bbox'][1],r['bbox'][0])
    # Margin fragments follow their same-row body fragment, independent of eventual semantic linkage.
    main=sorted([r for r in rows if not r['margin']],key=key)
    for r in sorted([r for r in rows if r['margin']],key=lambda r:yc(r['bbox'])):
        near=[(abs(yc(q['bbox'])-yc(r['bbox'])),i) for i,q in enumerate(main) if not q['margin']]
        if near and min(near)[0]<bodyh:main.insert(min(near)[1]+1,r)
        else:main.append(r)
    for i,r in enumerate(main):
        r['reading_order']=i;r['line_id']=page_id+'-l-'+digest(sorted(r['fragment_ids']))[:16];r.pop('margin')
    return main


def normalize(provider,payload,image,page_id,provenance):
    raw=raw_boxes(provider,payload);gray=np.asarray(image.convert('L'))
    _,mask=cv2.threshold(gray,0,1,cv2.THRESH_BINARY_INV+cv2.THRESH_OTSU)
    regions=[]
    for i,r in enumerate((payload.get('layout') or {}).get('boxes',[])):
        regions.append({'region_id':f'r{i:04d}','bbox':[float(v) for v in r['coordinate']],
                        'source_pointer':f'/provider_raw/layout/boxes/{i}','contains_region_ids':[]})
    for a in regions:
        a['contains_region_ids']=[b['region_id'] for b in regions if a!=b and area(a['bbox'])>area(b['bbox']) and overlap(a['bbox'],b['bbox'])>=.95*area(b['bbox'])]
    originals=[]
    for i,b in enumerate(raw):
        originals.append({'fragment_id':f'd{i:04d}','source_box_index':i,'bbox':b['bbox'],'polygon':b['polygon'],
           'confidence':b['confidence'],'source_pointer':f'/provider_raw/detection/{"dt_polys" if provider=="paddle" else "bboxes"}/{i}',
           'region_ids':[r['region_id'] for r in regions if overlap(b['bbox'],r['bbox'])>=.5*area(b['bbox'])],
           'hints':[],'features':{'width':width(b['bbox']),'height':height(b['bbox']),'aspect':width(b['bbox'])/max(1,height(b['bbox']))}})
    # JSON copy ensures source fragments are immutable when child hints/geometry are added.
    fragments,lanes,events=margin_split(json.loads(json.dumps(originals)),mask,image.width)
    bands=paddle_bands(fragments,regions) if provider=='paddle' else []
    method='provider_layout_containment' if bands else 'adaptive_x_y_support'
    if not bands:bands=adaptive_bands(fragments)
    assign_columns(fragments,bands)
    lines=group_rows(fragments,mask,bands,page_id)
    result={'schema':SCHEMA,'adapter_version':VERSION,'page_id':page_id,'width':image.width,'height':image.height,
            'provider':provider,'provider_raw':payload,'provider_raw_sha256':digest(payload),'source_fragments':originals,
            'fragments':fragments,'layout_regions':regions,'column_bands':bands,'column_method':method,'margin_lanes':lanes,
            'derivations':events,'logical_lines':lines,'provenance':provenance,
            'features':{'binarization':'Otsu, geometry-only ink evidence','source_fragment_count':len(originals)},
            'semantic_status':'low_level_geometric_hints_only'}
    validate(result)
    result['geometry_sha256']=digest(result)
    return result


def validate(d):
    assert d['schema']==SCHEMA and digest(d['provider_raw'])==d['provider_raw_sha256']
    source={f['fragment_id']:f for f in d['source_fragments']};frags={f['fragment_id']:f for f in d['fragments']}
    assert len(frags)==len(d['fragments'])
    seen=[]
    for line in d['logical_lines']:
        assert line['fragment_ids'];seen.extend(line['fragment_ids'])
        assert line['bbox']==envelope([frags[x]['bbox'] for x in line['fragment_ids']])
    assert sorted(seen)==sorted(frags),'every fragment belongs to exactly one logical line'
    assert len({l['line_id'] for l in d['logical_lines']})==len(d['logical_lines'])
    represented=defaultdict(list)
    for f in frags.values():represented[f.get('source_fragment_id',f['fragment_id'])].append(f)
    assert set(represented)==set(source),'no detector box may disappear'
    for sid,parts in represented.items():
        assert envelope([p['bbox'] for p in parts])==source[sid]['bbox']
        assert abs(sum(area(p['bbox']) for p in parts)-area(source[sid]['bbox']))<1e-5
    assert len(d['layout_regions'])==len((d['provider_raw'].get('layout') or {}).get('boxes',[]))
    original=raw_boxes(d['provider'],d['provider_raw'])
    assert len(original)==len(d['source_fragments'])
    for a,b in zip(original,d['source_fragments']):
        assert all(a[k]==b[k] for k in ['bbox','polygon','confidence'])
    banned={'heading_level','role','parent','toc_membership'}
    def check(x):
        if isinstance(x,dict):
            assert not banned.intersection(x)
            for v in x.values():check(v)
        elif isinstance(x,list):
            for v in x:check(v)
    check({k:v for k,v in d.items() if k not in {'provider_raw','provenance'}})
