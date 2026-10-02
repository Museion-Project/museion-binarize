"""Reference-free detection -> units -> bounded glyph support. Development candidate."""
import copy, math, statistics as st
import cv2
import numpy as np
import finalist_adapters as a
VERSION='shared-units-v3.2-leader-height'

def rectpoly(b):return [[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]]
def masks(image):
 g=np.asarray(image.convert('L'));t,m=cv2.threshold(g,0,1,cv2.THRESH_BINARY_INV+cv2.THRESH_OTSU)
 # Local contrast corroborates faint glyphs independently of page Otsu.
 bg=cv2.GaussianBlur(g,(0,0),5);local=(bg.astype(float)-g.astype(float)>12).astype('uint8')
 return g,t,m,np.maximum(m,local)
def bounds(b,shape):return [max(0,math.floor(b[0])),max(0,math.floor(b[1])),min(shape[1],math.ceil(b[2])),min(shape[0],math.ceil(b[3]))]

def order_units(lines,bands):
 """Order independent units within visual rows without merging their identities."""
 partitions={}
 for line in lines:partitions.setdefault((line['band_id'],line['column_id']),[]).append(line)
 clusters=[]
 for (bid,col),members in partitions.items():
  rows=[]
  for line in sorted(members,key=lambda l:(a.yc(l['bbox']),l['bbox'][0])):
   b=line['bbox'];row=rows[-1] if rows else None
   if row:
    boxes=[l['bbox'] for l in row];h=st.median(a.height(z) for z in boxes)
    compatible=all(min(z[3],b[3])-max(z[1],b[1])>=.4*min(a.height(z),a.height(b)) and abs(a.yc(z)-a.yc(b))<=.45*max(a.height(z),a.height(b)) for z in boxes)
   else:compatible=False
   if compatible:row.append(line)
   else:rows.append([line])
  for row in rows:
   bb=a.envelope([l['bbox'] for l in row]);band=next((bd for bd in bands if bd['band_id']==bid),None)
   key=(band['bbox'][1],1,band['column_ids'].index(col),bb[1],bb[0]) if band else (bb[1],0,0,bb[1],bb[0])
   clusters.append((key,sorted(row,key=lambda l:l['bbox'][0])))
 ordered=[l for _,row in sorted(clusters,key=lambda x:x[0]) for l in row]
 for i,l in enumerate(ordered):l['reading_order']=i
 return ordered
def split_ends(fs,mask,H):
 out=[];events=[]
 for f in fs:
  b=f['bbox'];h=a.height(b)
  header=b[1]<.15*H
  if a.width(b)<(4 if header else 7)*h or 'source_fragment_id' in f:out.append(f);continue
  cc=sorted(a.components(mask,b),key=lambda c:c[0]);groups=[]
  for c in cc:
   if groups and c[0]-groups[-1][2]<.20*h:groups[-1]=a.envelope([groups[-1],c[:4]])
   else:groups.append(c[:4])
  proposals=[]
  for side in [0,1]:
   if len(groups)<2:continue
   end=groups[0 if side==0 else -1];rest=a.envelope(groups[1:] if side==0 else groups[:-1]);gap=rest[0]-end[2] if side==0 else end[0]-rest[2]
   header=b[1]<.15*H
   near=[q['bbox'] for q in fs if q!=f and .7*h<abs(a.yc(q['bbox'])-a.yc(b))<4*h and a.width(q['bbox'])>7*h]
   # A short word at the ordinary text margin is not evidence of a note number.
   # Continuation rows must instead corroborate the indented body after the prefix.
   typical_left=st.median(q[0] for q in near) if near else None
   indentation=side==0 and b[1]>.55*H and len(near)>=2 and abs(typical_left-rest[0])<.6*h and typical_left-end[0]>h and sum(abs(q[0]-typical_left)<.5*h for q in near)>=2
   inner_gaps=[groups[j+1][0]-groups[j][2] for j in range(len(groups)-1) if j!=(0 if side==0 else len(groups)-2)]
   body_tracks=[q['bbox'] for q in fs if q is not f and q['bbox'][1]>b[3]+h and q['bbox'][1]<.75*H and a.width(q['bbox'])>7*a.height(q['bbox'])]
   # A short title suffix (e.g. a book letter) is not a page number merely
   # because it follows whitespace. Require evidence of an outer text edge.
   outer_edge=bool(body_tracks) and min(abs(end[0]-st.median(q[0] for q in body_tracks)),abs(end[2]-st.median(q[2] for q in body_tracks)))<2*h
   header_gap=header and outer_edge and gap>.5*h and (not inner_gaps or gap>1.4*st.median(inner_gaps))
   if a.width(end)<2.5*h and a.height(end)>.28*h and ((header_gap) or (indentation and gap>.5*h)):
    proposals.append((gap,side,(end[2]+rest[0])/2 if side==0 else (rest[2]+end[0])/2,header,indentation))
  if not proposals:out.append(f);continue
  gap,side,cut,header,indent=max(proposals);ids=[]
  for n,box in enumerate([[b[0],b[1],cut,b[3]],[cut,b[1],b[2],b[3]]]):
   child=copy.deepcopy(f);child.update(fragment_id=f['fragment_id']+f'-e{n}',bbox=box,polygon=a.clip_polygon_x(f['polygon'],box[0],box[2]),source_fragment_id=f['fragment_id'],source_polygon=f['polygon'])
   if n==side:child['hints'].append({'kind':'independent_end_unit','header_zone':header,'indentation_support':indent})
   out.append(child);ids.append(child['fragment_id'])
  events.append({'operation':'image_supported_end_split','source_fragment_id':f['fragment_id'],'output_fragment_ids':ids,'cut_x':cut,'gap':gap,'header_zone':header,'indentation_support':indent})
 return out,events

