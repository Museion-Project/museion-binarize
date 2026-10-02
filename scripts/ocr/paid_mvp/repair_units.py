"""Conservative source-bound whole-line units. Paragraph boxes are never line proof."""
import hashlib
import copy
import re
from .pipeline import canonical, check
VERSION='selective-v3'
def build(page, source_hash, response_hash, epoch):
 units=[]
 for r in page['regions']:
  proof=r.get('line_proof',{})
  located=r['locator']=='LOCATED' and '\n' not in r['original_text'] and proof.get('complete_line') is True and proof.get('source_image_sha256')==r['source_image_sha256'] and proof.get('bbox')==r['bbox']
  binding=dict(protocol=VERSION,source_sha256=source_hash,page_number=page['page_number'],image_sha256=r['source_image_sha256'],response_sha256=response_hash,base_sha256=page['text_sha256'],epoch=epoch,member_ids=r['member_ids'],raw_start=r['original_start'],raw_end=r['original_end'],bbox=r['bbox'],order=len(units),line_proof=proof,response_binding=r.get('response_binding'),confidence_evidence=r.get('confidence_evidence'))
  units.append(dict(**binding,id=hashlib.sha256(canonical(binding)).hexdigest(),original_text=r['original_text'],locator='LOCATED' if located else 'UNKNOWN',reason=None if located else 'complete_line_source_proof_missing',confidence=r.get('confidence')))
 return units

def verify(units, page):
 previous=-1;boxes=[]
 for u in units:
  check(u['locator']=='LOCATED','UNKNOWN unit cannot dispatch')
  check(hashlib.sha256(page['text'].encode()).hexdigest()==page['text_sha256'],'raw base hash changed')
  current=build(page,u['source_sha256'],u['response_sha256'],u['epoch'])
  check(any(candidate==u for candidate in current),'unit proof or binding changed')
  if page.get('strict_response_binding'):check(page['response_binding']['state']=='VERIFIED','response identity unverified')
  a,b=u['raw_start'],u['raw_end'];check(type(a) is int and type(b) is int and previous<=a<b<=len(page['text']),'raw offset/order')
  check(page['text'][a:b]==u['original_text'] and page['text'].count(u['original_text'])==1,'ambiguous raw anchor')
  check(u['base_sha256']==page['text_sha256'],'base changed');previous=b
  box=u['bbox'];check(box is not None,'missing geometry')
  for other in boxes:
   check(min(box[2],other[2])<=max(box[0],other[0]) or min(box[3],other[3])<=max(box[1],other[1]),'overlap ownership')
  boxes.append(box)

