import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import fitz
from .complete_source import complete_entries,observe_complete,parse_tsv,TSV_FIELDS
from .source_identity import bind_words,physical_lines,compare_readers


class CompleteSourceTests(unittest.TestCase):
 def layout(self,word_rows):
  raw=[dict(text=text,bbox=box,id=i) for i,(text,box) in enumerate(word_rows)]
  words=bind_words(raw,source_sha256='s',image_sha256='i',page=1,reader='native')
  zones=[dict(id='left',bbox=[0,0,100,200],folio_bbox=[75,0,100,200],page=1,source_sha256='s',image_sha256='i')]
  return physical_lines(words,zones),zones
 def relation(self,lines,index=0,kind='continuation'):
  return dict(kind=kind,**{'from':lines[index]['line_id'],'to':lines[index+1]['line_id']},
              evidence=dict(basis='source-pixels',source_sha256='s',image_sha256='i',note='source rows visibly continue'))
 def test_numeric_title_and_logic_topics_not_deleted(self):
  result,zones=self.layout([('12',[5,10,15,18]),('Logic',[20,10,45,18]),('Topics',[49,10,70,18]),('28',[85,10,96,18])])
  out=complete_entries(result,zones);self.assertEqual(out['entries'][0]['text'],'12 Logic Topics 28');self.assertTrue(out['complete_identity_ready'])
 def test_unique_continuation_owns_every_word_once(self):
  result,zones=self.layout([('Long',[5,10,40,18]),('title',[5,21,30,29]),('28',[85,21,96,29])])
  out=complete_entries(result,zones,[self.relation(result['lines'])]);self.assertEqual(len(out['entries']),1);self.assertEqual(out['ownership']['state'],'PASS');self.assertTrue(out['complete_identity_ready'])
 def test_no_proof_leaves_continuation_unknown(self):
  result,zones=self.layout([('Long',[5,10,40,18]),('title',[5,21,30,29]),('28',[85,21,96,29])])
  rel=self.relation(result['lines']);rel['evidence']['image_sha256']='wrong'
  out=complete_entries(result,zones,[rel]);self.assertEqual(len(out['entries']),2);self.assertFalse(out['complete_identity_ready']);self.assertEqual(len(out['rejected_relations']),1)
 def test_duplicate_relationship_is_not_silently_chosen(self):
  result,zones=self.layout([('Long',[5,10,40,18]),('title',[5,21,30,29]),('28',[85,21,96,29])]);rel=self.relation(result['lines'])
  out=complete_entries(result,zones,[rel,rel]);self.assertEqual(len(out['rejected_relations']),2);self.assertEqual(out['ownership']['state'],'PASS')
 def test_detached_folio_requires_unique_relation(self):
  result,zones=self.layout([('Topics',[5,10,40,18]),('28',[85,21,96,29])])
  out=complete_entries(result,zones);self.assertTrue(all(e['state']=='UNKNOWN' for e in out['entries']))
  out=complete_entries(result,zones,[self.relation(result['lines'],kind='folio')]);self.assertTrue(out['complete_identity_ready'])
 def test_author_after_folio_retained_and_roman_I_literal_unknown(self):
  result,zones=self.layout([('Topics',[5,10,40,18]),('I',[85,10,89,18]),('AUTHOR',[15,21,60,29])])
  out=complete_entries(result,zones,[self.relation(result['lines'],kind='author')]);entry=out['entries'][0]
  self.assertEqual(entry['printed_folio']['literal'],'I');self.assertIsNone(entry['printed_folio']['semantic']);self.assertIn('AUTHOR',entry['text']);self.assertEqual(entry['state'],'UNKNOWN')
  self.assertEqual(entry['title_literal'],'Topics');self.assertEqual(entry['authors_literal'],['AUTHOR'])
 def test_reader_disagreement_and_alternatives_never_inflate_entry_count(self):
  result,zones=self.layout([('Topics',[5,10,40,18]),('28',[85,10,96,18])]);lines=result['lines'];comparison=compare_readers(lines,[dict(lines[0],text='Other 29')])
  out=complete_entries(result,zones,reader_comparison=comparison);self.assertEqual(len(out['entries']),1);self.assertFalse(out['complete_identity_ready']);self.assertEqual(out['group_ownership']['state'],'PASS')
 def test_native_observer_verifies_source_pixels_and_tamper_fails(self):
  with tempfile.TemporaryDirectory() as directory:
   root=Path(directory);source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=200).insert_text((15,30),'Logic');doc.save(source);doc.close()
   with fitz.open(source) as doc:image=doc[0].get_pixmap(matrix=fitz.Matrix(2,2),alpha=False).tobytes('png')
   path=root/'source.png';path.write_bytes(image);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
   req=dict(input_pdf=str(source),input_sha256=sha(source),images=[dict(page_number=1,path=str(path),sha256=sha(path),kind='page',dpi=144)])
   layout=dict(page=1,source_sha256=sha(source),image_sha256=sha(path),basis='source-pixels',evidence_path=str(path),evidence_sha256=sha(path),regions=[dict(id='one',bbox=[0,0,200,200])])
   with patch('subprocess.run') as call:
    out=observe_complete(req,layouts=[layout]);call.assert_not_called()
   self.assertEqual(out['pages'][0]['ownership']['state'],'PASS');self.assertEqual(out['new_reader_calls'],0)
   self.assertEqual(out['pages'][0]['words'][0]['text'],'Logic')
   wrong=fitz.open();wrong.new_page(width=200,height=200).insert_text((15,30),'Changed');wrong.save(root/'other.pdf');wrong.close()
   req['input_pdf']=str(root/'other.pdf');req['input_sha256']=sha(root/'other.pdf')
   with self.assertRaisesRegex(ValueError,'SOURCE_PAGE_PIXELS_CHANGED'):observe_complete(req,layouts=[])
 def test_tsv_literal_quotes_and_reject_bad_raw(self):
  raw='\t'.join(TSV_FIELDS)+'\n5\t1\t1\t1\t1\t1\t10\t10\t20\t8\t99\t"Topics"\n'
  self.assertEqual(parse_tsv(raw)[0]['text'],'"Topics"')
  with self.assertRaisesRegex(ValueError,'INVALID_TSV_FIELDS'):parse_tsv(raw+'bad\n')
 def test_header_words_are_owned_by_group_and_never_deleted(self):
  result,zones=self.layout([('CONTENTS',[20,1,60,9]),('Topics',[5,30,40,38]),('28',[85,30,96,38])])
  role=dict(line_id=result['lines'][0]['line_id'],role='header',evidence=self.relation(result['lines'])['evidence'])
  out=complete_entries(result,zones,line_roles=[role]);self.assertEqual(len(out['entries']),1)
  self.assertEqual(out['groups'][0]['headers'][0]['text'],'CONTENTS');self.assertEqual(out['ownership']['total_members'],3);self.assertEqual(out['ownership']['state'],'PASS')
 def test_eval_expected_text_cannot_resolve_ambiguous_source_ownership(self):
  from .reconcile_source_entries import reconcile
  result,zones=self.layout([('Topics',[5,10,40,18]),('28',[85,10,96,18])]);out=complete_entries(result,zones)
  candidate=dict(source_sha256='s',pages=[dict(page_number=1,image_sha256='i',**out)])
  unit=dict(unit_id='u',bbox_pixels=[0,0,100,30],title_literal='Topics',authors_literal=[],printed_folio_literal='28')
  ledger=dict(source_sha256='s',image_sha256='i',page=1,image_dpi=72,units=[unit])
  self.assertEqual(reconcile(candidate,ledger)['one_to_one_complete_members'],1)
  ledger['units'].append(dict(unit,unit_id='v',title_literal='Other expected answer'))
  self.assertEqual(reconcile(candidate,ledger)['one_to_one_complete_members'],0)


if __name__=='__main__':unittest.main()
