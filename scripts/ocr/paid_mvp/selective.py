"""Offline selective contract and exact image manifests; live sender stays locked."""
import hashlib,json,math,io
from pathlib import Path
from .pipeline import canonical,check,digest,image_url,MISTRAL,LUNA,MODELS,BoundaryError
from .repair_units import VERSION,verify

def mistral_manifest(task):
 check(task.get('region_protocol')==VERSION,'selective-v3 required')
 from .pipeline import preflight
 old=preflight(task);calls=[c for c in old['calls'] if c['destination']==MISTRAL]
 for c in calls:
  body=dict(model=c['model'],document=dict(type='image_url',image_url=image_url(c['image_path'])),include_blocks=True,confidence_scores_granularity='word',include_image_base64=False)
  c['body_sha256']=hashlib.sha256(canonical(body)).hexdigest()
  c['request_id']=hashlib.sha256(canonical({k:c[k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
 manifest=dict(schema_version=3,protocol=VERSION,task=task,calls=calls,request_cap=len(calls),worst_case_reservation_usd=sum(c['reserve_usd'] for c in calls),mistral_free_only=True,no_retries=True,network_sent=False,luna_authorization='after_actual_base_and_crop_freeze')
 manifest['seal']=hashlib.sha256(canonical(manifest)).hexdigest();return manifest

def request_shape(page,units,image_path,crop_directory):
 from PIL import Image
 verify(units,page);check(len(units)<=12,'page unit budget')
 content=[dict(type='input_text',text='Return exact protocol/batch binding and patches. keep/unknown omit text. replace only printed complete target line; no neighboring text, reconstruction, translation or grammar repair.')];audit=[]
 root=Path(crop_directory);root.mkdir(parents=True,exist_ok=True)
 with Image.open(image_path) as image:
  for u in units:
   check(digest(image_path)==u['image_sha256'],'source image changed')
   b=u['bbox'];check(all(type(v) in (int,float) for v in b) and 0<=b[0]<b[2]<=image.width and 0<=b[1]<b[3]<=image.height,'crop bounds')
   bounds=[math.floor(b[0]),math.floor(b[1]),math.ceil(b[2]),math.ceil(b[3])]
   check(bounds!=[0,0,image.width,image.height] and (bounds[2]-bounds[0])*(bounds[3]-bounds[1])<=image.width*image.height*.25,'full-page or excessive crop prohibited')
   path=root/(u['id']+'.png');buffer=io.BytesIO();image.crop(bounds).save(buffer,format='PNG');crop=buffer.getvalue()
   check(not path.is_symlink(),'crop symlink prohibited')
   if path.exists():check(path.read_bytes()==crop,'existing crop differs; never overwrite frozen artifact')
   else:
    with path.open('xb') as file:file.write(crop)
   audit.append(dict(unit=u,crop_path=str(path.resolve()),crop_sha256=digest(path),crop_bounds=bounds,context=[]))
   content.extend([dict(type='input_text',text=json.dumps(u,ensure_ascii=False)),dict(type='input_image',image_url=image_url(path),detail='high')])
 binding=hashlib.sha256(canonical(audit)).hexdigest();content[0]['text']+='\n'+json.dumps(dict(protocol=VERSION,batch_sha256=binding))
 body=dict(model=MODELS[LUNA],store=False,reasoning=dict(effort='low'),max_output_tokens=4096,input=[dict(role='user',content=content)])
 return dict(protocol=VERSION,batch_sha256=binding,units=units,audit=audit,body=body,body_sha256=hashlib.sha256(canonical(body)).hexdigest(),destination=LUNA,network_sent=False)

def validate(payload,shape):
 check(type(payload) is dict and set(payload)=={'protocol','batch_sha256','patches'},'strict batch schema')
 check(payload['protocol']==VERSION and payload['batch_sha256']==shape['batch_sha256'],'batch binding')
 wanted={u['id']:u for u in shape['units']};seen=set()
 for p in payload['patches']:
  check(type(p) is dict and p.get('id') in wanted and p['id'] not in seen,'unit ownership/duplicate');seen.add(p['id']);u=wanted[p['id']]
  fields={'id','decision','unit_sha256'}|({'text'} if p.get('decision')=='replace' else set())
  check(set(p)==fields and p['decision'] in ['keep','unknown','replace'],'patch schema; keep/unknown text forbidden')
  check(p['unit_sha256']==hashlib.sha256(canonical(u)).hexdigest(),'source/epoch/member/offset binding')
  if p['decision']=='replace':check(u['locator']=='LOCATED' and type(p['text']) is str and '\n' not in p['text'] and '\x00' not in p['text'],'replacement line boundary')
 check(seen==set(wanted),'missing unit; reject entire batch');return payload

def live_blocked(*args,**kwargs):raise BoundaryError('selective live requires shared durable ledger and exact post-base crop approval; legacy whole-page sending disabled')

def ledger_directory(root=None):
 from .state import directory
 return directory(root)

def durable_request(call,body,config,approval,transport=None,*,state_root=None,semantic_validation_pending=False):
 """One ledger across operation IDs and App restarts; uncertainty never re-sends."""
 from .pipeline import request
 check(call['body_sha256']==hashlib.sha256(canonical(body)).hexdigest(),'body outside exact manifest')
 identity=hashlib.sha256(canonical({k:call[k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
 check(call['request_id']==identity,'durable content identity mismatch')
 ledger=ledger_directory(state_root)
 if transport is None:check(approval.get('state_root')==str(ledger),'exact approved persistent state root required')
 ledger.mkdir(mode=0o700,parents=True,exist_ok=True)
 # Global stop check and reservation share pipeline.request's BEGIN IMMEDIATE.
 return request(call,body,ledger,config,approval,transport,global_stop=True,defer_completion=call['destination']==MISTRAL or semantic_validation_pending)

def base_run(manifest,approval,config,transport=None,*,state_root=None):
 from .pipeline import authorize,write,read,observations
 from .repair_units import build,prove_lines
 from .selector import select
 check(manifest.get('protocol')==VERSION and all(c['destination']==MISTRAL for c in manifest['calls']),'Mistral-only stage')
 state_root=ledger_directory(state_root)
 authorize(manifest,approval);task=manifest['task'];check(not state_root.is_relative_to(Path(task['output_directory']).resolve()),'operation directory cannot own shared state');check(all(digest(Path(__file__).parent/name)==sha for name,sha in manifest.get('code_freeze',{}).items()),'frozen runtime code changed');fresh=mistral_manifest(task);check(all(manifest[k]==fresh[k] for k in ['task','calls','request_cap','worst_case_reservation_usd','protocol']),'changed source or manifest')
 out=Path(task['output_directory']);out.mkdir(parents=True,exist_ok=True)
 if (out/'result.json').exists():
  existing=read(out/'result.json');check(read(out/'task.json')==task,'operation ownership');return existing
 if (out/'task.json').exists():check(read(out/'task.json')==task,'operation ownership')
 else:write(out/'task.json',task,True)
 pages=[];stopped=False
 for call in manifest['calls']:
  number=call['page_number']
  if stopped or (out/'cancel').exists():pages.append(dict(page_number=number,status='cancelled' if (out/'cancel').exists() else 'not_sent'));continue
  try:
   body=dict(model=call['model'],document=dict(type='image_url',image_url=image_url(call['image_path'])),include_blocks=True,confidence_scores_granularity='word',include_image_base64=False)
   receipt=durable_request(call,body,config,approval,transport,state_root=state_root)
   check(receipt['http_status']==200 and receipt['list_price_estimate_usd'] is not None,'Mistral failure/usage unknown; stop')
   check(digest(call['image_path'])==call['image_sha256'],'source image changed during request')
   from PIL import Image
   with Image.open(call['image_path']) as source_image:dimensions=list(source_image.size)
   page=observations(receipt['response'],call['image_sha256'],number,expected_model=call['model'],expected_index=0,expected_dimensions=dimensions);prove_lines(page,call['image_path'])
   units=build(page,task['input_sha256'],receipt['response_sha256'],task['config_version']);selection=select(units)
   page['repair_units']=units;page['selection']=selection
   if page['response_binding']['state']!='VERIFIED':
    from .state import halt
    halt(state_root,call,receipt,page['response_binding']['failures']);stopped=True
   else:
    from .state import accept
    accept(state_root,call)
   pages.append(dict(page_number=number,status='cancelled' if (out/'cancel').exists() else 'awaiting_crop_authorization' if selection['selected'] else 'EXPORT_REVIEW',observation=page,base_receipt=receipt))
  except (BoundaryError,ValueError,KeyError,TypeError) as error:
   failure=dict(page_number=number,status='failed',reason=str(error))
   if 'receipt' in locals():
    failure['base_receipt']=receipt
    failure['raw_markdown_pages']=[dict(index=p.get('index'),markdown=p.get('markdown')) for p in receipt['response'].get('pages',[]) if isinstance(p,dict)] if isinstance(receipt['response'],dict) and isinstance(receipt['response'].get('pages'),list) else []
    if receipt['http_status']==200 and receipt['list_price_estimate_usd'] is not None:
     from .state import halt
     halt(state_root,call,receipt,str(error))
   pages.append(failure);stopped=True
  finally:
   if 'receipt' in locals():del receipt
 write(out/'pages-v0.json',pages,True)
 from .review_ui import render
 result=dict(schema_version=3,protocol=VERSION,operation_id=task['operation_id'],mode='paid',status='failed' if stopped else 'cancelled' if (out/'cancel').exists() else 'review_required',page_results=pages,output_directory=str(out.resolve()),evidence=dict(execution='synthetic_transport' if transport else 'fresh_Mistral_selective_base',quality_verified=False,human_reviewed=False,live_verified=False),review_required=True,artifacts=dict(pages_json=str((out/'pages-v0.json').resolve()),review_html=render(out,task,pages)),network_sent=transport is None,quality_verified=False,live_verified=False,ready=False)
 write(out/'result.json',result);return result

def repair_manifest(shape,task,page_number):
 check(shape['units'] and len(shape['units'])<=12,'no empty/over-budget Luna call')
 # Transport audits every crop through the sealed body. Primary image also satisfies legacy request guard.
 first=shape['audit'][0]
 call=dict(page_number=page_number,image_path=first['crop_path'],image_sha256=first['crop_sha256'],destination=LUNA,model=MODELS[LUNA],body_sha256=shape['body_sha256'],reserve_usd=.05,max_output_tokens=4096,full_original_image=False,crops=shape['audit'])
 call['request_id']=hashlib.sha256(canonical({k:call[k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
 manifest=dict(schema_version=3,protocol=VERSION,task=task,calls=[call],shape=shape,request_cap=1,worst_case_reservation_usd=.05,mistral_free_only=True,no_retries=True,network_sent=False)
 manifest['seal']=hashlib.sha256(canonical(manifest)).hexdigest();return manifest

def repair_run(manifest,approval,config,transport=None,*,state_root=None):
 from .pipeline import authorize,read,write
 state_root=ledger_directory(state_root)
 authorize(manifest,approval);shape=manifest['shape'];task=manifest['task'];out=Path(task['output_directory']);check(not state_root.is_relative_to(out.resolve()),'operation directory cannot own shared state');pages=read(out/'pages-v0.json')
 approved={(a['image_sha256'],a['destination'],a['model']) for a in approval['approved_images']}
 check(all((a['crop_sha256'],LUNA,MODELS[LUNA]) in approved for a in shape['audit']),'each actual crop requires exact image/destination authorization')
 check(digest(task['input_pdf'])==task['input_sha256'],'source changed')
 page=next(p for p in pages if p['page_number']==manifest['calls'][0]['page_number'])
 check(shape['units']==page['observation']['selection']['selected'],'selection changed; prior crop approval invalid')
 if transport is None:check(page['observation'].get('strict_response_binding') and page['observation']['response_binding']['state']=='VERIFIED','unverified historical base cannot dispatch')
 verify(shape['units'],page['observation'])
 for a in shape['audit']:check(digest(a['crop_path'])==a['crop_sha256'],'actual crop changed')
 check(not (out/'cancel').exists(),'cancelled before repair send')
 call=manifest['calls'][0]
 receipt=durable_request(call,shape['body'],config,approval,transport,state_root=state_root,semantic_validation_pending=True)
 check(receipt['http_status']==200 and receipt['list_price_estimate_usd'] is not None,'Luna failure/usage unknown; stop')
 try:
  response=receipt['response'];check(type(response) is dict,'Luna response schema')
  check(response.get('model')==MODELS[LUNA] and response.get('status')=='completed','Luna model/status')
  texts=[c['text'] for o in response.get('output',[]) if o.get('type')=='message' for c in o.get('content',[]) if c.get('type')=='output_text']
  payload=validate(json.loads(''.join(texts)),shape)
  result=dict(protocol=VERSION,patches=payload,shape=shape,shape_sha256=hashlib.sha256(canonical(shape)).hexdigest(),receipt=receipt,status='cancelled' if (out/'cancel').exists() else 'source_review_required',human_reviewed=False)
  write(out/('patches-'+shape['batch_sha256']+'.json'),result,True)
 except (BoundaryError,ValueError,KeyError,TypeError,AttributeError) as error:
  from .state import halt
  reason=str(error) if isinstance(error,BoundaryError) else 'Luna response/patch schema invalid: '+type(error).__name__
  halt(state_root,call,receipt,reason)
  raise BoundaryError(reason) from error
 # Known HTTP/usage is insufficient: only a bound, validated immutable patch
 # can release the received row and permit a later different request identity.
 from .state import accept
 accept(state_root,call)
 return result


def review_context(directory,pages):
 """Validated immutable batch proposals projected to existing consumer members."""
 import copy
 out=Path(directory);result=copy.deepcopy(pages);bindings=[]
 for path in sorted(out.glob('patches-*.json')):
  from .pipeline import read
  stored=read(path);check(stored['status']=='source_review_required','cancelled patch cannot adopt')
  shape=stored['shape'];check(hashlib.sha256(canonical(shape)).hexdigest()==stored['shape_sha256'],'patch shape changed')
  validate(stored['patches'],shape);bindings.append(dict(path=path.name,sha256=digest(path)))
  for patch in stored['patches']['patches']:
   if patch['decision']!='replace':continue
   unit=next(u for u in shape['units'] if u['id']==patch['id'])
   page=next(p for p in result if p['page_number']==unit['page_number']);check(page['observation']['text_sha256']==unit['base_sha256'],'base identity changed')
   current=next(u for u in page['observation']['repair_units'] if u['id']==unit['id']);check(current==unit,'source/member/epoch/geometry changed')
   region=next(r for r in page['observation']['regions'] if r['member_ids']==unit['member_ids'] and r['original_start']==unit['raw_start'] and r['original_end']==unit['raw_end']);check(region['locator']=='LOCATED','unknown member')
   page.setdefault('overlay',{'regions':[]})['regions'].append(dict(id=region['id'],decision='replace',text=patch['text'],unit_id=unit['id']))
 identity=hashlib.sha256(canonical(dict(pages=pages,patches=bindings))).hexdigest()
 return result,identity
