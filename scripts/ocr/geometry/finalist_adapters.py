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
VERSION = 'r2.2'
SURYA_VERSION = 'r3.2-spatial-support-dev'


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


def clip_polygon_x(polygon,x0,x1):
    result=[list(p) for p in polygon]
    for bound,greater in [(x0,True),(x1,False)]:
        old=result;result=[]
        for a,b in zip(old,old[1:]+old[:1]):
            ia=a[0]>=bound if greater else a[0]<=bound
            ib=b[0]>=bound if greater else b[0]<=bound
            if ia:result.append(a)
            if ia!=ib:
                t=(bound-a[0])/(b[0]-a[0]);result.append([bound,a[1]+t*(b[1]-a[1])])
    return result


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


def margin_split(fragments,mask,page_width,strict_ink=False):
    """Split only when a whitespace-separated end component recurs in a margin lane."""
    if len(fragments)<8:return fragments,[],[]
    hs=[height(f['bbox']) for f in fragments if width(f['bbox'])>3*height(f['bbox'])]
    h=st.median(hs) if hs else 1
    if strict_ink:
        # A repeated field of real dot leaders is a row layout; leading outline
        # numerals must not be split into a spurious margin lane on continuation rows.
        leaders=sum(bool(dotted_run(mask,f['bbox'],f['bbox'],h,reject_glyph_dots=True)) for f in fragments)
        if leaders>=3 and leaders>=.2*len(fragments):return fragments,[],[]
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
        if dotted_run(mask,row_extent,row_extent,h,reject_glyph_dots=strict_ink):continue
        if width(b)<1.6*h and (xc(b)<left or xc(b)>right):
            short.append({'source':f['fragment_id'],'bbox':b,'center':xc(b),'y':yc(b),'side':'left' if xc(b)<left else 'right'})
        if width(b)<.55*(right-left) or height(b)>1.8*h:continue
        cc=components(mask,b)
        if not cc:continue
        for side,edge in [('left',left),('right',right)]:
            if strict_ink:
                nearby=[q for q in wide if q['fragment_id']!=f['fragment_id'] and abs(yc(q['bbox'])-yc(b))<5*h]
                if len(nearby)>=3:
                    local_edge=modal_edge([q['bbox'][0 if side=='left' else 2] for q in nearby],h)
                    # Only a distinct indentation track supersedes the page mode;
                    # a few neighboring merged margin boxes must not move it.
                    if abs(local_edge-edge)>1.5*h:edge=local_edge
            outer=[c for c in cc if (xc(c)>edge+.10*h if side=='right' else xc(c)<edge-.10*h) and c[4]>=max(2,.01*h*h)]
            if not outer:continue
            ob=envelope([c[:4] for c in outer])
            if width(ob)>(2*h if strict_ink else 1.4*h) or height(ob)<.17*h:continue
            inner=[c for c in cc if c not in outer]
            if not inner:continue
            ib=envelope([c[:4] for c in inner])
            gap=ob[0]-ib[2] if side=='right' else ib[0]-ob[2]
            if gap<.18*h or width(ib)<.5*(right-left):continue
            cut=(ob[0]+ib[2])/2 if side=='right' else (ib[0]+ob[2])/2
            proposals.append({'source':f['fragment_id'],'bbox':ob,'center':xc(ob),'y':yc(ob),'side':side,
                              'cut':cut,'valley_width':gap,'ink_component_boxes':[c[:4] for c in outer],
                              **({'local_body_edge':edge} if strict_ink else {})})
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
            nf['source_polygon']=f['polygon']
            nf['polygon']=clip_polygon_x(f['polygon'],box[0],box[2])
            nf['source_geometry_features']=f['features']
            nf['features']={'width':width(box),'height':height(box),'aspect':width(box)/max(1,height(box))}
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


