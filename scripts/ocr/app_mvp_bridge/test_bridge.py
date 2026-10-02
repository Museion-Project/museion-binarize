"""Consumer/security contract tests; no OCR/cloud/quality evidence."""
import json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import fitz
from .__main__ import main,context,source_bound,sha,denied,packaged_config,register_directory_table,register_local_operation
class BridgeBoundary(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.source=self.root/'source.pdf';self.source.write_bytes(b'immutable-source');self.config={'session_root':str(self.root/'sessions')};self.request=dict(input_pdf=str(self.source),input_sha256=sha(self.source),mode='paid')
 def tearDown(self):self.temp.cleanup()
 def test_explicit_mode_and_action_no_generic_execution(self):
  for mode,action in [('automatic','readiness'),('paid','run'),('local','shell')]:
   with self.assertRaises(ValueError):main(dict(self.request,mode=mode,action=action),self.config)
 def test_packaged_inventory_rejects_tampered_code_and_stale_version(self):
  resources=self.root/'resources';resources.mkdir();manifest=dict(schema='museion-local-runtime/1',config_version='v2',exporter_version='export1',tessdata='tessdata')
  for name in ['python','apple_helper','tesseract','font']:
   relative={'python':'python/bin/python','apple_helper':'bin/helper','tesseract':'bin/tesseract','font':'fonts/font.ttf'}[name]
   file=resources/relative;file.parent.mkdir(parents=True,exist_ok=True);file.write_text('test fixture');manifest[name]=relative
  from .runtime_contract import SOURCE_FILES,INITIALIZERS
  (resources/'tessdata').mkdir()
  for lang in ('eng','grc'):(resources/'tessdata'/(lang+'.traineddata')).write_text('fixture')
  for name in set(SOURCE_FILES)|INITIALIZERS:
   file=resources/name;file.parent.mkdir(parents=True,exist_ok=True);file.write_text('')
  core=resources/'scripts/ocr/mvp/core.py';core.write_text("CONFIG_VERSION='v2'\n")
  store=resources/'scripts/ocr/mvp/store.py';store.write_text("EXPORTER_VERSION='export1'\n")
  manifest['code_files']={name:sha(resources/name) for name in SOURCE_FILES}
  manifest['runtime_files']={str(p.relative_to(resources)):sha(p) for p in resources.rglob('*') if p.is_file()}
  manifest['modes']=['local']
  path=resources/'runtime-manifest.json';path.write_text(json.dumps(manifest))
  self.assertTrue(packaged_config(resources,self.root/'sessions')['runtime_inventory_verified'])
  manifest['config_version']='v1';path.write_text(json.dumps(manifest))
  with self.assertRaisesRegex(ValueError,'RESOURCE_CONFIG_VERSION_MISMATCH'):packaged_config(resources,self.root/'sessions')
  manifest['config_version']='v2';path.write_text(json.dumps(manifest));core.write_text('tampered')
  with self.assertRaisesRegex(ValueError,'RESOURCE_HASH_MISMATCH'):packaged_config(resources,self.root/'sessions')
 def test_paid_start_never_creates_a_request_or_session(self):
  with self.assertRaisesRegex(ValueError,'CLOUD_SEND_DISABLED'):main(dict(self.request,action='start'),self.config)
  self.assertEqual(list((self.root/'sessions').iterdir()),[])
 def test_source_hash_and_path_are_independent(self):
  source_bound(self.source,sha(self.source),self.request)
  with self.assertRaisesRegex(ValueError,'HASH'):source_bound(self.source,'0'*64,self.request)
  duplicate=self.root/'duplicate.pdf';duplicate.write_bytes(self.source.read_bytes())
  with self.assertRaisesRegex(ValueError,'PATH'):source_bound(duplicate,sha(duplicate),self.request)
 def test_session_identifier_cannot_escape_owned_root(self):
  with self.assertRaisesRegex(ValueError,'INVALID_SESSION'):context(dict(self.request,session_id='../secrets'),self.config)
 def test_existing_result_must_match_current_document_before_copy(self):
  imported=self.root/'result.json';imported.write_text(json.dumps(dict(mode='paid-contents',status='failed',input_sha256='0'*64)))
  with self.assertRaisesRegex(ValueError,'HASH'):main(dict(self.request,mode='paid-contents',action='import',import_path=str(imported)),self.config)
  self.assertFalse(any((self.root/'sessions').glob('*/operation')))
 def test_saved_subset_cannot_silently_discard_unprocessed_pages(self):
  doc=fitz.open();doc.new_page();doc.new_page();doc.save(self.source);doc.close();self.request['input_sha256']=sha(self.source)
  derived=self.root/'old-subset.pdf';doc=fitz.open();doc.new_page();doc.save(derived);doc.close()
  sid='a'*32;folder=self.root/'sessions'/sid;folder.mkdir(parents=True);(folder/'desktop-session.json').write_text(json.dumps(dict(mode='local',source=str(self.source),source_sha256=sha(self.source),provenance='synthetic consumer boundary')))
  output=self.root/'must-not-exist.pdf'
  with patch('scripts.ocr.app_mvp_bridge.__main__.view',return_value=dict(revision=0,output_pdf=str(derived))):
   with self.assertRaisesRegex(ValueError,'HISTORICAL_OUTPUT_SCOPE'):main(dict(self.request,mode='local',action='save',session_id=sid,expected_revision=0,output_path=str(output)),self.config)
  self.assertFalse(output.exists())
 def test_network_disabled_for_all_actions(self):
  with self.assertRaisesRegex(OSError,'NETWORK_DISABLED'):denied('api.openai.com')
 def test_local_candidate_never_allows_import_or_other_modes(self):
  for mode,action in [('local','import'),('paid','readiness'),('critical-edition','start')]:
   with self.assertRaisesRegex(ValueError,'LOCAL_CANDIDATE_ONLY'):main(dict(self.request,mode=mode,action=action),dict(self.config,local_only=True))
 def test_legacy_unbound_packaged_manifest_is_rejected(self):
  (self.root/'runtime-manifest.json').write_text(json.dumps(dict(schema='museion-local-runtime/1',apple_helper='../outside')))
  with self.assertRaisesRegex(ValueError,'RESOURCE_IDENTITY_INCOMPLETE'):packaged_config(self.root,self.root/'sessions')
 def test_legacy_directory_is_diagnostic_and_cannot_save_or_review(self):
  folder=self.root/'sessions'/('a'*32);folder.mkdir(parents=True)
  table=dict(revision=0,source=str(self.source),source_sha256=sha(self.source))
  meta=dict(mode='paid-contents',source=str(self.source),source_sha256=sha(self.source),provenance='legacy')
  register_directory_table(self.root/'legacy.json',folder,table,meta)
  self.assertTrue(meta['diagnostic_only']);(folder/'desktop-session.json').write_text(json.dumps(meta))
  for action in ('review','save'):
   with self.assertRaisesRegex(ValueError,'DIRECTORY_DIAGNOSTIC_ONLY'):main(dict(self.request,mode='paid-contents',session_id=folder.name,action=action),self.config)
 def test_resume_is_bound_to_actual_source_path_and_hash(self):
  sid='b'*32;folder=self.root/'sessions'/sid;(folder/'operation').mkdir(parents=True)
  (folder/'desktop-session.json').write_text(json.dumps(dict(mode='local',source=str(self.source),source_sha256=sha(self.source),provenance='test runtime state')))
  (folder/'operation/CURRENT.json').write_text('{}')
  with patch('scripts.ocr.app_mvp_bridge.__main__.view',return_value=dict(session_id=sid,revision=1)):
   actual=main(dict(self.request,mode='local',action='resume'),self.config);self.assertEqual(actual['session_id'],sid)
  duplicate=self.root/'duplicate.pdf';duplicate.write_bytes(self.source.read_bytes())
  for extra in [dict(input_sha256='0'*64),dict(input_pdf=str(duplicate))]:
   with self.assertRaisesRegex(ValueError,'NO_LOCAL_DRAFT'):main(dict(self.request,mode='local',action='resume',**extra),self.config)
 def test_cancel_before_registration_never_creates_output_directory(self):
  sid='c'*32;folder=self.root/'sessions'/sid;folder.mkdir(parents=True)
  (folder/'desktop-session.json').write_text(json.dumps(dict(mode='local',source=str(self.source),source_sha256=sha(self.source),client_operation_id='d'*32,provenance='test')))
  request=dict(self.request,mode='local',action='cancel',session_id=sid,client_operation_id='d'*32)
  with self.assertRaisesRegex(ValueError,'UNKNOWN_OPERATION'):main(request,self.config)
  self.assertFalse((folder/'operation').exists())
 def test_registered_start_can_cancel_in_session_event_before_any_worker(self):
  import io
  doc=fitz.open();doc.new_page().insert_text((20,30),'native retained');doc.new_page();doc.save(self.source);doc.close();self.request['input_sha256']=sha(self.source)
  config=dict(self.config,local=dict(font=str(Path(__file__).resolve().parents[3]/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf')))
  outer=self;token='e'*32;events=[]
  class CancelOnSession(io.StringIO):
   def write(self,value):
    if value.strip():
     event=json.loads(value);events.append(event);folder=Path(config['session_root'])/event['session_id'];outer.assertTrue((folder/'operation/job.json').is_file())
     main(dict(outer.request,mode='local',action='cancel',session_id=event['session_id'],client_operation_id=token),config)
    return len(value)
  with patch('scripts.ocr.app_mvp_bridge.__main__.sys.stderr',CancelOnSession()),patch('scripts.ocr.mvp.local.subprocess.Popen') as spawn:
   result=main(dict(self.request,mode='local',action='start',pages=[1],client_operation_id=token),config)
   spawn.assert_not_called()
  self.assertEqual(result['status'],'cancelled');self.assertEqual(events[0]['client_operation_id'],token)
  with fitz.open(result['output_pdf']) as saved:self.assertEqual(len(saved),2);self.assertIn('native retained',saved[0].get_text())
  op=Path(config['session_root'])/result['session_id']/'operation';self.assertFalse(list((op/'raw').glob('worker-*')))
  with self.assertRaisesRegex(ValueError,'CANCEL_OPERATION_MISMATCH'):main(dict(self.request,mode='local',action='cancel',session_id=result['session_id'],client_operation_id='f'*32),config)
 def test_registration_never_overwrites_an_existing_output(self):
  folder=self.root/'new';folder.mkdir();(folder/'operation').mkdir();marker=folder/'operation/result.pdf';marker.write_bytes(b'old-artifact')
  with self.assertRaises(FileExistsError):register_local_operation(folder,{}, {})
  self.assertEqual(marker.read_bytes(),b'old-artifact')
 def test_real_native_start_uses_current_consumer_version_without_ocr(self):
  from scripts.ocr.mvp.core import CONFIG_VERSION,CONSUMER_POLICY
  text='NATIVE VERSION CROSSING 314159 '+('preserve original visible text without recognition '*4)
  doc=fitz.open();doc.new_page().insert_textbox(fitz.Rect(20,20,550,500),text);doc.save(self.source);doc.close()
  self.request['input_sha256']=sha(self.source)
  # Native-only test: these non-executable PDF identity fixtures cannot OCR.
  config=dict(self.config,local=dict(apple_helper=str(self.source),tesseract=str(self.source),font=str(Path(__file__).resolve().parents[3]/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf')))
  result=main(dict(self.request,mode='local',action='start',pages=[1],client_operation_id='1'*32),config)
  op=Path(config['session_root'])/result['session_id']/'operation'
  self.assertEqual(json.loads((op/'job.json').read_text())['task']['config_version'],CONFIG_VERSION)
  self.assertEqual(result['pages'][0]['status'],'NATIVE_PRESERVED')
  self.assertFalse(list((op/'raw').glob('**/*.call.json')))
  from scripts.ocr.mvp.store import load_snapshot
  snapshot,_=load_snapshot(op)
  self.assertEqual(snapshot['consumer_policy'],CONSUMER_POLICY)
  with fitz.open(result['output_pdf']) as saved:self.assertIn('NATIVE VERSION CROSSING 314159',saved[0].get_text())
if __name__=='__main__':unittest.main()
