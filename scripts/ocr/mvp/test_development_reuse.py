"""Baseline reuse identities, not recognition accuracy or execution probes."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from PIL import Image
from .development_panel import verify_reuse,qualify,sha


class DevelopmentReuseTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.op=self.root/'op';self.op.mkdir();self.raw=self.op/'raw';self.raw.mkdir()
  source=self.root/'source.pdf';source.write_bytes(b'immutable source fixture');image=self.root/'source.png';Image.new('RGB',(20,20),'white').save(image)
  crop=self.root/'crop.png';Image.open(image).crop((0,0,20,20)).save(crop)
  module=self.root/'core.py';module.write_text("CONFIG_VERSION='v2'\n");helper=self.root/'helper';helper.write_bytes(b'helper');reader=self.root/'reader';reader.write_bytes(b'reader');trained=self.root/'eng.traineddata';trained.write_bytes(b'trained')
  self.config=dict(residual_enabled=False,apple_helper=str(helper),tesseract=str(reader))
  self.runtime=dict(config=self.config,code_sha256={'core.py':sha(module)},loaded_modules={'scripts.ocr.mvp.core':dict(path=str(module),sha256=sha(module))},apple_helper_sha256=sha(helper),tesseract_executable_sha256=sha(reader),tessdata=[dict(name='eng',path=str(trained),sha256=sha(trained))])
  receipt=self.raw/'runtime-version.json';receipt.write_text(json.dumps(self.runtime));tsv=self.raw/'reader.tsv';tsv.write_text('immutable raw')
  member=dict(id='w1',text='known',bbox=[1,1,10,8]);self.page=dict(page=1,image_path=str(image),image_sha256=sha(image),raw_files={str(p.relative_to(self.op)):sha(p) for p in [receipt,tsv]},independent_reader=[member],words=[member])
  snapshot=self.op/'snapshot.json';snapshot.write_text(json.dumps(dict(config_version='v2',source_pdf=str(source),input_sha256=sha(source),pages=[self.page])))
  job=self.op/'job.json';job.write_text(json.dumps(dict(config=self.config,task=dict(config_version='v2',input_pdf=str(source),input_sha256=sha(source),page_numbers=[1]))))
  self.unit=dict(unit_id='u',book='b',page=1,source_id='b-p1',bbox=[0,0,20,20],crop=str(crop),crop_sha256=sha(crop),pg_scorable=True,source_visual_transcription='known missing',genuine_lexical_gap=True,baseline_members=dict(independent_reader=[member],words=[member]))
  rec=lambda p:dict(path=str(p),sha256=sha(p))
  self.freeze=dict(schema='development-reuse/1',candidate_config_version='v2',expected_execution_identity={k:self.runtime[k] for k in ('code_sha256','apple_helper_sha256','tesseract_executable_sha256','tessdata')},units={'u':dict(book='b',source_id='b-p1',operation=str(self.op),baseline_page=1,crop_bbox=[0,0,20,20],snapshot=rec(snapshot),job=rec(job),source_pdf=rec(source),source_image=rec(image),runtime_receipt=rec(receipt))})
 def tearDown(self):self.temp.cleanup()
 def check(self,unit=None,freeze=None):return verify_reuse([unit or self.unit],['u'],freeze or self.freeze)
 def test_complete_real_artifact_binding_verifies_without_calls(self):
  self.assertEqual(self.check()['state'],'VERIFIED');self.assertEqual(self.check()['new_ocr_calls'],0)
 def test_current_candidate_version_cannot_reuse_research_version(self):
  f=copy.deepcopy(self.freeze);f['candidate_config_version']='research-v1';self.assertEqual(self.check(freeze=f)['state'],'UNVERIFIED')
 def test_execution_identity_not_relabelled_with_current_hash(self):
  f=copy.deepcopy(self.freeze);f['expected_execution_identity']['apple_helper_sha256']='new';self.assertEqual(self.check(freeze=f)['state'],'UNVERIFIED')
 def test_raw_source_and_loaded_code_tamper_fail_closed(self):
  for name,message in [('raw/reader.tsv','REUSE_RAW_CHANGED'),('core.py','REUSE_LOADED_SOURCE_CHANGED')]:
   p=self.op/name if name.startswith('raw/') else self.root/name;before=p.read_bytes();p.write_bytes(b'changed')
   with self.assertRaisesRegex(ValueError,message):self.check()
   p.write_bytes(before)
 def test_reference_member_list_cannot_replace_reader_observations(self):
  u=copy.deepcopy(self.unit);u['baseline_members']['words'][0]['text']='reference-fed'
  with self.assertRaisesRegex(ValueError,'REUSE_MEMBER_LEDGER_MISMATCH'):self.check(unit=u)
 def test_crop_hash_alone_does_not_prove_source_pixels(self):
  u=copy.deepcopy(self.unit);Image.new('RGB',(20,20),'black').save(u['crop']);u['crop_sha256']=sha(u['crop'])
  with self.assertRaisesRegex(ValueError,'REUSE_CROP_PIXELS_MISMATCH'):self.check(unit=u)
 def test_duplicate_physical_unit_with_new_id_cannot_inflate_gap_panel(self):
  u=copy.deepcopy(self.unit);u['unit_id']='another';u['structure']='body';self.unit['structure']='body'
  with self.assertRaisesRegex(ValueError,'DUPLICATE_PHYSICAL_SOURCE_UNIT'):qualify([self.unit,u])
 def test_missing_freeze_remains_unverified_even_if_data_qualifies(self):
  self.assertEqual(verify_reuse([self.unit],['u'],None)['state'],'UNVERIFIED')


if __name__=='__main__':unittest.main()
