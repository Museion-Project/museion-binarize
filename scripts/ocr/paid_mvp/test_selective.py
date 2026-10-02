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
  out=self.root/'operation';out.mkdir();task=dict(operation_id='test',page_numbers=[1],input_pdf=str(pdf),input_sha256=digest(pdf),images=[dict(path=str(self.image),sha256=digest(self.image))]);write(out/'task.json',task);pages=[dict(page_number=1,status='EXPORT_REVIEW',observation=self.page)];write(out/'pages-v0.json',pages)
  review(out,0,task['input_sha256'],[]);pages[0]['observation']['repair_units']=self.units;write(out/'pages-v0.json',pages)
  with self.assertRaises(BoundaryError):export_pdf(out,1)
 def test_full_pdf_unselected_page_kept(self):
  from .pipeline import write,review,export_pdf
  import fitz
  pdf=self.root/'input.pdf';d=fitz.open();d.new_page(width=200,height=400);p=d.new_page(width=222,height=333);p.insert_text((20,30),'native unselected');d.save(pdf);d.close()
  out=self.root/'operation';out.mkdir();task=dict(operation_id='test',page_numbers=[1],input_pdf=str(pdf),input_sha256=digest(pdf),images=[dict(path=str(self.image),sha256=digest(self.image))]);write(out/'task.json',task);write(out/'pages-v0.json',[dict(page_number=1,status='EXPORT_REVIEW',observation=self.page)])
  review(out,0,task['input_sha256'],[]);result=export_pdf(out,1)
  with fitz.open(result['searchable_pdf']) as reopened:self.assertEqual(len(reopened),2);self.assertIn('native unselected',reopened[1].get_text());self.assertEqual(reopened[1].rect.width,222)
  self.assertIn('unlocated tail',Path(result['text']).read_text());self.assertEqual(result['coverage'][0]['status'],'EXPORT_REVIEW')
 def test_selective_base_crop_patch_review_pdf_cycle(self):
  from unittest.mock import patch
  from .pipeline import review,export_pdf
  from .selective import mistral_manifest,base_run,repair_manifest,repair_run,review_context
  import fitz
  pdf=self.root/'source.pdf';d=fitz.open();d.new_page(width=200,height=400);d.new_page(width=250,height=350);d.save(pdf);d.close();out=self.root/'operation'
  task=dict(mode='paid',operation_id='synthetic-cycle',input_pdf=str(pdf),input_sha256=digest(pdf),page_numbers=[1],images=[dict(path=str(self.image),sha256=digest(self.image))],output_directory=str(out),config_version='v3-test',region_protocol=VERSION)
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