def organize(fs,mask,bands,pid):
 if not fs:return []
 hs=[a.height(f['bbox']) for f in fs if a.width(f['bbox'])>3*a.height(f['bbox'])];bodyh=st.median(hs) if hs else 1
 rows=[]
 for f in sorted(fs,key=lambda f:(a.yc(f['bbox']),f['bbox'][0])):
  b=f['bbox'];independent=any(h['kind'] in ['recurring_margin_lane','independent_end_unit'] for h in f['hints']);best=None
  if not independent:
   for row in reversed(rows):
    rb=row['bbox'];h=min(a.height(b),a.height(rb))
    if a.yc(b)-a.yc(rb)>2*bodyh:break
    if row['independent'] or row['column_id']!=f['column_id']:continue
    if abs(a.yc(b)-a.yc(rb))>.42*h:continue
    gap=max(0,b[0]-rb[2],rb[0]-b[2]);dots=a.dotted_run(mask,rb,b,bodyh,True,near_gap=(min(rb[2],b[2]),max(rb[0],b[0])))
    # A leader and a contained trailing token can share a baseline despite
    # different detector box heights. Keep every independent/column guard.
    left,right=sorted([b,rb],key=lambda z:z[0])
    leader_height_support=bool(dots) and left[2]<=right[0] and a.width(left)>4*h and a.width(right)<=2.5*a.height(right) and min(b[3],rb[3])-max(b[1],rb[1])>=.8*h
    if max(a.height(b),a.height(rb))>1.6*h and not leader_height_support:continue
    small,large=sorted([b,rb],key=a.width)
    if not dots and a.width(small)<=2*a.height(small) and a.width(large)>4*h and gap>.35*h:continue
    union=a.envelope([rb,b])
    if any(union[0]<bd['cut_x']<union[2] and bd['bbox'][1]<=a.yc(union)<=bd['bbox'][3] for bd in bands):continue
    supports=[q['bbox'] for q in fs if q['column_id']==f['column_id'] and .6*bodyh<abs(a.yc(q['bbox'])-a.yc(b))<4*bodyh and a.width(q['bbox'])>4*bodyh]
    supported=len(supports)>=2 and union[0]>=min(q[0] for q in supports)-h and union[2]<=max(q[2] for q in supports)+h
    if gap<=1.2*h or dots or (supported and gap<=3.5*h):
     # Whole-group baseline and height check, not just the last pair.
     members=[q for q in fs if q['fragment_id'] in row['fragment_ids']]+[f]
     if max(a.yc(q['bbox']) for q in members)-min(a.yc(q['bbox']) for q in members)>.42*min(a.height(q['bbox']) for q in members):continue
     best=row;row['hints'].append({'kind':'joint_row_evidence','gap':gap,'same_region_support':len(supports),'dotted':bool(dots),'whole_group_baseline_checked':True});break
  if best:best['fragment_ids'].append(f['fragment_id']);best['bbox']=a.envelope([best['bbox'],b])
  else:rows.append({'bbox':b[:],'fragment_ids':[f['fragment_id']],'column_id':f['column_id'],'band_id':f['band_id'],'hints':f['hints'][:],'independent':independent})
 def key(r):
  band=next((b for b in bands if b['band_id']==r['band_id']),None)
  return (band['bbox'][1],1,band['column_ids'].index(r['column_id']),r['bbox'][1],r['bbox'][0]) if band else (r['bbox'][1],0,0,r['bbox'][1],r['bbox'][0])
 # Keep marginalia after the nearest body row, while independent headers sort geometrically.
 main=sorted([r for r in rows if not any(h['kind']=='recurring_margin_lane' for h in r['hints'])],key=key)
 for r in [r for r in rows if any(h['kind']=='recurring_margin_lane' for h in r['hints'])]:
  nearest=min(((abs(a.yc(q['bbox'])-a.yc(r['bbox'])),i) for i,q in enumerate(main)),default=(1e9,0))
  main.insert(nearest[1]+1,r)
 byid={f['fragment_id']:f for f in fs}
 for i,r in enumerate(main):
  r['fragment_ids'].sort(key=lambda fid:byid[fid]['bbox'][0])
  r.pop('independent');r.update(reading_order=i,line_id=pid+'-l-'+a.digest(sorted(r['fragment_ids']))[:16])
 return order_units(main,bands)

