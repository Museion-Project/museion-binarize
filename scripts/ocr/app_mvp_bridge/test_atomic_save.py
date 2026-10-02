"""Save failures on real disk copies; no GUI, OCR or cloud admission evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import fitz
from .__main__ import atomic_save_copy,sha,main


class AtomicSaveTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
  self.src=self.root/'derived.pdf';d=fitz.open();d.new_page().insert_text((30,40),'Complete searchable 314159');d.save(self.src);d.close()
  self.dest=self.root/'new.pdf';self.receipt=self.root/'save-complete.json'
 def tearDown(self):self.temp.cleanup()
 def save(self,validate=lambda:None):
  return atomic_save_copy(self.src,self.dest,validate=validate,receipt_path=self.receipt,result=dict(revision=2,review_required=True))
 def assert_no_partial(self):
  self.assertFalse(self.dest.exists());self.assertFalse(self.receipt.exists());self.assertFalse(list(self.root.glob('.museion-*')))
 def test_success_reopens_exact_pdf_and_receipt(self):
  result=self.save();self.assertEqual(sha(self.src),sha(self.dest));self.assertEqual(json.loads(self.receipt.read_text()),result)
  with fitz.open(self.dest) as d:self.assertIn('Complete searchable 314159',d[0].get_text())
  self.assertFalse(list(self.root.glob('save-pending-*')))
 def test_existing_target_is_preserved(self):
  self.dest.write_bytes(b'user file')
  with self.assertRaises(FileExistsError):self.save()
  self.assertEqual(self.dest.read_bytes(),b'user file');self.assertFalse(self.receipt.exists())
 def test_write_interrupted_never_publishes_partial(self):
  def fail(inp,out):out.write(b'%PDF-partial');raise OSError('simulated full disk')
  with patch('scripts.ocr.app_mvp_bridge.__main__.shutil.copyfileobj',side_effect=fail):
   with self.assertRaisesRegex(OSError,'full disk'):self.save()
  self.assert_no_partial()
 def test_target_created_at_commit_is_preserved(self):
  link=os.link
  def race(a,b):
   if Path(b)==self.dest:self.dest.write_bytes(b'race winner')
   return link(a,b)
  with patch('scripts.ocr.app_mvp_bridge.__main__.os.link',side_effect=race):
   with self.assertRaises(FileExistsError):self.save()
  self.assertEqual(self.dest.read_bytes(),b'race winner');self.assertFalse(self.receipt.exists())
 def test_changed_derived_during_copy_cannot_publish(self):
  from shutil import copyfileobj
  def change(inp,out):copyfileobj(inp,out);self.src.write_bytes(b'changed derived output')
  with patch('scripts.ocr.app_mvp_bridge.__main__.shutil.copyfileobj',side_effect=change):
   with self.assertRaisesRegex(ValueError,'DERIVED_OUTPUT_CHANGED'):self.save()
  self.assert_no_partial()
 def test_source_revision_or_cancellation_validation_blocks_commit(self):
  for message in ('SOURCE_CHANGED','STALE_REVISION','SAVE_CANCELLED'):
   def validate():raise ValueError(message)
   with self.assertRaisesRegex(ValueError,message):self.save(validate)
   self.assert_no_partial()
 def test_receipt_failure_rolls_back_only_owned_pdf(self):
  link=os.link
  def fail(a,b):
   if Path(b)==self.receipt:raise OSError('receipt commit failed')
   return link(a,b)
  with patch('scripts.ocr.app_mvp_bridge.__main__.os.link',side_effect=fail):
   with self.assertRaisesRegex(OSError,'receipt commit failed'):self.save()
  self.assert_no_partial();journal=list(self.root.glob('save-pending-*'));self.assertEqual(len(journal),1)
  self.assertEqual(json.loads(journal[0].read_text())['state'],'PREPARED')
 def test_failure_does_not_delete_externally_replaced_output(self):
  link=os.link
  def fail(a,b):
   if Path(b)==self.receipt:
    self.dest.unlink();self.dest.write_bytes(b'new user file');raise OSError('receipt failed')
   return link(a,b)
  with patch('scripts.ocr.app_mvp_bridge.__main__.os.link',side_effect=fail):
   with self.assertRaises(OSError):self.save()
  self.assertEqual(self.dest.read_bytes(),b'new user file')
 def test_main_revalidates_real_source_before_commit(self):
  source=self.root/'source.pdf';source.write_bytes(self.src.read_bytes());sid='a'*32;folder=self.root/'sessions'/sid;folder.mkdir(parents=True)
  h=sha(source);(folder/'desktop-session.json').write_text(json.dumps(dict(mode='local',source=str(source),source_sha256=h,provenance='local test')))
  from shutil import copyfileobj
  def change(inp,out):copyfileobj(inp,out);source.write_bytes(b'changed source')
  request=dict(mode='local',action='save',session_id=sid,input_pdf=str(source),input_sha256=h,expected_revision=2,output_path=str(self.dest))
  with patch('scripts.ocr.app_mvp_bridge.__main__.view',return_value=dict(revision=2,output_pdf=str(self.src))),patch('scripts.ocr.app_mvp_bridge.__main__.shutil.copyfileobj',side_effect=change):
   with self.assertRaisesRegex(ValueError,'SOURCE_CHANGED'):main(request,dict(session_root=str(folder.parent)))
  self.assertFalse(self.dest.exists());self.assertFalse(list(folder.glob('save-*.json')))
 def test_real_native_bridge_save_reload_and_literal_search_without_ocr(self):
  text='Native searchable 314159 '+('preserve source words in every selected page '*5)
  source=self.root/'source.pdf';doc=fitz.open()
  for i in range(2):doc.new_page().insert_textbox(fitz.Rect(20,20,550,500),text)
  doc.save(source);doc.close();h=sha(source)
  # Explicit native-only identity fixtures; no external research/runtime path.
  config=dict(session_root=str(self.root/'sessions'),local=dict(apple_helper=str(source),tesseract=str(source),font=str(Path(__file__).resolve().parents[3]/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf')))
  request=dict(mode='local',input_pdf=str(source),input_sha256=h)
  from scripts.ocr.free_local.pipeline import native_text
  with fitz.open(source) as doc:self.assertTrue(all(native_text(page) is not None for page in doc))
  started=main(dict(request,action='start',pages=[1,2],client_operation_id='c'*32),config)
  operation=Path(config['session_root'])/started['session_id']/'operation'
  self.assertFalse(list((operation/'raw').glob('**/*.call.json')))
  self.assertTrue(all(p['status']=='NATIVE_PRESERVED' for p in started['pages']))
  saved=main(dict(request,action='save',session_id=started['session_id'],expected_revision=started['revision'],output_path=str(self.dest)),config)
  self.assertEqual(saved['saved']['source_sha256'],h);self.assertEqual(sha(source),h);self.assertEqual(sha(Path(started['output_pdf'])),sha(self.dest))
  reloaded=main(dict(request,action='reload',session_id=started['session_id']),config);self.assertEqual(reloaded['revision'],started['revision'])
  result=main(dict(request,action='search',session_id=started['session_id'],saved_pdf=saved['saved']['output_pdf'],query='314159'),config)
  self.assertEqual(len([hit for hit in result['hits'] if hit['consumer']=='PDF exact Unicode']),2)
 def test_main_cancellation_during_save_prevents_commit(self):
  source=self.root/'source.pdf';source.write_bytes(self.src.read_bytes());sid='b'*32;folder=self.root/'sessions'/sid;(folder/'operation').mkdir(parents=True)
  h=sha(source);(folder/'desktop-session.json').write_text(json.dumps(dict(mode='local',source=str(source),source_sha256=h,provenance='local test')))
  from shutil import copyfileobj
  def cancel(inp,out):copyfileobj(inp,out);(folder/'operation/CANCEL').touch()
  request=dict(mode='local',action='save',session_id=sid,input_pdf=str(source),input_sha256=h,expected_revision=2,output_path=str(self.dest))
  with patch('scripts.ocr.app_mvp_bridge.__main__.view',return_value=dict(revision=2,output_pdf=str(self.src))),patch('scripts.ocr.app_mvp_bridge.__main__.shutil.copyfileobj',side_effect=cancel):
   with self.assertRaisesRegex(ValueError,'SAVE_CANCELLED'):main(request,dict(session_root=str(folder.parent)))
  self.assertFalse(self.dest.exists());self.assertFalse(list(folder.glob('save-*.json')))


if __name__=='__main__':unittest.main()
