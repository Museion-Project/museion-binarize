import copy,hashlib,json,tempfile,unittest
from pathlib import Path
from PIL import Image,ImageDraw
from .pipeline import observations,BoundaryError,canonical,digest,run
from .repair_units import build,prove_lines,verify,VERSION
from .selector import select
from .selective import request_shape,validate,durable_request

class SelectiveTest(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.image=self.root/'page.png'
  # Keep the literal square/Unicode fixture. Noto Sans lacks that glyph; using
  # an OS fallback hid the dependency. Pin the test font to our PyMuPDF build's
  # embedded CJK font, materialized only in this test's temporary directory.
  import fitz
  fixture_font=fitz.Font('cjk');self.assertTrue(fixture_font.has_glyph(ord('□')))
  font_path=self.root/'fixture-font.ttf';font_path.write_bytes(fixture_font.buffer);self.font=str(font_path)
  image=Image.new('RGB',(200,400),'white');ImageDraw.Draw(image).rectangle((25,25,90,34),fill='black');image.save(self.image)
  self.page=observations(dict(pages=[dict(markdown='□ bad\nunlocated tail',dimensions=dict(width=200,height=400),blocks=[dict(content='□ bad',top_left_x=20,top_left_y=20,bottom_right_x=100,bottom_right_y=40)])]),digest(self.image),1)
  prove_lines(self.page,self.image);self.units=build(self.page,'source','response','epoch1');self.shape=request_shape(self.page,self.units,self.image,self.root/'crops')
 def tearDown(self):self.tmp.cleanup()
 def payload(self,decision='keep'):
  u=self.units[0];return dict(protocol=VERSION,batch_sha256=self.shape['batch_sha256'],patches=[dict(id=u['id'],decision=decision,unit_sha256=hashlib.sha256(canonical(u)).hexdigest(),**({'text':'good'} if decision=='replace' else {}))])
 def test_whole_line_pixel_proof(self):self.assertEqual(self.units[0]['locator'],'LOCATED')
 def test_paragraph_is_unknown(self):
  self.page['regions'][0].pop('line_proof');self.assertEqual(build(self.page,'s','r','e')[0]['locator'],'UNKNOWN')
 def test_selector_base_only(self):self.assertEqual(len(select(self.units)['selected']),1)
 def test_confidence_missing_not_high(self):self.assertEqual(select(self.units)['confidence_missing'],1)
 def test_budget_does_not_topk(self):self.assertEqual(select(self.units*13)['status'],'budget-blocked');self.assertFalse(select(self.units*13)['selected'])
 def test_actual_serialized_crop(self):
  content=self.shape['body']['input'][0]['content'];self.assertEqual(len([c for c in content if c['type']=='input_image']),1);self.assertNotIn(digest(self.image),[a['crop_sha256'] for a in self.shape['audit']]);self.assertEqual(self.shape['audit'][0]['crop_bounds'],[20,20,100,40])
 def test_keep_unknown_no_text(self):
  for decision in ['keep','unknown']:
   p=self.payload(decision);validate(p,self.shape);p['patches'][0]['text']=''
   with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_replace_protocol(self):validate(self.payload('replace'),self.shape)
 def test_wrong_epoch(self):
  p=self.payload();p['patches'][0]['unit_sha256']='changed'
  with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_duplicate_batch_rejected(self):
  p=self.payload();p['patches']*=2
  with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_missing_batch_rejected(self):
  p=self.payload();p['patches']=[]
  with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_foreign_source(self):
  p=self.payload();p['batch_sha256']='wrong'
  with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_raw_offsets(self):
  u=copy.deepcopy(self.units);u[0]['raw_start']=1
  with self.assertRaises(BoundaryError):verify(u,self.page)
 def test_duplicate_anchor(self):
  page=copy.deepcopy(self.page);page['text']+='□ bad'
  with self.assertRaises(BoundaryError):verify(self.units,page)
 def test_order_overlap_rejected(self):
  with self.assertRaises(BoundaryError):verify(self.units*2,self.page)
 def test_raw_markdown_retained(self):self.assertEqual(self.page['text'],'□ bad\nunlocated tail');self.assertTrue(self.page['uncovered_text_requires_review'])
 def test_legacy_live_disabled(self):
  with self.assertRaises(BoundaryError):run({}, {},None)
 def test_cropped_line_boundary(self):
  p=self.payload('replace');p['patches'][0]['text']='line\nneighbor'
  with self.assertRaises(BoundaryError):validate(p,self.shape)
 def test_unknown_not_dispatched(self):
  u=copy.deepcopy(self.units);u[0]['locator']='UNKNOWN';self.assertFalse(select(u)['selected'])
  with self.assertRaises(BoundaryError):request_shape(self.page,u,self.image,self.root/'other')
 def test_whole_page_forbidden(self):
  u=copy.deepcopy(self.units);u[0]['bbox']=[0,0,200,400]
  with self.assertRaises(BoundaryError):request_shape(self.page,u,self.image,self.root/'other')

 def test_cross_operation_restart_dedup(self):
  from unittest.mock import patch
  from .selective import repair_manifest
  shape=self.shape;call=repair_manifest(shape,{},1)['calls'][0];sent=[]
  def transport(url,body):sent.append(url);return 200,canonical(dict(usage=dict(input_tokens=10,output_tokens=10)))
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=self.root/'ledger'):
   first=durable_request(call,shape['body'],None,{},transport)
   second=durable_request(copy.deepcopy(call),shape['body'],None,{},transport)
  self.assertEqual(first,second);self.assertEqual(len(sent),1)
 def test_unknown_usage_stops_different_call(self):
  from unittest.mock import patch
  from .selective import repair_manifest
  call=repair_manifest(self.shape,{},1)['calls'][0];sent=[]
  def transport(url,body):sent.append(url);return 429,b'{}'
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=self.root/'ledger'):
   durable_request(call,self.shape['body'],None,{},transport)
   changed=copy.deepcopy(self.shape['body']);changed['max_output_tokens']=4000;other=copy.deepcopy(call);other['body_sha256']=hashlib.sha256(canonical(changed)).hexdigest();other['request_id']=hashlib.sha256(canonical({k:other[k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
   with self.assertRaises(BoundaryError):durable_request(other,changed,None,{},transport)
  self.assertEqual(len(sent),1)
 def test_pixel_neighbor_cannot_prove_line(self):
  image=Image.open(self.image);ImageDraw.Draw(image).rectangle((120,25,150,34),fill='black');image.save(self.image)
  self.page['regions'][0].pop('line_proof');prove_lines(self.page,self.image)
  self.assertNotIn('line_proof',self.page['regions'][0])
 def test_review_identity_change_invalidates(self):
  from .pipeline import write,review,export_pdf
  import fitz
  pdf=self.root/'input.pdf';d=fitz.open();d.new_page(width=200,height=400);d.new_page(width=200,height=400);d.save(pdf);d.close()
  out=self.root/'operation';out.mkdir();task=dict(operation_id='test',font_path=self.font,page_numbers=[1],input_pdf=str(pdf),input_sha256=digest(pdf),images=[dict(path=str(self.image),sha256=digest(self.image))]);write(out/'task.json',task);pages=[dict(page_number=1,status='EXPORT_REVIEW',observation=self.page)];write(out/'pages-v0.json',pages)
  review(out,0,task['input_sha256'],[]);pages[0]['observation']['repair_units']=self.units;write(out/'pages-v0.json',pages)
  with self.assertRaises(BoundaryError):export_pdf(out,1)
 def test_full_pdf_unselected_page_kept(self):
  from .pipeline import write,review,export_pdf
  import fitz
  pdf=self.root/'input.pdf';d=fitz.open();d.new_page(width=200,height=400);p=d.new_page(width=222,height=333);p.insert_text((20,30),'native unselected');d.save(pdf);d.close()
  out=self.root/'operation';out.mkdir();task=dict(operation_id='test',font_path=self.font,page_numbers=[1],input_pdf=str(pdf),input_sha256=digest(pdf),images=[dict(path=str(self.image),sha256=digest(self.image))]);write(out/'task.json',task);write(out/'pages-v0.json',[dict(page_number=1,status='EXPORT_REVIEW',observation=self.page)])
  review(out,0,task['input_sha256'],[]);result=export_pdf(out,1)
  with fitz.open(result['searchable_pdf']) as reopened:self.assertEqual(len(reopened),2);self.assertIn('native unselected',reopened[1].get_text());self.assertEqual(reopened[1].rect.width,222)
  self.assertIn('unlocated tail',Path(result['text']).read_text());self.assertEqual(result['coverage'][0]['status'],'EXPORT_REVIEW')
 def test_selective_base_crop_patch_review_pdf_cycle(self):
  from unittest.mock import patch
  from .pipeline import review,export_pdf
  from .selective import mistral_manifest,base_run,repair_manifest,repair_run,review_context
  import fitz
  pdf=self.root/'source.pdf';d=fitz.open();d.new_page(width=200,height=400);d.new_page(width=250,height=350);d.save(pdf);d.close();out=self.root/'operation'
  task=dict(mode='paid',font_path=self.font,operation_id='synthetic-cycle',input_pdf=str(pdf),input_sha256=digest(pdf),page_numbers=[1],images=[dict(path=str(self.image),sha256=digest(self.image))],output_directory=str(out),config_version='v3-test',region_protocol=VERSION)
  def approval(m):return dict(manifest_seal=m['seal'],combined_reservation_reconciled=True,mistral_free_only=True,historical_spent_usd=.04,historical_unsettled_usd=.008,combined_new_reserve_usd=m['worst_case_reservation_usd'],cap_usd=1,approved_request_ids=[c['request_id'] for c in m['calls']],approved_images=m['calls'])
  m=mistral_manifest(task)
  def mistral(url,body):
   return 200,canonical(dict(model='mistral-ocr-4-1',usage_info=dict(pages_processed=1),pages=[dict(index=0,markdown='□ bad\nunlocated tail',dimensions=dict(width=200,height=400),blocks=[dict(content='□ bad',top_left_x=20,top_left_y=20,bottom_right_x=100,bottom_right_y=40)])]))
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=self.root/'ledger'):
   result=base_run(m,approval(m),None,mistral);page=result['page_results'][0]['observation'];self.assertEqual(len(page['selection']['selected']),1)
   shape=request_shape(page,page['selection']['selected'],self.image,self.root/'actual-crop');repair=repair_manifest(shape,task,1);u=shape['units'][0]
   payload=dict(protocol=VERSION,batch_sha256=shape['batch_sha256'],patches=[dict(id=u['id'],decision='replace',unit_sha256=hashlib.sha256(canonical(u)).hexdigest(),text='good')])
   def luna(url,body):return 200,canonical(dict(model='gpt-6-luna',status='completed',usage=dict(input_tokens=10,output_tokens=10),output=[dict(type='message',content=[dict(type='output_text',text=json.dumps(payload))])]))
   repair_run(repair,approval(repair),None,luna)
  _,identity=review_context(out,result['page_results']);region=page['regions'][0]
  action=dict(region_id=region['id'],member_ids=region['member_ids'],action='accept',source_evidence='synthetic test only',reviewer='machine fixture',source_image_sha256=digest(self.image),source_image_loaded=True,review_identity=identity)
  review(out,0,task['input_sha256'],[action]);export=export_pdf(out,1)
  self.assertIn('good\nunlocated tail',Path(export['text']).read_text());self.assertFalse(export['human_reviewed'])
  with fitz.open(export['searchable_pdf']) as reopened:self.assertEqual(len(reopened),2);self.assertIn('good',reopened[0].get_text())
 def test_simultaneous_different_operations_atomic_global_stop(self):
  """Both threads reach BEGIN before either reserves; exactly one fake send."""
  import threading,concurrent.futures
  from unittest.mock import patch
  from . import pipeline
  from .selective import repair_manifest
  original_locked=pipeline.locked;barrier=threading.Barrier(2);blocked=threading.Event();sent=[];mutex=threading.Lock()
  (self.root/'ledger').mkdir();original_locked(self.root/'ledger').close()
  class SimultaneousConnection:
   def __init__(self,db):self.db=db
   def execute(self,sql,*args):
    if sql=='BEGIN IMMEDIATE':barrier.wait(timeout=5)
    return self.db.execute(sql,*args)
   def __getattr__(self,name):return getattr(self.db,name)
  def locked(directory):return SimultaneousConnection(original_locked(directory))
  first=repair_manifest(self.shape,{},1)['calls'][0];second=copy.deepcopy(first);other_body=copy.deepcopy(self.shape['body']);other_body['max_output_tokens']=4000;second['body_sha256']=hashlib.sha256(canonical(other_body)).hexdigest();second['request_id']=hashlib.sha256(canonical({k:second[k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
  def transport(url,body):
   with mutex:sent.append(url)
   if not blocked.wait(timeout=5):raise AssertionError('concurrent request failed to reject while first was reserved')
   raise TimeoutError('simulated uncertain network delivery')
  def worker(call,body):
   try:return durable_request(call,body,None,{},transport)
   except BoundaryError as error:blocked.set();return error
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=self.root/'ledger'),patch('scripts.ocr.paid_mvp.pipeline.locked',side_effect=locked):
   with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
    futures=[pool.submit(worker,first,self.shape['body']),pool.submit(worker,second,other_body)];results=[f.result(timeout=10) for f in futures]
  self.assertEqual(len(sent),1);self.assertEqual(sum(isinstance(r,BoundaryError) for r in results),1)
  receipt=next(r for r in results if isinstance(r,dict));self.assertEqual(receipt['error'],'TimeoutError');self.assertIsNone(receipt['list_price_estimate_usd'])
  db=original_locked(self.root/'ledger');rows=db.execute('SELECT state,cost FROM calls').fetchall();db.close();self.assertEqual(rows,[('failed',None)])
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=self.root/'ledger'):
   for call,body in [(first,self.shape['body']),(second,other_body)]:
    with self.assertRaises(BoundaryError):durable_request(call,body,None,{},transport)
  self.assertEqual(len(sent),1)

 def semantic_repair_fixture(self,name):
  """Only generated pixels/PDF and a fake response; never the user's ledger."""
  from .pipeline import write
  from .selective import repair_manifest
  import fitz
  pdf=self.root/(name+'.pdf');source=fitz.open();source.new_page(width=200,height=400);source.save(pdf);source.close()
  out=self.root/name;out.mkdir();page=copy.deepcopy(self.page);page['repair_units']=self.units;page['selection']=dict(selected=self.units)
  task=dict(mode='paid',operation_id=name,input_pdf=str(pdf),input_sha256=digest(pdf),images=[dict(path=str(self.image),sha256=digest(self.image))],output_directory=str(out),config_version='synthetic-semantic-halt',region_protocol=VERSION)
  write(out/'pages-v0.json',[dict(page_number=1,status='awaiting_crop_authorization',observation=page)])
  manifest=repair_manifest(self.shape,task,1);ledger=self.root/(name+'-shared-state')
  approval=dict(manifest_seal=manifest['seal'],combined_reservation_reconciled=True,mistral_free_only=True,historical_spent_usd=.04,historical_unsettled_usd=.008,combined_new_reserve_usd=manifest['worst_case_reservation_usd'],cap_usd=1,approved_request_ids=[c['request_id']for c in manifest['calls']],approved_images=manifest['calls'])
  return manifest,approval,ledger,out

 def luna_response(self,payload):
  return dict(model='gpt-6-luna',status='completed',usage=dict(input_tokens=10,output_tokens=10),output=[dict(type='message',content=[dict(type='output_text',text=json.dumps(payload))])])

 def changed_repair_call(self,manifest):
  call=copy.deepcopy(manifest['calls'][0]);body=copy.deepcopy(manifest['shape']['body']);body['max_output_tokens']=4000
  call['body_sha256']=hashlib.sha256(canonical(body)).hexdigest();call['request_id']=hashlib.sha256(canonical({k:call[k]for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
  return call,body

 def test_rejected_luna_semantics_blocks_shared_dispatch(self):
  from unittest.mock import patch
  from .selective import repair_run
  import sqlite3
  invalid=[]
  response=self.luna_response(self.payload());response['model']='unexpected-model';invalid.append(('wrong-model',response))
  response=self.luna_response(self.payload());response['status']='incomplete';invalid.append(('incomplete-status',response))
  response=self.luna_response(self.payload());response['output'][0]['content'][0]['text']='not JSON';invalid.append(('invalid-JSON',response))
  payload=self.payload();payload['batch_sha256']='another-batch';invalid.append(('wrong-batch',self.luna_response(payload)))
  payload=self.payload();payload['patches'][0]['unit_sha256']='another-member';invalid.append(('wrong-member',self.luna_response(payload)))
  payload=self.payload();payload['patches']=[];invalid.append(('missing-member',self.luna_response(payload)))
  payload=self.payload();payload['patches']=None;invalid.append(('invalid-patches-shape',self.luna_response(payload)))
  response=self.luna_response(self.payload());response['output']=['not-message-object'];invalid.append(('invalid-output-shape',response))
  for name,response in invalid:
   with self.subTest(reason=name):
    manifest,approval,ledger,out=self.semantic_repair_fixture(name);base_bytes=(out/'pages-v0.json').read_bytes();sent=[]
    def transport(url,body):sent.append(url);return 200,canonical(response)
    with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=ledger):
     with self.assertRaises(BoundaryError):repair_run(manifest,approval,None,transport)
     with sqlite3.connect(ledger/'state.sqlite')as db:state,cost,receipt_path=db.execute('SELECT state,cost,receipt FROM calls').fetchone()
     self.assertEqual(state,'identity_failed');self.assertIsNotNone(cost)
     receipt=json.loads(Path(receipt_path).read_text());self.assertEqual(Path(receipt['raw_path']).read_bytes(),canonical(response))
     self.assertEqual(digest(receipt['raw_path']),receipt['response_sha256'])
     halt=json.loads((Path(receipt['raw_path']).parent/'dispatch-halt.json').read_text());self.assertTrue(halt['raw_preserved'])
     self.assertEqual((out/'pages-v0.json').read_bytes(),base_bytes);self.assertEqual(list(out.glob('patches-*.json')),[])
     # Cached identical identity and a later distinct request both stay stopped.
     call=manifest['calls'][0]
     with self.assertRaises(BoundaryError):durable_request(call,manifest['shape']['body'],None,{},transport)
     other,body=self.changed_repair_call(manifest)
     with self.assertRaises(BoundaryError):durable_request(other,body,None,{},transport)
    self.assertEqual(len(sent),1)

 def test_luna_waits_for_semantic_validation_before_next_request(self):
  from unittest.mock import patch
  from .selective import repair_run
  import sqlite3
  manifest,approval,ledger,out=self.semantic_repair_fixture('pending-validation');sent=[];observed=[]
  def transport(url,body):sent.append(url);return 200,canonical(self.luna_response(self.payload()))
  original_validate=validate
  def verify_pending(payload,shape):
   with sqlite3.connect(ledger/'state.sqlite')as db:state=db.execute('SELECT state FROM calls').fetchone()[0]
   observed.append(state);self.assertEqual(state,'received')
   other,body=self.changed_repair_call(manifest)
   with self.assertRaises(BoundaryError):durable_request(other,body,None,{},transport)
   return original_validate(payload,shape)
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=ledger),patch('scripts.ocr.paid_mvp.selective.validate',side_effect=verify_pending):
   result=repair_run(manifest,approval,None,transport)
  self.assertEqual(observed,['received']);self.assertEqual(len(sent),1);self.assertEqual(result['status'],'source_review_required')
  with sqlite3.connect(ledger/'state.sqlite')as db:self.assertEqual(db.execute('SELECT state FROM calls').fetchone()[0],'complete')
  self.assertEqual(len(list(out.glob('patches-*.json'))),1)

 def test_successful_luna_validation_allows_later_exact_request(self):
  from unittest.mock import patch
  from .selective import repair_run
  import sqlite3
  manifest,approval,ledger,out=self.semantic_repair_fixture('valid-validation');base_bytes=(out/'pages-v0.json').read_bytes();sent=[]
  def transport(url,body):sent.append(url);return 200,canonical(self.luna_response(self.payload()))
  with patch('scripts.ocr.paid_mvp.selective.ledger_directory',return_value=ledger):
   result=repair_run(manifest,approval,None,transport)
   self.assertFalse(result['human_reviewed']);self.assertEqual(result['status'],'source_review_required')
   with sqlite3.connect(ledger/'state.sqlite')as db:self.assertEqual(db.execute('SELECT state FROM calls').fetchone()[0],'complete')
   other,body=self.changed_repair_call(manifest);durable_request(other,body,None,{},transport)
  self.assertEqual(len(sent),2);self.assertEqual((out/'pages-v0.json').read_bytes(),base_bytes)
