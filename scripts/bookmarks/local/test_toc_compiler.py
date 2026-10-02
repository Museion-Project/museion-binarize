import unittest
from bookmarks import project
class GroupingContract(unittest.TestCase):
 def raw(self,rows):
  return dict(id='test',source='x.pdf',source_sha256='x',page_index=0,page_count=100,width=600,height=800,bbox_convention='visible_top_left_points_xywh',state='locally_recognized',source_kind='native_text',observations=[dict(id=str(i),text=t,bbox=[x,y,w,12],tokens=[dict(text=t,bbox=[x,y,w,12])],confidence=1) for i,(t,x,y,w) in enumerate(rows)])
 def test_page_on_first_line_allows_obvious_short_continuation(self):
  r=self.raw([('The History of Natural Philosophy',70,100,300),('ix',520,100,15),('Monograph Series',70,120,110),('Preface',70,160,90),('xi',520,160,15)])
  e=project(r,'raw');self.assertEqual(len(e['lines']),2);self.assertIn('Monograph Series',e['lines'][0]['text']);self.assertEqual(e['lines'][0]['printed_candidate'],'ix')
 def test_missing_page_does_not_swallow_new_label(self):
  r=self.raw([('A. First subject',70,100,180),('B. Second subject',70,120,180),('86',520,120,15),('C. Third subject',70,140,180),('98',520,140,15)])
  e=project(r,'raw');self.assertEqual(len(e['lines']),3);self.assertIsNone(e['lines'][0]['printed_candidate'])
 def test_paged_entry_does_not_swallow_named_section(self):
  r=self.raw([('First subject',70,100,280),('10',520,100,15),('Notes',70,120,80),('20',520,120,15)])
  self.assertEqual(len(project(r,'raw')['lines']),2)
 def test_reread_near_neighbor_is_routed_by_measured_position(self):
  r=self.raw([('1.4. A very long title',70,100,250),('continued on next visual line',75,116,220),('84',520,116,15),('A. Separate title',70,140,180),('86',520,140,15)])
  reads={'0':dict(candidate=dict(text='84',bbox=[520,116,15,12],confidence=1),evidence_ref='reread0')}
  e=project(r,'raw',reads);self.assertEqual(len(e['lines']),2);self.assertIn('continued',e['lines'][0]['text']);self.assertEqual(e['lines'][0]['text'].count('84'),1)
if __name__=='__main__':unittest.main()

class AuthorNamedTitle(unittest.TestCase):
 def test_author_and_introduction_stay_one_entry(self):
  helper=GroupingContract();r=helper.raw([('JOHN DOE',70,100,100),('Introduction',70,120,130),('1',520,120,15),('2. Other chapter',70,170,200),('10',520,170,15)])
  e=project(r,'raw');self.assertEqual(len(e['lines']),2);self.assertIn('JOHN DOE Introduction',e['lines'][0]['text'])

class TitleGeometryContract(unittest.TestCase):
 def test_number_width_does_not_change_title_indent(self):
  from bookmarks import title_geometry
  for prefix,x,w in [('9.',70,12),('10.',64,18)]:
   o={'tokens':[{'text':prefix,'bbox':[x,100,w,12]},{'text':'Title','bbox':[90,100,80,12]}]}
   self.assertEqual(title_geometry(o)[0],90)
 def test_unsplit_token_does_not_invent_substring_geometry(self):
  from bookmarks import title_geometry
  o={'tokens':[{'text':'1.Title','bbox':[70,100,90,12]}]}
  self.assertIsNone(title_geometry(o)[1])