def dotted_run(mask,a,b,h,reject_glyph_dots=False,near_gap=None):
    y0=max(0,int(min(a[1],b[1])));y1=int(max(a[3],b[3]));x0=int(min(a[0],b[0]));x1=int(max(a[2],b[2]))
    if x1-x0<3*h:return None
    cc=components(mask,[x0,y0,x1,y1]);dots=[c for c in cc if height(c)<.35*h and width(c)<.35*h and c[4]>=2]
    minimum=3 if reject_glyph_dots else 5
    if len(dots)<minimum:return None
    for seed in dots:
        aligned=sorted([c for c in dots if abs(yc(c)-yc(seed))<.12*h],key=xc)
        for i in range(len(aligned)-minimum+1):
            run=aligned[i:i+7]
            xs=[xc(c) for c in run];gaps=np.diff(xs)
            if len(run)<minimum or xs[-1]-xs[0]<(0.5*h if reject_glyph_dots else 2*h) or np.std(gaps)/max(1,np.mean(gaps))>.35:continue
            if near_gap and (run[-1][2]<near_gap[0]-2*h or run[0][0]>near_gap[1]+2*h):continue
            # Accents above ordinary letters are not a dotted whitespace leader.
            if reject_glyph_dots and any(height(c)>=.35*h and c[0]<run[-1][2] and c[2]>run[0][0] for c in cc):continue
            return {'kind':'dotted_row_candidate','dot_component_boxes':[c[:4] for c in run],
                    'dot_count':len(run),'y_dispersion':st.pstdev(yc(c) for c in run),'x_span':xs[-1]-xs[0]}
    return None


def split_multiline(fragments,mask):
    """Surya-only: split unusually tall boxes at supported inter-row ink valleys.

    Uses image evidence and page-relative scale only, never reference annotations.
    Children partition the entire source box, including its whitespace padding.
    """
    hs=[height(f['bbox']) for f in fragments if width(f['bbox'])>3*height(f['bbox'])]
    if not hs:return fragments,[]
    h=st.median(hs);output=[];events=[]
    for f in fragments:
        b=f['bbox']
        if 'source_fragment_id' in f or height(b)<=2*h or width(b)<=5*h:
            output.append(f);continue
        x0,y0,x1,y1=map(int,b)
        profile=mask[y0:y1,x0:x1].sum(axis=1).astype(float)
        # Smooth isolated specks; broad, substantial ink runs support each row.
        radius=max(1,round(.04*h));smooth=np.convolve(profile,np.ones(2*radius+1)/(2*radius+1),'same')
        active=smooth>=.35*max(smooth,default=0)
        edges=np.flatnonzero(np.diff(np.r_[False,active,False].astype(int)))
        runs=[(int(a),int(z)) for a,z in zip(edges[::2],edges[1::2]) if z-a>=.15*h]
        cuts=[];evidence=[]
        centers=[(a+z)/2 for a,z in runs]
        if len(runs)>=2 and all(.55*h<=b-a<=1.5*h for a,b in zip(centers,centers[1:])):
            for (begin,end),(start,finish) in zip(runs,runs[1:]):
                valley=end+int(np.argmin(smooth[end:start])) if start>end else end
                local=min(max(smooth[begin:end]),max(smooth[start:finish]))
                if smooth[valley]>.20*local:break
                cuts.append(float(y0+valley));evidence.append({'cut_y':float(y0+valley),'valley_to_peak':float(smooth[valley]/local)})
        if len(cuts)!=len(runs)-1 or not cuts:
            output.append(f);continue
        boundaries=[b[1],*cuts,b[3]];ids=[]
        for i,(top,bottom) in enumerate(zip(boundaries,boundaries[1:])):
            box=[b[0],top,b[2],bottom];fid=f['fragment_id']+f'-y{i}'
            polygon=[[y,x] for x,y in clip_polygon_x([[y,x] for x,y in f['polygon']],top,bottom)]
            child={**f,'fragment_id':fid,'bbox':box,'polygon':polygon,'source_fragment_id':f['fragment_id'],
                   'source_polygon':f['polygon'],'source_geometry_features':f['features'],
                   'features':{'width':width(box),'height':height(box),'aspect':width(box)/height(box)},
                   'hints':list(f['hints'])+[{'kind':'image_supported_row_split','row_index':i}],
                   'derivation':{'operation':'ink_valley_row_split','cut_ys':cuts,'row_index':i}}
            output.append(child);ids.append(fid)
        events.append({'operation':'ink_valley_row_split','source_fragment_id':f['fragment_id'],
                       'output_fragment_ids':ids,'cut_ys':cuts,'page_line_height':h,'valley_evidence':evidence})
    return output,events


