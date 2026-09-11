import copy,json,tempfile,unittest
from pathlib import Path
from pdf_test_fixture import make_pdf
from pypdf import PdfReader
from bookmarks import *

class Contracts(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.source=self.root/'source.pdf'
  make_pdf(self.source,[f'Original page {i+1} Native text and font resources remain' for i in range(3)],old_outline=True)
  self.table=dict(schema='mpdf-bookmark-table/1',source=str(self.source),source_sha256=sha(self.source),page_count=3,entries=[dict(id='a',title='Root',parent=None,level=0,target_pdf_page=0,state='manually_confirmed',review_reasons=[]),dict(id='b',title='Child',parent='a',level=1,target_pdf_page=2,state='manually_confirmed',review_reasons=[])])
 def tearDown(self):self.tmp.cleanup()
 def test_writer_preserves_objects_pixels_text_and_real_destinations(self):
  receipt=export(self.table,self.root/'out.pdf');self.assertEqual(receipt['pages_checked'],3);self.assertEqual(receipt['actual_destinations_checked'],2)
  self.assertEqual(PdfReader(self.source).outline[0].title,'Old')
 def test_writer_refuses_unresolved_collision_and_source_mutation(self):
  t=copy.deepcopy(self.table);t['entries'][0]['review_reasons']=['printed_missing']
  with self.assertRaises(ValueError):export(t,self.root/'x.pdf')
  with self.assertRaises(ValueError):export(self.table,self.source)
  self.source.write_bytes(self.source.read_bytes()+b'\n')
  with self.assertRaisesRegex(ValueError,'source PDF changed'):export(self.table,self.root/'x.pdf')
 def test_edit_add_delete_reparent_retarget_and_confirm(self):
  patch=dict(source_sha256=self.table['source_sha256'],operations=[dict(op='update',id='b',fields=dict(title='Changed',parent=None,target_pdf_page=1)),dict(op='confirm',id='b',evidence_ref='test review'),dict(op='add',entry=dict(id='c',title='New',parent='b',target_pdf_page=2,level=1)),dict(op='confirm',id='c',evidence_ref='test review'),dict(op='delete',id='a')])
  t=edit(self.table,patch);self.assertEqual([e['level'] for e in t['entries']],[0,1]);export(t,self.root/'edited.pdf')
 def test_cycle_and_invalid_page_rejected(self):
  with self.assertRaises(ValueError):edit(self.table,dict(source_sha256=self.table['source_sha256'],operations=[dict(op='update',id='a',fields=dict(parent='b'))]))
  t=copy.deepcopy(self.table);t['entries'][0]['target_pdf_page']=3
  with self.assertRaises(ValueError):validate_table(t)
 def test_mapping_requires_exact_inspected_anchor(self):
  t=copy.deepcopy(self.table);t['entries'][0].update(printed_family='arabic',printed_value=2,target_pdf_page=None,review_reasons=['pagination_uninspected']);t['entries'][1].update(printed_family='arabic',printed_value=3)
  a=[dict(family='arabic',printed_value=2,pdf_page=1,state='inspected',evidence_ref='independent-page-label')]
  self.assertEqual(paginate(t,a)['entries'][0]['target_pdf_page'],1)
  a[0]['state']='uninspected'
  with self.assertRaises(ValueError):paginate(t,a)
 def test_native_glyphs_measured_and_raw_kept(self):
  p=self.root/'native.json';native(str(self.source),[0],p);r=load(p)[0];self.assertTrue(r['glyphs']);e=project(r,p);self.assertEqual(e['lines'][0]['source_kind'],'native_text');self.assertGreater(e['lines'][0]['bbox']['x'],60)
 def test_roman_patterns_no_digit_guessing(self):
  self.assertEqual(number('xi'),('roman',11));self.assertIsNone(number('ioi'));self.assertIsNone(number('1.1.1'));self.assertIsNone(number('IIII'))
 def test_double_column_numbers_do_not_cross_lanes(self):
  raw=dict(source=str(self.source),source_sha256=sha(self.source),page_index=0,page_count=30,width=600,height=800,bbox_convention='visible_top_left_points_xywh',state='locally_recognized',observations=[])
  for col in [0,300]:
   for row in range(3):
    for x,text in [(col+20,f'Title {chr(65+row)}'),(col+250,str(2+row+col//30))]:
     b=[x,100+row*40,30,12];raw['observations'].append(dict(id=f'{col}-{row}-{x}',text=text,bbox=b,tokens=[dict(text=text,bbox=b)],confidence=1))
  e=project(raw,'raw.json');self.assertEqual(len(e['lines']),6);self.assertEqual([l['printed_candidate'] for l in e['lines']],['2','3','4','12','13','14'])
if __name__=='__main__':unittest.main()
