import copy
import unittest
from pagination import build_model, map_table, target_candidates, sample_pages, parsed_folio, candidates_from_words


def observation(index,value,family='arabic',side='single',explicit=False):
    return dict(pdf_page=index,printed_value=value,family=family,side=side,confidence=1,explicit=explicit,evidence_ref=f'folio:{index}:{value}',kind='native_margin',bbox=[10,10,10,10])


def table(labels):
    entries=[dict(id=str(i),title=f'Chapter {i}',printed_family=f,printed_value=v,printed_page=str(v),source_page=0,review_reasons=['pagination_uninspected'],target_pdf_page=None) for i,(f,v) in enumerate(labels)]
    return dict(page_count=160,source_sha256='source',entries=entries)


class PaginationTests(unittest.TestCase):
    def test_contents_running_header_is_not_an_independent_folio(self):
        words=[(10,10,20,20,'vi',0,0,0),(120,10,160,20,'Contents',1,0,0)]
        self.assertEqual(candidates_from_words(words,400,600,6,'native_margin','page6'),[])

    def test_two_consistent_pages_establish_rule_without_fifteen_anchors(self):
        model=build_model([observation(19,10),observation(49,40)],100)
        self.assertEqual(model['status'],'ready')
        self.assertEqual(model['rules'][0]['anchor_count'],2)
        self.assertEqual(target_candidates(model,'arabic',1)[0]['pdf_page'],10)

    def test_roman_and_arabic_are_independent_sequences(self):
        model=build_model([observation(4,3,'roman'),observation(6,5,'roman'),observation(35,17),observation(65,47)],100)
        self.assertEqual(model['sequence_count'],2)
        self.assertEqual(target_candidates(model,'roman',6)[0]['pdf_page'],7)
        self.assertEqual(target_candidates(model,'arabic',1)[0]['pdf_page'],19)

    def test_restarted_arabic_is_disambiguated_by_complete_contents_order(self):
        model=build_model([observation(19,10),observation(39,30),observation(109,10),observation(129,30)],160)
        model['source_sha256']='source'
        mapped=map_table(table([('arabic',1),('arabic',20),('arabic',1),('arabic',20)]),model)
        self.assertEqual([e['target_pdf_page'] for e in mapped['entries']],[10,29,100,119])

    def test_duplicate_number_without_disambiguating_context_keeps_candidates(self):
        model=build_model([observation(19,10),observation(39,30),observation(109,10),observation(129,30)],160)
        model['source_sha256']='source'
        entry=map_table(table([('arabic',10)]),model)['entries'][0]
        self.assertIsNone(entry['target_pdf_page'])
        self.assertEqual([c['pdf_page'] for c in entry['pagination_candidates']],[19,109])
        self.assertIn('pagination_ambiguous',entry['review_reasons'])

    def test_inserted_page_leaves_boundary_uncertain_instead_of_wrong_offset(self):
        model=build_model([observation(10,1),observation(20,11),observation(25,15),observation(35,25)],60)
        self.assertEqual(model['status'],'partial')
        self.assertEqual(target_candidates(model,'arabic',14),[])

    def test_two_up_spread_maps_both_printed_pages_to_one_physical_page(self):
        obs=[observation(4,10,side='left'),observation(4,11,side='right'),observation(9,20,side='left'),observation(9,21,side='right')]
        model=build_model(obs,50)
        self.assertTrue(model['spread'])
        self.assertEqual(target_candidates(model,'arabic',12)[0]['pdf_page'],5)
        self.assertEqual(target_candidates(model,'arabic',13)[0]['pdf_page'],5)

    def test_reflowed_page_exact_observation_wins_over_extrapolation(self):
        model=build_model([observation(15,1,explicit=True),observation(17,2,explicit=True),observation(19,3,explicit=True),observation(39,13,explicit=True),observation(40,14,explicit=True)],100)
        self.assertEqual(target_candidates(model,'arabic',1)[0]['pdf_page'],15)
        self.assertEqual(target_candidates(model,'arabic',2)[0]['pdf_page'],17)

    def test_constant_chapter_number_does_not_establish_pagination(self):
        model=build_model([observation(5,3),observation(15,3),observation(25,3)],50)
        self.assertFalse(model['rules'])
        self.assertEqual(model['status'],'unavailable')

    def test_parenthesized_note_number_and_attached_note_marker_are_not_folios(self):
        self.assertIsNone(parsed_folio('(12)'))
        words=[(10,920,15,930,'12',0,0,0),(18,920,100,930,'Footnote',1,0,0)]
        self.assertEqual(candidates_from_words(words,600,1000,2,'native','test'),[])

    def test_toc_page_cannot_be_its_own_witness_and_source_mismatch_is_rejected(self):
        t=table([('arabic',10)])
        model=build_model([observation(0,10,explicit=True)],160);model['source_sha256']='source'
        self.assertIsNone(map_table(t,model)['entries'][0]['target_pdf_page'])
        other=copy.deepcopy(model);other['source_sha256']='other'
        with self.assertRaises(ValueError):map_table(t,other)

    def test_sampling_is_small_reproducible_and_in_bounds(self):
        for n in [1,5,18,500,2000]:
            a=sample_pages(n,123);self.assertEqual(a,sample_pages(n,123));self.assertLessEqual(len(a),8)
            self.assertEqual(a,sorted(set(a)));self.assertTrue(all(0<=p<n for p in a))

if __name__=='__main__':unittest.main()