def group_rows(fragments,mask,bands,page_id,guard_tall=False):
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
                if guard_tall and max(height(b),height(rb))>2*min(height(b),height(rb)):continue
                ov=max(0,min(b[3],rb[3])-max(b[1],rb[1]))/min(height(b),height(rb))
                if ov<.5 or abs(yc(b)-yc(rb))>.55*min(height(b),height(rb)):continue
                gap=max(0,b[0]-rb[2],rb[0]-b[2])
                # Search all leader runs near the joining gap, not the first
                # ellipsis elsewhere in the row.
                join_gap=((rb[2],b[0]) if rb[2]<b[0] else (b[2],rb[0])) if guard_tall and gap>0 else None
                dots=dotted_run(mask,rb,b,bodyh,reject_glyph_dots=guard_tall,near_gap=join_gap)
                near_small=[q for q in fragments if q['fragment_id']!=f['fragment_id'] and .6*bodyh<abs(yc(q['bbox'])-yc(b))<3*bodyh and height(q['bbox'])<.83*bodyh and q['column_id']==f['column_id']]
                dense=height(b)<.83*bodyh and height(rb)<.9*bodyh and len(near_small)>=2
                # Preserve independently detected narrow units across a real gap.
                # Overlapping apparatus pieces and true dotted TOC rows still group.
                small,large=sorted([b,rb],key=width)
                split_row=any(hint['kind']=='image_supported_row_split' for hint in f['hints']+row['hints'])
                neighbors=[q['bbox'] for q in fragments if q['column_id']==f['column_id'] and .6*bodyh<abs(yc(q['bbox'])-yc(b))<5*bodyh and width(q['bbox'])>8*height(q['bbox'])]
                separate_track=False
                if len(neighbors)>=2:
                    separate_track=(small[2]<st.median(q[0] for q in neighbors)-.1*bodyh if xc(small)<xc(large) else small[0]>st.median(q[2] for q in neighbors)+.1*bodyh)
                # A short first word inside the neighboring body track is not a
                # separate note/margin unit merely because it has a word space.
                if guard_tall and not dots and not split_row and (separate_track or gap>3*bodyh) and gap>=.5*min(height(b),height(rb)) and width(small)<=2*height(small) and width(large)>4*height(small):continue
                evidence_gap=gap
                if split_row and 'pre_refinement_bbox' in f:
                    prior=f['pre_refinement_bbox'];evidence_gap=min(gap,max(0,prior[0]-rb[2],rb[0]-prior[2]))
                if evidence_gap<=1.2*bodyh or dots or dense:
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


def retain_ink_supported(fragments,mask,threshold):
    """Exclude only boxes with zero page-Otsu ink, retaining explicit raw provenance.

    This is a conservative image support gate, not a classifier of verso text.
    Any ink pixel retains a box. No reference, identity or confidence is consulted.
    """
    retained=[];events=[]
    for f in fragments:
        x0,y0,x1,y1=f['bbox']
        x0=max(0,math.floor(x0));y0=max(0,math.floor(y0))
        x1=min(mask.shape[1],math.ceil(x1));y1=min(mask.shape[0],math.ceil(y1))
        crop=mask[y0:y1,x0:x1] if x1>x0 and y1>y0 else mask[:0,:0]
        count=int(crop.sum())
        # An out-of-image/empty box is not evidence of absent ink.
        if crop.size and count==0:
            events.append({'operation':'exclude_no_ink_support','source_fragment_id':f['fragment_id'],
                           'output_fragment_ids':[],'bbox':f['bbox'][:],
                           'evidence':{'method':'page_otsu','threshold':float(threshold),
                                       'ink_pixels':count,'sampled_pixels':int(crop.size),
                                       'pixel_bounds':[x0,y0,x1,y1]}})
        else:retained.append(f)
    return retained,events


def refine_empty_x_edges(fragments,mask,threshold):
    """Trim substantial empty horizontal margins, preserving every ink pixel.

    This repairs bleed-through-expanded boxes and padded split fragments. It does
    not add reference-shaped padding or trim vertical accents/descenders.
    """
    output=[];events=[]
    for f in fragments:
        b=f['bbox'];x0=max(0,math.floor(b[0]));y0=max(0,math.floor(b[1]))
        x1=min(mask.shape[1],math.ceil(b[2]));y1=min(mask.shape[0],math.ceil(b[3]))
        crop=mask[y0:y1,x0:x1];xs=np.flatnonzero(crop.sum(axis=0))
        if not len(xs):output.append(f);continue
        ink_left=float(x0+xs[0]);ink_right=float(x0+xs[-1]+1);h=height(b);pad=max(1.,.05*h)
        left=max(b[0],ink_left-pad) if ink_left-b[0]>=.5*h else b[0]
        right=min(b[2],ink_right+pad) if b[2]-ink_right>=.5*h else b[2]
        if left==b[0] and right==b[2]:output.append(f);continue
        box=[left,b[1],right,b[3]];sid=f.get('source_fragment_id',f['fragment_id']);fid=f['fragment_id']+'-i'
        nf={**f,'fragment_id':fid,'bbox':box,'polygon':clip_polygon_x(f['polygon'],left,right),
            'source_fragment_id':sid,'source_polygon':f.get('source_polygon',f['polygon']),
            'source_geometry_features':f.get('source_geometry_features',f['features']),
            'pre_refinement_bbox':b[:],'pre_refinement_polygon':f['polygon'],
            'features':{'width':width(box),'height':height(box),'aspect':width(box)/height(box)}}
        output.append(nf)
        events.append({'operation':'empty_x_edge_refinement','source_fragment_id':sid,
                       'input_fragment_id':f['fragment_id'],'output_fragment_ids':[fid],
                       'before_bbox':b[:],'after_bbox':box[:],
                       'evidence':{'method':'page_otsu','threshold':float(threshold),
                                   'ink_x_bounds':[ink_left,ink_right],'ink_pixels_preserved':int(crop.sum()),
                                   'removed_ink_pixels':0}})
    return output,events


