import copy, tempfile, unittest
from pathlib import Path
from pdf_test_fixture import make_pdf
from pypdf import PdfReader
from pypdf.generic import DictionaryObject, NameObject, NumberObject
from bookmarks import sha, save, object_fingerprint
from desktop_bridge import reviewed_table, save_review, check_cancel
from numeric_lane import prepare

class DesktopContracts(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.source=self.root/'original.pdf'
  make_pdf(self.source,[f'Original {i}' for i in range(3)])
  self.base=dict(schema='mpdf-bookmark-table/1',source=str(self.source),source_sha256=sha(self.source),page_count=3,entries=[dict(id='a',title='A',parent=None,level=0,target_pdf_page=None,state='needs_review',review_reasons=['printed_missing'],source_page=0,evidence_ids=['observed-a'])])
  self.edits=[dict(id='a',title='Reviewed A',parent=None,target_pdf_page=1)]
 def tearDown(self):self.tmp.cleanup()
 def test_book_review_only_accepts_editable_fields_and_preserves_evidence(self):
  t=reviewed_table(self.base,self.edits,True,'navigation')
  self.assertEqual(t['entries'][0]['evidence_ids'],['observed-a'])
  self.assertIsNone(self.base['entries'][0]['target_pdf_page'])
  for field in ['source','evidence_ids','state']:
   with self.assertRaises(ValueError):reviewed_table(self.base,[dict(self.edits[0],**{field:'injected'})],True,'navigation')
  with self.assertRaises(ValueError):reviewed_table(self.base,self.edits,False,'navigation')
 def test_invalid_identity_cycle_page_and_control_char_are_rejected(self):
  for fields in [dict(id='unknown'),dict(parent='a'),dict(target_pdf_page=3),dict(title='bad\x00title')]:
   with self.assertRaises(ValueError):reviewed_table(self.base,[dict(self.edits[0],**fields)],True,'navigation')
 def test_manual_entry_reparent_and_source_projection_are_validated(self):
  t=reviewed_table(self.base,[dict(self.edits[0],parent='manual-root'),dict(id='manual-root',title='Root',parent=None,target_pdf_page=0)],True,'source')
  self.assertEqual([(e['id'],e['level']) for e in t['entries']],[('manual-root',0),('a',1)])
 def test_cancelled_save_does_not_publish(self):
  save(self.root/'table.json',self.base);(self.root/'cancel').touch()
  with self.assertRaises(InterruptedError):save_review(dict(table_path=str(self.root/'table.json'),entries=self.edits,review_accepted=True,projection='navigation',output=str(self.root/'out.pdf')),self.root)
  self.assertFalse((self.root/'out.pdf').exists())
 def test_combined_writer_keeps_review_source_identity_and_processed_pages(self):
  processed=self.root/'processed.pdf'
  make_pdf(processed,[f'Processed {i}' for i in range(3)])
  save(self.root/'table.json',self.base)
  receipt=save_review(dict(table_path=str(self.root/'table.json'),entries=self.edits,review_accepted=True,projection='navigation',processed_source=str(processed),output=str(self.root/'out.pdf')),self.root)
  self.assertEqual(receipt['actual_destinations_checked'],1)
  d=PdfReader(self.root/'out.pdf');self.assertEqual(d.outline[0].title,'Reviewed A');self.assertEqual(d.get_destination_page_number(d.outline[0]),1);self.assertIn('Processed',d.pages[0].extract_text())
  self.assertEqual(sha(self.source),self.base['source_sha256'])
 def test_dictionary_key_order_is_harmless_but_changed_page_value_is_not(self):
  before=DictionaryObject({NameObject('/Rotate'):NumberObject(0),NameObject('/Type'):NameObject('/Page')})
  after=DictionaryObject({NameObject('/Type'):NameObject('/Page'),NameObject('/Rotate'):NumberObject(0)})
  self.assertEqual(object_fingerprint(before),object_fingerprint(after))
  after[NameObject('/Rotate')]=NumberObject(90)
  self.assertNotEqual(object_fingerprint(before),object_fingerprint(after))
 def test_blank_vision_page_has_zero_numeric_crops(self):
  raw=dict(source=str(self.source),source_sha256=sha(self.source),page_index=0,page_count=3,width=595,height=842,bbox_convention='normalized_bottom_left_xywh',observations=[])
  save(self.root/'raw.json',raw)
  self.assertEqual(prepare([self.root/'raw.json'],self.root/'crops'),[])
if __name__=='__main__':unittest.main()