def crops(lines,mask,bands):
 records=[]
 for l in lines:
  b=l['bbox'];h=a.height(b);limit=[max(0,b[0]-.4*h),max(0,b[1]-.25*h),min(mask.shape[1],b[2]+.4*h),min(mask.shape[0],b[3]+.25*h)]
  for q in lines:
   if q is l:continue
   z=q['bbox']
   if min(b[2],z[2])>max(b[0],z[0]):
    if a.yc(z)<a.yc(b)-.4*min(h,a.height(z)):limit[1]=max(limit[1],min(b[1],(z[3]+b[1])/2))
    if a.yc(z)>a.yc(b)+.4*min(h,a.height(z)):limit[3]=min(limit[3],max(b[3],(z[1]+b[3])/2))
   if min(b[3],z[3])>max(b[1],z[1]):
    if z[2]<=b[0]:limit[0]=max(limit[0],(z[2]+b[0])/2)
    if z[0]>=b[2]:limit[2]=min(limit[2],(z[0]+b[2])/2)
  for bd in bands:
   if bd['bbox'][1]<=a.yc(b)<=bd['bbox'][3]:
    if b[2]<=bd['cut_x']:limit[2]=min(limit[2],bd['cut_x'])
    if b[0]>=bd['cut_x']:limit[0]=max(limit[0],bd['cut_x'])
  selected=[]
  for c in a.components(mask,limit):
   cb=c[:4];inter=a.overlap(cb,b)
   accent=(cb[2]>b[0] and cb[0]<b[2] and max(0,b[1]-cb[3],cb[1]-b[3])<.12*h and a.height(cb)<.35*h)
   if inter>0 or accent:selected.append(cb)
  extended=a.envelope([b,*selected]) if selected else b[:]
  records.append({'line_id':l['line_id'],'unit_bbox':b[:],'glyph_support_bbox':extended,'transcription_crop_bbox':extended,'component_support':selected,'search_limit':limit,'used_by':'development crop materializer only; production freeze unchanged'})
 return records