def normalize(provider,payload,image,page_id,provenance):
    raw=raw_boxes(provider,payload);gray=np.asarray(image.convert('L'))
    threshold,mask=cv2.threshold(gray,0,1,cv2.THRESH_BINARY_INV+cv2.THRESH_OTSU)
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
    fragments=json.loads(json.dumps(originals));exclusions=[]
    if provider=='surya':fragments,exclusions=retain_ink_supported(fragments,mask,threshold)
    fragments,lanes,events=margin_split(fragments,mask,image.width,strict_ink=provider=='surya')
    events=exclusions+events
    if provider=='surya':
        fragments,split_events=split_multiline(fragments,mask);events+=split_events
        fragments,refine_events=refine_empty_x_edges(fragments,mask,threshold);events+=refine_events
    bands=paddle_bands(fragments,regions) if provider=='paddle' else []
    method='provider_layout_containment' if bands else 'adaptive_x_y_support'
    if not bands:bands=adaptive_bands(fragments)
    assign_columns(fragments,bands)
    lines=group_rows(fragments,mask,bands,page_id,guard_tall=provider=='surya')
    result={'schema':SCHEMA,'adapter_version':SURYA_VERSION if provider=='surya' else VERSION,'page_id':page_id,'width':image.width,'height':image.height,
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
    for f in frags.values():
        represented[f.get('source_fragment_id',f['fragment_id'])].append(f)
        if 'source_fragment_id' in f:
            assert f['source_polygon']==source[f['source_fragment_id']]['polygon']
            assert all(f['bbox'][0]-1e-6<=p[0]<=f['bbox'][2]+1e-6 and f['bbox'][1]-1e-6<=p[1]<=f['bbox'][3]+1e-6 for p in f['polygon'])
    excluded={}
    for event in d['derivations']:
        if event['operation']!='exclude_no_ink_support':continue
        sid=event['source_fragment_id']
        assert d['provider']=='surya' and sid in source and sid not in excluded
        assert event['output_fragment_ids']==[] and event['bbox']==source[sid]['bbox']
        assert event['evidence']['method']=='page_otsu'
        assert event['evidence']['ink_pixels']==0 and event['evidence']['sampled_pixels']>0
        excluded[sid]=event
    assert not set(represented).intersection(excluded),'excluded boxes cannot become text geometry'
    assert set(represented)|set(excluded)==set(source),'every detector box needs geometry or exclusion evidence'
    for sid,parts in represented.items():
        boxes=[p.get('pre_refinement_bbox',p['bbox']) for p in parts]
        assert envelope(boxes)==source[sid]['bbox']
        assert abs(sum(area(b) for b in boxes)-area(source[sid]['bbox']))<1e-5
        for p in parts:
            if 'pre_refinement_bbox' not in p:continue
            old=p['pre_refinement_bbox'];b=p['bbox']
            assert old[0]<=b[0]<b[2]<=old[2] and b[1]==old[1] and b[3]==old[3]
            events=[e for e in d['derivations'] if e['operation']=='empty_x_edge_refinement' and e['output_fragment_ids']==[p['fragment_id']]]
            assert len(events)==1 and events[0]['source_fragment_id']==sid
            e=events[0];assert e['before_bbox']==old and e['after_bbox']==b
            assert e['evidence']['removed_ink_pixels']==0 and e['evidence']['ink_pixels_preserved']>0
            assert b[0]<=e['evidence']['ink_x_bounds'][0]<=e['evidence']['ink_x_bounds'][1]<=b[2]
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