def prove_lines(page,image_path):
 """Conservative one occupied ink band and full-width whitespace ownership.
 Disconnected accents may remain UNKNOWN; this is not recognition correctness.
 """
 import math
 from PIL import Image
 from .pipeline import digest
 with Image.open(image_path) as image:
  w,h=image.size;pixels=image.convert('L');image_hash=digest(image_path)
  # A provider paragraph is not one repair line. Derive line bounds only when
  # literal raw line breaks and separate source ink bands agree one-to-one.
  # This proves ownership/geometry, never that the recognizer's text is correct.
  page.setdefault('raw_regions',copy.deepcopy(page['regions']))
  derived=[]
  for parent in page['regions']:
   text=parent['original_text']
   if '\n' not in text and '\r' not in text:
    derived.append(parent);continue
   if (parent['locator']!='LOCATED' or parent['source_image_sha256']!=image_hash
       or parent['dimensions']!=[w,h] or not parent.get('bbox')):
    derived.append(parent);continue
   x0,y0=map(math.floor,parent['bbox'][:2]);x1,y1=map(math.ceil,parent['bbox'][2:])
   if not (1<=x0<x1<w-1 and 3<=y0<y1<h-3):derived.append(parent);continue
   spans=list(re.finditer(r'[^\r\n]+',text))
   rows=[y for y in range(y0,y1) if pixels.crop((x0,y,x1,y+1)).getextrema()[0]<180]
   bands=[]
   for y in rows:
    if not bands or y>bands[-1][1]:bands.append([y,y+1])
    else:bands[-1][1]=y+1
   safe=(len(spans)==len(bands)>1 and all(m.group().strip()==m.group() and m.group() for m in spans)
         and bands[0][0]>=y0+2 and bands[-1][1]<=y1-2
         and all(b[0]-a[1]>=5 for a,b in zip(bands,bands[1:])))
   if not safe:
    parent['line_partition_reason']='literal_lines_and_source_bands_not_uniquely_aligned'
    derived.append(parent);continue
   from .confidence import adapt
   for ordinal,(span,band) in enumerate(zip(spans,bands)):
    start=parent['original_start']+span.start();end=parent['original_start']+span.end()
    box=[parent['bbox'][0],band[0]-2,parent['bbox'][2],band[1]+2]
    binding=dict(parent_region_id=parent['id'],source_image_sha256=image_hash,raw_start=start,raw_end=end,bbox=box)
    member=hashlib.sha256(canonical(binding)).hexdigest()
    confidence=adapt(page['raw_response']['pages'][0],{},start,end)
    unique=page['text'][start:end]==span.group() and page['text'].count(span.group())==1
    derived.append(dict(parent,id=member,member_ids=[member],parent_member_ids=list(parent['member_ids']),
                        parent_region_id=parent['id'],original_text=span.group(),original_start=start,original_end=end,
                        bbox=box,geometry_precision='source_ink_band_candidate',locator='LOCATED' if unique else 'UNKNOWN',
                        reason=None if unique else 'ambiguous_complete_line_anchor',
                        confidence=confidence['value'],confidence_evidence=confidence,
                        line_partition=dict(method='literal-lines-source-ink-bands-v1',ordinal=ordinal,
                                            parent_bbox=parent['bbox'],parent_raw_offset=[parent['original_start'],parent['original_end']],
                                            source_ink_bands=bands,recognition_correctness_verified=False)))
  page['regions']=derived
  for r in page['regions']:
   r.pop('line_proof',None) # Re-validation cannot retain an obsolete proof.
   r['line_proof_reason']='complete_line_source_proof_missing'
   if r['source_image_sha256']!=image_hash:
    r['line_proof_reason']='source_image_changed';continue
   if r['locator']!='LOCATED' or '\n' in r['original_text'] or '\r' in r['original_text'] or r['dimensions']!=[w,h]:continue
   x0,y0=map(math.floor,r['bbox'][:2]);x1,y1=map(math.ceil,r['bbox'][2:])
   if not (1<=x0<x1<w-1 and 1<=y0<y1<h-1 and y1-y0<=h*.08):continue
   def ink(rect):return pixels.crop(rect).getextrema()[0]<180
   rows=[y for y in range(y0,y1) if ink((x0,y,x1,y+1))]
   if not rows:continue
   bands=[]
   for y in rows:
    if not bands or y>bands[-1][1]:bands.append([y,y+1])
    else:bands[-1][1]=y+1
   if len(bands)!=1:
    r['line_proof_reason']='multiple_or_disconnected_ink_bands';continue
   if any(ink(rect) for rect in [(0,y0,x0,y1),(x1,y0,w,y1),(0,y0-1,w,y0+1),(0,y1-1,w,y1+1)]):
    r['line_proof_reason']='neighbor_or_clipped_ink';continue
   r['line_proof_reason']=None
   r['line_proof']=dict(method='pixel-whitespace-single-ink-band-v2',complete_line=True,source_image_sha256=image_hash,bbox=r['bbox'],pixel_bounds=[x0,y0,x1,y1],ink_bands=bands,geometry_precision='whole_line',recognition_evidence='unique_exact_current_mistral_raw_anchor',recognition_correctness_verified=False)
 return page
