import copy, tempfile, unittest
from pathlib import Path
import fitz
from bookmarks import sha, save, same_original_object
from desktop_bridge import reviewed_table, save_review, check_cancel
from numeric_lane import prepare

class DesktopContracts(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.source=self.root/'original.pdf'
  with fitz.open() as d:
   for i in range(3):d.new_page().insert_text((72,72),f'Original {i}')
   d.save(self.source)
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
  with fitz.open() as d:
   for i in range(3):d.new_page().insert_text((72,72),f'Processed {i}')
   d.save(processed)
  save(self.root/'table.json',self.base)
  receipt=save_review(dict(table_path=str(self.root/'table.json'),entries=self.edits,review_accepted=True,projection='navigation',processed_source=str(processed),output=str(self.root/'out.pdf')),self.root)
  self.assertEqual(receipt['actual_destinations_checked'],1)
  with fitz.open(self.root/'out.pdf') as d:self.assertEqual(d.get_toc(),[[1,'Reviewed A',2]]);self.assertIn('Processed',d[0].get_text())
  self.assertEqual(sha(self.source),self.base['source_sha256'])
 def test_dictionary_key_order_is_harmless_but_changed_page_value_is_not(self):
  before=fitz.open();before.new_page();xref=before[0].xref
  before.update_object(xref,'<< /MediaBox [0 0 595 842] /Rotate 0 /Type /Page >>')
  after=fitz.open();after.new_page();after.update_object(xref,'<< /Type /Page /Rotate 0 /MediaBox [0 0 595 842] >>')
  self.assertTrue(same_original_object(before,after,xref))
  after.xref_set_key(xref,'Rotate','90')
  self.assertFalse(same_original_object(before,after,xref))
  before.close();after.close()
 def test_blank_vision_page_has_zero_numeric_crops(self):
  raw=dict(source=str(self.source),source_sha256=sha(self.source),page_index=0,page_count=3,width=595,height=842,bbox_convention='normalized_bottom_left_xywh',observations=[])
  save(self.root/'raw.json',raw)
  self.assertEqual(prepare([self.root/'raw.json'],self.root/'crops'),[])
if __name__=='__main__':unittest.main()
