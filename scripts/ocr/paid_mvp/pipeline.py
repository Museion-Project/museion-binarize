"""Mistral observations -> strict Luna region proposals -> versioned review -> PDF.
Cloud execution requires an exact preflight seal and coordinator reservation.
"""
import base64, hashlib, html, json, math, os, re, sqlite3, time, urllib.error, urllib.request
from pathlib import Path

MISTRAL='https://api.mistral.ai/v1/ocr'
LUNA='https://api.openai.com/v1/responses'
MODELS={MISTRAL:'mistral-ocr-4-1',LUNA:'gpt-6-luna'}
class BoundaryError(ValueError): pass

def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def canonical(x):return json.dumps(x,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()
def read(path):return json.loads(Path(path).read_text())
def write(path,x,exclusive=False):
 p=Path(path);p.parent.mkdir(parents=True,exist_ok=True)
 with p.open('x' if exclusive else 'w') as f:json.dump(x,f,ensure_ascii=False,indent=2);f.write('\n')
def check(condition,message):
 if not condition:raise BoundaryError(message)
def locked(directory):
 c=sqlite3.connect(Path(directory)/'state.sqlite',timeout=20);c.execute('PRAGMA journal_mode=WAL');c.execute('CREATE TABLE IF NOT EXISTS calls(id TEXT PRIMARY KEY,state TEXT,reserve REAL,cost REAL,receipt TEXT)');c.execute('CREATE TABLE IF NOT EXISTS reviews(revision INTEGER PRIMARY KEY,data TEXT)');return c

def readiness():
 font='/System/Library/Fonts/Supplemental/Arial Unicode.ttf';runtime=Path(font).exists() and __import__('importlib.util',fromlist=['find_spec']).find_spec('fitz') is not None
 return dict(schema_version=1,mode='paid',component_ready=True,local_runtime_ready=runtime,app_ready=False,distribution_ready=False,quality_ready=False,live_verified=False,ready=False,blockers=[dict(code='LIVE_QUALITY_PENDING',message='Selective-v3 component only; fresh Mistral/crop-Luna/source-review chain and independent six-page quality unverified',evidence='intent.md §8.68 NEXT2–NEXT4')],dependencies=[dict(name='PyMuPDF',available=__import__('importlib.util',fromlist=['find_spec']).find_spec('fitz') is not None),dict(name='Unicode font',path=font,available=Path(font).exists())],evidence=[])

def preflight(task):
 check(task.get('region_protocol','v1') in ['v1','v2-minimal-keep-unknown','selective-v3'],'unsupported region protocol');check(task.get('mode')=='paid','explicit paid mode required');check(task.get('operation_id') and task.get('config_version'),'identity/config required')
 check(digest(task['input_pdf'])==task['input_sha256'],'source hash mismatch')
 check(len(task['page_numbers'])==len(task['images']) and len(set(task['page_numbers']))==len(task['page_numbers']),'page/image scope mismatch')
 import fitz
 with fitz.open(task['input_pdf']) as pdf:
  check(all(type(n) is int and 1<=n<=len(pdf) for n in task['page_numbers']),'page range')
 calls=[]
 for n,img in zip(task['page_numbers'],task['images']):
  check(digest(img['path'])==img['sha256'],'image hash mismatch')
  from PIL import Image
  with Image.open(img['path']) as image:
   check(image.width*image.height<=16000000,'image exceeds reservation pixel bound')
   if task.get('input_kind')=='explicit_image_only_scan_fixture':
    with fitz.open(task['input_pdf']) as source:
     sp=source[n-1];embedded=sp.get_images();check(len(embedded)==1 and not sp.get_text().strip(),'scan input must contain one image and no text');pix=fitz.Pixmap(source,embedded[0][0]);check(pix.n==3 and (pix.width,pix.height)==image.size and pix.samples==image.convert('RGB').tobytes(),'scan input image pixels mismatch');rects=sp.get_image_rects(embedded[0][0]);check(len(rects)==1 and all(abs(a-b)<.001 for a,b in zip(rects[0],sp.rect)),'scan image must occupy full page')
  check(img.get('source_pdf_sha256',task['input_sha256'])==task['input_sha256'] and img.get('source_page_number',n)==n,'image/source binding mismatch')
  for endpoint,reserve in [(MISTRAL,.004),(LUNA,.05)]:
   identity=hashlib.sha256(canonical(dict(source=task['input_sha256'],page=n,image=img['sha256'],endpoint=endpoint,model=MODELS[endpoint],config=task['config_version'],operation=task['operation_id']))).hexdigest()
   calls.append(dict(request_id=identity,page_number=n,image_path=str(Path(img['path']).resolve()),image_sha256=img['sha256'],destination=endpoint,model=MODELS[endpoint],reserve_usd=reserve,max_output_tokens=12000 if endpoint==LUNA else None,full_original_image=True))
 manifest=dict(schema_version=1,task=task,calls=calls,request_cap=len(calls),worst_case_reservation_usd=round(sum(x['reserve_usd'] for x in calls),8),mistral_free_only=True,no_retries=True,network_sent=False)
 manifest['seal']=hashlib.sha256(canonical(manifest)).hexdigest();return manifest

def authorize(manifest,approval):
 unsigned={k:v for k,v in manifest.items() if k!='seal'}
 check(hashlib.sha256(canonical(unsigned)).hexdigest()==manifest['seal'],'manifest seal mismatch')
 check(approval.get('manifest_seal')==manifest['seal'] and approval.get('combined_reservation_reconciled') is True,'coordinator reservation required')
 check(set(approval.get('approved_request_ids',[]))=={c['request_id'] for c in manifest['calls']},'exact request identities required')
 check(approval.get('mistral_free_only') is True,'free-only constraint')
 check(approval.get('historical_unsettled_usd') is not None,'historical unknown reservations required')
 check(all(type(approval.get(k)) in (int,float) and math.isfinite(approval[k]) and approval[k]>=0 for k in ['historical_spent_usd','historical_unsettled_usd','combined_new_reserve_usd','cap_usd']),'invalid budget amount')
 check(approval['historical_spent_usd']+approval['historical_unsettled_usd']+approval['combined_new_reserve_usd']<=approval['cap_usd']<=1,'joint historical cap exceeded')
 approved={(x['image_sha256'],x['destination'],x['model']) for x in approval['approved_images']}
 check(all((c['image_sha256'],c['destination'],c['model']) in approved for c in manifest['calls']),'image/destination outside authority')
 check(manifest['worst_case_reservation_usd']<=approval['combined_new_reserve_usd'],'reservation too small')

def credential(endpoint,config):
 name='MISTRAL_API_KEY' if endpoint==MISTRAL else 'OPENAI_API_KEY'
 # Existing protected configuration only; never return keys in logs or exceptions.
 for line in Path(config).read_text().splitlines():
  m=re.match(r'^\s*(?:export\s+)?'+name+r'\s*=\s*(.*?)\s*$',line)
  if m:return m[1].strip('\"\'')
 raise BoundaryError('credential unavailable: '+name)

def request(call,body,directory,config,approval,transport=None,global_stop=False,defer_completion=False):
 check(transport is not None or call.get('body_sha256')==hashlib.sha256(canonical(body)).hexdigest(), 'legacy live sender disabled; exact selective serialized body required')
 check(sum(len(c.get('text','').encode()) for i in body.get('input',[]) for c in i.get('content',[]))<=100000,'prompt exceeds conservative input bound');check(body.get('model')==call['model'],'explicit model mismatch');check(digest(call['image_path'])==call['image_sha256'],'upload image changed')
 directory=Path(directory);directory.mkdir(parents=True,exist_ok=True);db=locked(directory)
 try:
  db.execute('BEGIN IMMEDIATE')
  existing=db.execute('SELECT state,receipt FROM calls WHERE id=?',(call['request_id'],)).fetchone()
  if existing:
   db.rollback();check(existing[0] in ('complete','received'),'prior/inflight request cannot be resent');saved=read(existing[1]);check(digest(saved['raw_path'])==saved['response_sha256'],'immutable raw response changed');check(canonical(read(Path(saved['raw_path']).parent/'request.json'))==canonical(body),'request changed under durable identity');return saved
  if global_stop:
   blocked=db.execute("SELECT id FROM calls WHERE state!='complete' OR cost IS NULL LIMIT 1").fetchone()
   check(blocked is None,'global prior inflight/failure/unknown usage stop; no subsequent send')
  db.execute('INSERT INTO calls VALUES(?,?,?,?,?)',(call['request_id'],'reserved',call['reserve_usd'],None,None));db.commit()
  dest=directory/'raw'/call['request_id'];dest.mkdir(parents=True,exist_ok=False);write(dest/'request.json',body,True)
  write(dest/'attempt.json',dict(request_id=call['request_id'],request_sha256=digest(dest/'request.json'),started=time.time(),reserve_usd=call['reserve_usd'],retry=False),True)
  # Identity is durable before the send; uncertain outcomes retain their entire reservation.
  start=time.monotonic();status=None;raw=b'';error=None;transport_evidence=None
  try:
   if transport:status,raw=transport(call['destination'],body)
   else:
    req=urllib.request.Request(call['destination'],data=canonical(body),headers={'Authorization':'Bearer '+credential(call['destination'],config),'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=150) as r:status=r.status;raw=r.read()
  except urllib.error.HTTPError as e:status=e.code;raw=e.read();error='HTTP_'+str(status)
  except Exception as e:
   error=type(e).__name__
   if isinstance(e,urllib.error.URLError):
    reason=e.reason;transport_evidence=dict(reason_type=type(reason).__name__,errno=getattr(reason,'errno',None),sanitized_reason=str(reason)[:500],http_response_observed=False,network_delivery_proven=False)
  (dest/'response.raw').write_bytes(raw)
  try:response=json.loads(raw)
  except Exception:response={}
  usage=(response.get('usage') if call['destination']==LUNA else response.get('usage_info')) if isinstance(response,dict) else None;cost=None
  if call['destination']==MISTRAL and isinstance(usage,dict) and type(usage.get('pages_processed')) is int and usage['pages_processed']==1:cost=.004
  if call['destination']==LUNA and isinstance(usage,dict) and all(type(usage.get(k)) is int and usage[k]>=0 for k in ['input_tokens','output_tokens']):
   cached=usage.get('input_tokens_details',{}).get('cached_tokens',0)
   if type(cached) is int and 0<=cached<=usage['input_tokens']:cost=((usage['input_tokens']-cached)*.1+cached*.01+usage['output_tokens']*.5)/1e6
  receipt=dict(http_status=status,error=error,transport_evidence=transport_evidence,seconds=time.monotonic()-start,usage=usage,list_price_estimate_usd=cost,unsettled_reserve_usd=call['reserve_usd'] if cost is None else 0,account_debit_verified=False,response=response,raw_path=str((dest/'response.raw').resolve()),response_sha256=digest(dest/'response.raw'))
  write(dest/'receipt.json',receipt,True);db.execute('UPDATE calls SET state=?,cost=?,receipt=? WHERE id=?',('received' if status==200 and defer_completion else 'complete' if status==200 else 'failed',cost,str((dest/'receipt.json').resolve()),call['request_id']));db.commit();return receipt
 finally:db.close()

def observations(response,image_hash,page_number,*,expected_model=None,expected_index=None,expected_dimensions=None):
 import copy
 from .confidence import adapt
 check(isinstance(response,dict) and isinstance(response.get('pages'),list) and len(response['pages'])==1,'one image page required')
 p=response['pages'][0];check(isinstance(p,dict),'page schema');text=p.get('markdown');check(type(text) is str,'markdown required');d=p.get('dimensions');d=d if isinstance(d,dict) else {};w=d.get('width');h=d.get('height');geometry_known=type(w) in (int,float) and type(h) in (int,float) and math.isfinite(w) and math.isfinite(h) and w>0 and h>0
 if not geometry_known:w=h=0
 strict=expected_model is not None or expected_index is not None or expected_dimensions is not None
 failures=[]
 if strict and any(v is None for v in (expected_model,expected_index,expected_dimensions)):failures.append('identity_expectations_incomplete')
 if expected_model is not None and response.get('model')!=expected_model:failures.append('model_mismatch')
 if expected_index is not None and (type(p.get('index')) is not int or p['index']!=expected_index):failures.append('page_index_mismatch')
 if expected_dimensions is not None and [w,h]!=list(expected_dimensions):failures.append('image_dimensions_mismatch')
 binding=dict(state='VERIFIED' if strict and not failures else 'UNKNOWN',expected_model=expected_model,actual_model=response.get('model'),expected_index=expected_index,actual_index=p.get('index'),expected_dimensions=expected_dimensions,actual_dimensions=[w,h],failures=failures,live_verified=False)
 regions=[];cursor=0
 check(p.get('blocks') is None or type(p.get('blocks')) is list,'blocks schema');p=dict(p);p['blocks']=p.get('blocks') or []
 for i,b in enumerate(p['blocks']):
  if not isinstance(b,dict):continue
  content=b.get('content')
  if type(content) is not str:content='' # Non-text blocks remain raw, never fabricate an anchor.
  bbox=[b.get(k) for k in ['top_left_x','top_left_y','bottom_right_x','bottom_right_y']]
  good=all(type(v) in (int,float) and math.isfinite(v) for v in bbox)
  good=good and 0<=bbox[0]<bbox[2]<=w and 0<=bbox[1]<bbox[3]<=h
  # Only exact unique text anchors are locatable. Repeated or missing blocks stay UNKNOWN.
  start=text.find(content,cursor) if content else -1
  loc=good and start>=0 and text.count(content)==1 and not failures
  if loc:cursor=start+len(content)
  confidence=adapt(p,b,start if loc else None,start+len(content) if loc else None)
  regions.append(dict(id=hashlib.sha256(canonical([image_hash,page_number,i,b])).hexdigest(),member_ids=[str(i)],page_number=page_number,source_image_sha256=image_hash,original_text=content,original_start=start if loc else None,original_end=start+len(content) if loc else None,bbox=bbox if good else None,locator='LOCATED' if loc else 'UNKNOWN',reason=None if loc else 'ambiguous_text_anchor_or_geometry',dimensions=[w,h],confidence=confidence['value'],confidence_evidence=confidence,raw_block=copy.deepcopy(b),geometry_precision='paragraph',response_binding=copy.deepcopy(binding)))
 for i,r in enumerate(regions):
  if r['locator']!='LOCATED':continue
  for other in regions[:i]:
   if other['bbox'] is None:continue
   a=r['bbox'];b=other['bbox'];area=max(0,min(a[2],b[2])-max(a[0],b[0]))*max(0,min(a[3],b[3])-max(a[1],b[1]));small=min((a[2]-a[0])*(a[3]-a[1]),(b[2]-b[0])*(b[3]-b[1]))
   if area>small*.15:r['locator']='UNKNOWN';r['reason']='overlapping_region_ownership';other['locator']='UNKNOWN';other['reason']='overlapping_region_ownership'
 covered=set(i for r in regions if r['locator']=='LOCATED' for i in range(r['original_start'],r['original_end']));uncovered=sum(not c.isspace() and i not in covered for i,c in enumerate(text))
 return dict(raw_response=copy.deepcopy(response),strict_response_binding=strict,response_binding=binding,page_number=page_number,text=text,text_sha256=hashlib.sha256(text.encode()).hexdigest(),regions=regions,uncovered_nonspace_characters=uncovered,uncovered_text_requires_review=uncovered>0,source_page_completeness_verified=False)

def validate_overlay(response,page):
 check(response.get('model')=='gpt-6-luna' and response.get('status')=='completed','Luna model/status')
 texts=[c['text'] for o in response.get('output',[]) if o.get('type')=='message' for c in o.get('content',[]) if c.get('type')=='output_text']
 try:x=json.loads(''.join(texts))
 except Exception:raise BoundaryError('strict JSON required')
 check(set(x)=={'regions'} and type(x['regions']) is list,'overlay schema')
 wanted={r['id']:r for r in page['regions']};seen=set()
 for r in x['regions']:
  check(type(r) is dict and set(r)=={'id','decision','text'},'region schema');check(r['id'] in wanted and r['id'] not in seen,'region ownership/duplicate');seen.add(r['id'])
  check(r['decision'] in ['keep','unknown','replace'] and type(r['text']) is str,'decision/text')
  check(r['decision']=='replace' or r['text']==wanted[r['id']]['original_text'],'keep/unknown changed original')
  check(r['decision']!='replace' or wanted[r['id']]['locator']=='LOCATED','UNKNOWN cannot replace')
 check(seen==set(wanted),'missing regions');return x

def image_url(path):return 'data:image/png;base64,'+base64.b64encode(Path(path).read_bytes()).decode()

def run(manifest,approval,config,transport=None):
 check(transport is not None, 'legacy all-region/full-image live disabled by intent §8.68; historical replay only')
 authorize(manifest,approval);task=manifest['task'];preflight(task);out=Path(task['output_directory']);out.mkdir(parents=True,exist_ok=True)
 taskfile=out/'task.json'
 if taskfile.exists():check(read(taskfile)==task,'operation directory belongs to another task')
 else:write(taskfile,task,True)
 if (out/'result.json').exists():
  result=read(out/'result.json');result['evidence']['reload_existing_operation']=True;return result
 pages=[];timings={};stopped=False
 for number,img in zip(task['page_numbers'],task['images']):
  if (out/'cancel').exists():break
  if stopped:pages.append(dict(page_number=number,status='not_sent',reason='prior failure stop'));continue
  calls=[c for c in manifest['calls'] if c['page_number']==number]
  mc=next(c for c in calls if c['destination']==MISTRAL);lc=next(c for c in calls if c['destination']==LUNA)
  try:
   mr=request(mc,dict(model=mc['model'],document=dict(type='image_url',image_url=image_url(img['path'])),include_blocks=True,confidence_scores_granularity='word',include_image_base64=False),out,config,approval,transport)
   check(mr['http_status']==200,'Mistral request failed');check(transport is not None or mr['list_price_estimate_usd'] is not None,'Mistral usage unknown; stop');page=observations(mr['response'],img['sha256'],number)
   if (out/'cancel').exists():pages.append(dict(page_number=number,status='cancelled',observation=page));break
   prompt='Review only each supplied region within its original-page pixel bbox. Preserve printed Greek diacritics, Latin, numbers, sigla, editorial symbols and punctuation. No grammar repair, reconstruction, modernization or translation. Return strict JSON {"regions":[{"id":string,"decision":"keep"|"replace"|"unknown","text":string}]}. Exactly one item per input region. keep/unknown text must exactly equal original_text. UNKNOWN locator permits only keep/unknown. The full original image provides context; never add neighboring text.\n'+json.dumps(page['regions'],ensure_ascii=False)
   if task.get('region_protocol')=='v2-minimal-keep-unknown':
    prompt=prompt.replace('Return strict JSON {"regions":[{"id":string,"decision":"keep"|"replace"|"unknown","text":string}]}. Exactly one item per input region. keep/unknown text must exactly equal original_text.', 'Return strict JSON {"regions":[{"id":string,"decision":"keep"|"unknown"} OR {"id":string,"decision":"replace","text":string}]}. Exactly one item per input region. keep/unknown must omit text entirely; backend preserves immutable originals. Extra fields reject whole response.')
   lr=request(lc,dict(model=lc['model'],reasoning=dict(effort='low'),max_output_tokens=12000,store=False,input=[dict(role='user',content=[dict(type='input_text',text=prompt),dict(type='input_image',image_url=image_url(img['path']),detail='high')])]),out,config,approval,transport)
   check(lr['http_status']==200,'Luna request failed');check(lr['list_price_estimate_usd'] is None or lr['list_price_estimate_usd']<=lc['reserve_usd'],'usage exceeded reservation; stop');check(transport is not None or lr['list_price_estimate_usd'] is not None,'Luna usage unknown; stop');from .protocol import validate_v2
   overlay=validate_v2(lr['response'],page) if task.get('region_protocol')=='v2-minimal-keep-unknown' else validate_overlay(lr['response'],page)
   pages.append(dict(page_number=number,status='review_required',observation=page,overlay=overlay));timings[str(number)]=dict(mistral_seconds=mr['seconds'],luna_seconds=lr['seconds'])
  except (BoundaryError,TypeError,KeyError,ValueError) as e:pages.append(dict(page_number=number,status='failed',reason=str(e) if isinstance(e,BoundaryError) else type(e).__name__,**({'observation':page} if 'page' in locals() and page.get('page_number')==number else {})));stopped=True
 # Preserve every denominator page, including cancelled/not attempted.
 present={p['page_number'] for p in pages}
 pages.extend(dict(page_number=n,status='cancelled' if (out/'cancel').exists() else 'not_sent') for n in task['page_numbers'] if n not in present)
 revision_path=out/'pages-v0.json'
 if revision_path.exists():check(read(revision_path)==pages,'immutable observation revision differs')
 else:write(revision_path,pages,True)
 db=locked(out);usage=[dict(request_id=a,state=b,reserve_usd=c,cost_estimate_usd=d) for a,b,c,d in db.execute('SELECT id,state,reserve,cost FROM calls')];db.close()
 from .review_ui import render
 review_html=Path(render(out,task,pages))
 result=dict(schema_version=1,operation_id=task['operation_id'],mode='paid',status='cancelled' if (out/'cancel').exists() else ('failed' if stopped else 'review_required'),input_sha256=task['input_sha256'],output_directory=str(out.resolve()),artifacts=dict(pages_json=str(revision_path.resolve()),review_html=str(review_html.resolve()),raw_directory=str((out/'raw').resolve())),page_results=pages,review_required=True,timings=timings,usage=usage,evidence=dict(execution='synthetic_transport' if transport else 'fresh_live',quality_verified=False,human_reviewed=False));write(out/'result.json',result);return result

def review(directory,expected_revision,source_hash,actions):
 out=Path(directory);task=read(out/'task.json');check(source_hash==task['input_sha256']==digest(task['input_pdf']),'review source hash mismatch')
 check(all(digest(i['path'])==i['sha256'] for i in task['images']),'review image identity changed');pages=read(out/'pages-v0.json');from .selective import review_context
 pages,review_identity=review_context(out,pages);regions={r['id']:r for p in pages for r in p.get('observation',{}).get('regions',[])}
 db=locked(out)
 try:
  db.execute('BEGIN IMMEDIATE');row=db.execute('SELECT revision,data FROM reviews ORDER BY revision DESC LIMIT 1').fetchone();current=row[0] if row else 0;check(current==expected_revision,'stale review revision');prior=json.loads(row[1]) if row else {};check(not row or prior.get('pages_sha256')==review_identity,'review identity changed');adopted=prior.get('adopted',{});seen=set()
  proposals={r['id']:r for p in pages for r in p.get('overlay',{}).get('regions',[])}
  for a in actions:
   if task.get('region_protocol')=='selective-v3':
    check(a.get('review_identity')==review_identity and a.get('source_image_loaded') is True,'source review identity/loading proof required')
   rid=a['region_id'];check(rid in regions and rid not in seen,'ownership/duplicate action');seen.add(rid);r=regions[rid]
   check(task.get('region_protocol')!='selective-v3' or a.get('source_image_sha256')==r['source_image_sha256'],'review image binding');check(a.get('member_ids')==r['member_ids'],'member ownership mismatch');check(a['action'] in ['accept','reject','change'],'review action')
   if a['action']=='reject':adopted.pop(rid,None);continue
   check(r['locator']=='LOCATED','UNKNOWN amendment must remain pending')
   if task.get('region_protocol')=='selective-v3':
    units=[u for p in pages for u in p.get('observation',{}).get('repair_units',[]) if u['page_number']==r['page_number'] and u['member_ids']==r['member_ids'] and u['raw_start']==r['original_start'] and u['raw_end']==r['original_end']]
    check(len(units)==1 and units[0]['locator']=='LOCATED','complete line UNKNOWN cannot be manually amended')
   if a['action']=='accept':check(rid in proposals and proposals[rid]['decision']=='replace','accept requires replace proposal');text=proposals[rid]['text']
   else:text=a.get('text')
   check(type(text) is str and bool(a.get('source_evidence')),'source-supported review evidence required');adopted[rid]=dict(text=text,source_evidence=a['source_evidence'],reviewer=a.get('reviewer','unspecified'),human_approved=a.get('human_approved') is True)
  receipt=dict(revision=current+1,source_hash=source_hash,pages_sha256=review_identity,adopted=adopted,actions=actions,time=time.time());db.execute('INSERT INTO reviews VALUES(?,?)',(current+1,json.dumps(receipt,ensure_ascii=False)));db.commit();write(out/f'review-v{current+1}.json',receipt,True)
  from .review_ui import render
  receipt['review_html']=render(out,task,pages,receipt)
  if (out/'result.json').exists():
   completion=read(out/'result.json');completion['review_revision']=current+1;completion['artifacts']['review_html']=receipt['review_html'];write(out/'result.json',completion)
  return receipt
 finally:db.close()

def export_pdf(directory,expected_revision):
 import fitz
 out=Path(directory);task=read(out/'task.json');check(digest(task['input_pdf'])==task['input_sha256'],'export source changed');db=locked(out)
 try:
  db.execute('BEGIN IMMEDIATE');row=db.execute('SELECT revision,data FROM reviews ORDER BY revision DESC LIMIT 1').fetchone();check(row and row[0]==expected_revision,'export stale/no review revision');receipt=json.loads(row[1])
  check(all(digest(i['path'])==i['sha256'] for i in task['images']),'review image identity changed');pages=read(out/'pages-v0.json');from .selective import review_context
  pages,review_identity=review_context(out,pages);pdf=fitz.open(task['input_pdf']);texts=[];placements=[];coverage=[];snapshot=review_identity;check(receipt.get('pages_sha256',snapshot)==snapshot,'review identity changed')
  for p in pages:
   if 'observation' not in p:
    texts.append('');coverage.append(dict(page_number=p['page_number'],status='EXPORT_REVIEW',reason=p.get('status')));continue
   page=pdf[p['page_number']-1];check(not page.get_text().strip(),'existing native layer requires explicit scan-input preparation; refuse duplicate OCR')
   from . import pdf_unicode
   font=fitz.Font(fontfile=task.get('font_path','/System/Library/Fonts/Supplemental/Arial Unicode.ttf'));fontname='paidunicode';characters=''.join(receipt['adopted'].get(r['id'],{}).get('text',r['original_text']) for r in p['observation']['regions'] if r['locator']=='LOCATED');pdf_unicode.install(pdf,page,font,fontname,characters)
   original=p['observation']['text'];changes=[]
   for r in p['observation']['regions']:
    if r['locator']!='LOCATED':continue
    text=receipt['adopted'].get(r['id'],{}).get('text',r['original_text']);w,h=r['dimensions'];b=r['bbox'];rect=fitz.Rect(b[0]*page.rect.width/w,b[1]*page.rect.height/h,b[2]*page.rect.width/w,b[3]*page.rect.height/h)
    # Paragraph bbox is honest coarse geometry. Unicode text is inserted once per line.
    lines=text.splitlines() or [''];step=rect.height/max(1,len(lines));size=max(.4,min(9,step*.7))
    for i,line in enumerate(lines):
     check('\x00' not in line and all(font.has_glyph(ord(c)) for c in line if not c.isspace()),'unsupported Unicode glyph')
     width=font.text_length(line,fontsize=size)
     if width>rect.width and width:size*=rect.width/width
     pdf_unicode.append(pdf,page,fontname,size,rect.x0,rect.y0+(i+.8)*step,line)
    placements.append(dict(region_id=r['id'],page_number=p['page_number'],bbox_points=list(rect),text=text,geometry_precision='paragraph_coarse'))
    if r['id'] in receipt['adopted']:changes.append((r['original_start'],r['original_end'],text))
   for a,b,t in sorted(changes,reverse=True):original=original[:a]+t+original[b:]
   texts.append(original);coverage.append(dict(page_number=p['page_number'],status='EXPORT_REVIEW' if p['observation']['uncovered_nonspace_characters'] else 'LOCATED_PROJECTION',uncovered_nonspace_characters=p['observation']['uncovered_nonspace_characters'],full_raw_text_preserved=True))
  dest=out/f'searchable-v{expected_revision}.pdf';attempt=0
  while dest.exists():
   attempt+=1;dest=out/f'searchable-v{expected_revision}-attempt{attempt}.pdf'
  pdf.save(dest);pdf.close()
  with fitz.open(task['input_pdf']) as source,fitz.open(dest) as exported:
   check(len(source)==len(exported),'page count changed')
   for a,b in zip(source,exported):check(a.rect==b.rect and a.get_pixmap(matrix=fitz.Matrix(.5,.5)).samples==b.get_pixmap(matrix=fitz.Matrix(.5,.5)).samples,'visible PDF altered')
   consumer=[]
   for p in placements:
    extracted=exported[p['page_number']-1].get_text();ok=all(line.strip() in extracted for line in p['text'].splitlines() if line.strip());consumer.append(dict(region_id=p['region_id'],unicode_roundtrip=ok));check(ok,'PDF Unicode roundtrip failed')
  textfile=out/f'text-v{expected_revision}.txt';textfile.write_text('\n\f\n'.join(texts));report=dict(searchable_pdf=str(dest.resolve()),text=str(textfile.resolve()),revision=expected_revision,placements=placements,consumer=consumer,visible_pixels_unchanged=True,visible_verification_dpi=36,source_input_preserved=True,geometry_precision='paragraph_coarse',complete_text_coverage=False,export_status='EXPORT_REVIEW',coverage=coverage,app_nfc_search_verified=False,human_reviewed=all(a['human_approved'] for a in receipt['adopted'].values()) and bool(receipt['adopted']))
  write(out/f'export-v{expected_revision}.json',report,True)
  if (out/'result.json').exists():
   completion=read(out/'result.json');completion['artifacts'].update(searchable_pdf=report['searchable_pdf'],text=report['text']);completion['review_revision']=expected_revision;completion['evidence']['pdf_consumer']=report;write(out/f'completion-v{expected_revision}.json',completion,True);write(out/'result.json',completion)
  db.commit();return report
 finally:db.close()