def normalize(provider,payload,image,page_id,provenance,mode='shared'):
 gray,t,otsu,mask=masks(image);originals=[]
 for i,b in enumerate(a.raw_boxes(provider,payload)):
  originals.append({'fragment_id':f'd{i:04d}','source_box_index':i,**b,'source_pointer':f'/provider_raw/detection/{"dt_polys" if provider=="paddle" else "bboxes"}/{i}','region_ids':[],'hints':[],'features':{'width':a.width(b['bbox']),'height':a.height(b['bbox']),'aspect':a.width(b['bbox'])/max(1,a.height(b['bbox']))}})
 fs=copy.deepcopy(originals);events=[];lanes=[]
 if mode=='shared':
  kept=[]
  for f in fs:
   x0,y0,x1,y1=bounds(f['bbox'],gray.shape);crop=mask[y0:y1,x0:x1]
   global_pixels=int(otsu[y0:y1,x0:x1].sum());h=a.height(f['bbox'])
   local_components=a.components(mask,f['bbox']) if crop.size and crop.any() and not global_pixels else []
   spatial_support=any(a.height(c[:4])>=max(2,.15*h) and c[4]>=max(2,.002*h*h) for c in local_components)
   if crop.size and not global_pixels and not spatial_support:events.append({'operation':'exclude_no_joint_ink_support','source_fragment_id':f['fragment_id'],'output_fragment_ids':[],'bbox':f['bbox'],'evidence':{'otsu_threshold':t,'local_contrast_threshold':12,'global_pixels':global_pixels,'joint_pixels':int(crop.sum()),'local_components':local_components,'minimum_local_component_height':max(2,.15*h),'minimum_local_component_area':max(2,.002*h*h),'reason':'Neither global foreground nor a local-contrast component with glyph-scale spatial support'}})
   else:kept.append(f)
  fs=kept
  fs,lanes,e=a.margin_split(fs,mask,image.width,strict_ink=True);events+=e
  fs,e=a.split_multiline(fs,mask);events+=e
  fs,e=split_ends(fs,mask,image.height);events+=e
  # Trimming uses union of global and local evidence, never Otsu alone.
  fs,e=a.refine_empty_x_edges(fs,mask,t)
  for event in e:event['evidence'].update(method='union_page_otsu_local_contrast',local_contrast_threshold=12)
  events+=e
 bands=a.adaptive_bands(fs);a.assign_columns(fs,bands)
 if mode=='shared':lines=organize(fs,mask,bands,page_id)
 else:
  # Reuse column ordering but forbid all pair grouping with one-at-a-time geometry.
  lines=[{'bbox':f['bbox'][:],'fragment_ids':[f['fragment_id']],'column_id':f['column_id'],'band_id':f['band_id'],'hints':[]} for f in fs]
  def key(l):
   b=next((b for b in bands if b['band_id']==l['band_id']),None)
   return (b['bbox'][1],1,b['column_ids'].index(l['column_id']),l['bbox'][1],l['bbox'][0]) if b else (l['bbox'][1],0,0,l['bbox'][1],l['bbox'][0])
  lines.sort(key=key)
  for i,l in enumerate(lines):l.update(reading_order=i,line_id=page_id+'-l-'+a.digest(l['fragment_ids'])[:16])
  lines=order_units(lines,bands)
 doc={'schema':a.SCHEMA,'adapter_version':VERSION+'-'+mode,'page_id':page_id,'width':image.width,'height':image.height,'provider':provider,'provider_raw':payload,'provider_raw_sha256':a.digest(payload),'source_fragments':originals,'fragments':fs,'layout_regions':[],'column_bands':bands,'column_method':'adaptive_x_y_support','margin_lanes':lanes,'derivations':events,'logical_lines':lines,'provenance':provenance,'features':{'source_fragment_count':len(originals)},'semantic_status':'low_level_geometric_hints_only'}
 doc['geometry_sha256']=a.digest(doc)
 side={'version':VERSION,'crops':crops(lines,mask,bands) if mode=='shared' else [{'line_id':l['line_id'],'unit_bbox':l['bbox'],'glyph_support_bbox':l['bbox'],'transcription_crop_bbox':l['bbox']} for l in lines]}
 validate(doc)
 return doc,side

def validate(d):
 assert a.digest(d['provider_raw'])==d['provider_raw_sha256']
 assert [{k:f[k] for k in ['bbox','polygon','confidence']} for f in d['source_fragments']]==a.raw_boxes(d['provider'],d['provider_raw'])
 fs={f['fragment_id']:f for f in d['fragments']};seen=[]
 for l in d['logical_lines']:
  seen+=l['fragment_ids'];assert l['bbox']==a.envelope([fs[i]['bbox'] for i in l['fragment_ids']])
 assert sorted(seen)==sorted(fs) and len(seen)==len(fs)
 represented={f.get('source_fragment_id',f['fragment_id']) for f in fs.values()};excluded={e['source_fragment_id'] for e in d['derivations'] if e['operation']=='exclude_no_joint_ink_support'}
 assert not represented&excluded
 assert represented|excluded=={f['fragment_id'] for f in d['source_fragments']}
