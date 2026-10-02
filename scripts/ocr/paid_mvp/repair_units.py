"""Conservative source-bound whole-line units. Paragraph boxes are never line proof."""
import hashlib
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
