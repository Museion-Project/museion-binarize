"""Explain existing refusals without changing word adoption or raw evidence."""
import copy
import unittest

from . import core, store


def word(member,text,box,engine='apple',line='row'):
    return dict(id=member,text=text,bbox=box,engine=engine,line_id=line,confidence=95)


class SupportConflictTests(unittest.TestCase):
    def test_exact_contact_is_separate_and_preserves_observed_geometry(self):
        apple=word('a','Latin',[30,10,50,20]);reader=word('t','νοῦς',[10,10,30,20],'tesseract')
        before=copy.deepcopy((apple,reader));words,decisions=core.compose([apple],[reader],[])
        self.assertEqual({w['id'] for w in words},{'a','t'})
        self.assertEqual((apple,reader),before)
        self.assertFalse(any(d.get('reason')=='partial_overlap' for d in decisions))
        self.assertEqual(next(w for w in words if w['id']=='t')['bbox'],reader['bbox'])

    def test_fractional_contact_remains_a_refusal_with_exact_support(self):
        apple=word('a','Latin',[30-0.000001,10,50,20]);reader=word('t','νοῦς',[10,10,30,20],'tesseract')
        before=copy.deepcopy((apple,reader));words,decisions=core.compose([apple],[reader],[])
        self.assertEqual([w['id'] for w in words],['a']);self.assertEqual((apple,reader),before)
        decision=next(d for d in decisions if d.get('reader_id')=='t')
        self.assertEqual(decision['reason'],'partial_overlap')
        conflict=decision['support_conflicts'][0]
        self.assertEqual(conflict['member_id'],'a');self.assertEqual(conflict['bbox'],apple['bbox'])
        self.assertEqual(conflict['source_members'],['a']);self.assertTrue(conflict['same_row'])
        self.assertGreater(conflict['intersection_area'],0);self.assertLess(conflict['intersection_area'],0.001)
        self.assertEqual(conflict['support_status'],'READER_OBSERVATION_UNVERIFIED')

    def test_other_row_intersections_remain_refused_and_all_members_are_visible(self):
        apple=[word('a','Lower',[10,29,25,40]),word('b','row',[27,29,40,40])]
        reader=word('t','κόσμος',[10,10,40,30],'tesseract')
        words,decisions=core.compose(apple,[reader],[])
        self.assertEqual({w['id'] for w in words},{'a','b'})
        conflicts=next(d for d in decisions if d.get('reader_id')=='t')['support_conflicts']
        self.assertEqual([c['member_id'] for c in conflicts],['a','b'])
        self.assertTrue(all(not c['same_row'] for c in conflicts))
        self.assertEqual([c['intersection_area'] for c in conflicts],[15,13])

    def test_selected_reader_support_is_traced_without_duplicate_adoption(self):
        first=word('first','κόσμος',[10,10,40,25],'tesseract')
        second=word('second','νοῦς',[39,10,60,25],'tesseract')
        raw=copy.deepcopy([first,second]);words,decisions=core.compose([],[first,second],[])
        self.assertEqual([w['id'] for w in words],['first']);self.assertEqual([first,second],raw)
        conflict=next(d for d in decisions if d.get('reader_id')=='second')['support_conflicts'][0]
        self.assertEqual((conflict['member_id'],conflict['engine']),('first','tesseract'))
        self.assertEqual(conflict['source_members'],[])
        self.assertEqual(len(core.ownership(words)),1)

    def test_trace_records_do_not_alias_input_or_final_boxes(self):
        apple=word('a','Latin',[29,10,50,20]);reader=word('t','νοῦς',[10,10,30,20],'tesseract')
        before=copy.deepcopy((apple,reader));words,decisions=core.compose([apple],[reader],[])
        conflict=next(d for d in decisions if d.get('reader_id')=='t')['support_conflicts'][0]
        conflict['bbox'][0]=-999;conflict['source_members'].append('invented')
        self.assertEqual((apple,reader),before);self.assertEqual(words[0]['bbox'],apple['bbox'])
        self.assertEqual(words[0]['source_members'],['a'])

    def test_read_only_alternative_retains_trace_without_source_truth_or_new_members(self):
        apple=word('a','Latin',[29,10,50,20]);reader=word('t','νοῦς',[10,10,30,20],'tesseract')
        words,decisions=core.compose([apple],[reader],[])
        page=dict(page=1,image_sha256='b'*64,words=words,original_apple=[apple],independent_reader=[reader],residual_reader=[],decisions=decisions)
        snapshot=dict(schema_version=1,revision=0,input_sha256='a'*64,pages=[page],receipts=[]);before=copy.deepcopy(snapshot)
        result=store.reader_alternatives(snapshot);self.assertEqual(snapshot,before)
        self.assertEqual(len(result['rows']),1);alternative=result['rows'][0]
        self.assertEqual(alternative['decisions'],decisions)
        self.assertEqual(alternative['raw_record'],reader);self.assertTrue(alternative['read_only'])
        self.assertEqual(alternative['geometry_status'],'READER_OBSERVATION_UNVERIFIED')
        self.assertNotIn('source_correct',alternative);self.assertEqual([w['id'] for w in page['words']],['a'])


class UnadoptedPolicyTraceTests(unittest.TestCase):
    def test_concatenated_latin_support_retains_all_words_and_each_reading_reason(self):
        apple=word('a','thecommunity',[10,10,100,30])
        reader=[word('t1','the',[12,12,30,28],'tesseract'),word('t2','“community',[36,12,99,28],'tesseract')]
        before=copy.deepcopy((apple,reader));words,decisions=core.compose([apple],reader,[])
        self.assertEqual([w['text'] for w in words],['thecommunity']);self.assertEqual((apple,reader),before)
        self.assertEqual([d['reader_id'] for d in decisions],['t1','t2'])
        for d in decisions:
            self.assertEqual(d['state'],'NOT_ADOPTED');self.assertEqual(d['reason'],'covered_reader_policy_not_adopted')
            self.assertFalse(d['admission_observation']['greek_admission_supported'])
            self.assertEqual(d['support_conflicts'][0]['member_id'],'a')
            self.assertEqual(d['support_conflicts'][0]['support_status'],'READER_OBSERVATION_UNVERIFIED')
        self.assertEqual(core.ownership(words)[0]['source_members'],['a'])

    def test_agreeing_latin_alternative_is_not_labelled_an_error_or_omission(self):
        apple=word('a','the',[10,10,40,30]);reader=word('t','the',[10,10,40,30],'tesseract')
        words,decisions=core.compose([apple],[reader],[])
        self.assertEqual([w['id'] for w in words],['a'])
        self.assertEqual(decisions[0]['state'],'NOT_ADOPTED')
        self.assertNotIn('source_correct',decisions[0]);self.assertNotIn('complete_gap',decisions[0])

    def test_low_confidence_covered_greek_records_the_existing_policy(self):
        apple=word('a','word',[10,10,60,30]);reader=word('t','κόσμος',[10,10,60,30],'tesseract');reader['confidence']=9
        words,decisions=core.compose([apple],[reader],[])
        self.assertEqual([w['id'] for w in words],['a'])
        self.assertEqual(decisions[0]['reason'],'covered_reader_policy_not_adopted')
        self.assertFalse(decisions[0]['admission_observation']['greek_admission_supported'])
        self.assertEqual(decisions[0]['admission_observation']['confidence'],9)

    def test_low_confidence_uncovered_latin_remains_unadopted(self):
        reader=word('t','the',[10,10,40,30],'tesseract');reader['confidence']=64
        words,decisions=core.compose([],[reader],[])
        self.assertEqual(words,[]);self.assertEqual(decisions[0]['reason'],'reader_admission_not_supported')
        self.assertEqual(decisions[0]['support_conflicts'],[])

    def test_uncovered_punctuation_without_base_is_not_given_a_word(self):
        reader=word('t','…',[10,10,40,30],'tesseract')
        words,decisions=core.compose([],[reader],[])
        self.assertEqual(words,[]);self.assertFalse(decisions[0]['admission_observation']['has_base_text'])
        self.assertEqual(decisions[0]['reason'],'reader_admission_not_supported')

    def test_existing_uncovered_adoption_is_not_overridden_by_diagnostic(self):
        reader=word('t','the',[10,10,40,30],'tesseract');reader['confidence']=65
        words,decisions=core.compose([],[reader],[])
        self.assertEqual([w['id'] for w in words],['t'])
        self.assertEqual([d['state'] for d in decisions],['RESIDUAL_DRAFT'])
        self.assertEqual(len(core.ownership(words)),1)

    def test_existing_covered_greek_adoption_has_no_duplicate_diagnostic(self):
        apple=word('a','word',[10,10,60,30]);reader=word('t','κόσμος',[10,10,60,30],'tesseract')
        words,decisions=core.compose([apple],[reader],[])
        self.assertEqual([w['id'] for w in words],['t'])
        self.assertEqual([d['state'] for d in decisions],['GREEK_DRAFT'])
        self.assertEqual(words[0]['source_members'],['a'])

    def test_residual_policy_record_is_preserved_without_adoption(self):
        reader=word('r','the',[10,10,40,30],'tesseract');reader['confidence']=64;reader['residual_member']='res-original'
        words,decisions=core.compose([],[],[reader])
        self.assertEqual(words,[]);self.assertEqual(decisions[0]['reader_id'],'r')
        self.assertEqual(decisions[0]['state'],'NOT_ADOPTED')

    def test_trace_reaches_readonly_alternative_and_does_not_alias_evidence(self):
        apple=word('a','thecommunity',[10,10,100,30]);reader=word('t','the',[12,12,30,28],'tesseract')
        before=copy.deepcopy((apple,reader));words,decisions=core.compose([apple],[reader],[])
        page=dict(page=1,image_sha256='b'*64,words=words,original_apple=[apple],independent_reader=[reader],residual_reader=[],decisions=decisions)
        snapshot=dict(schema_version=1,revision=0,input_sha256='a'*64,pages=[page],receipts=[]);frozen=copy.deepcopy(snapshot)
        alternative=store.reader_alternatives(snapshot)['rows'][0]
        self.assertEqual(snapshot,frozen);self.assertTrue(alternative['read_only'])
        self.assertEqual(alternative['decision_status'],'RECORDED');self.assertEqual(alternative['decisions'],decisions)
        alternative['decisions'][0]['support_conflicts'][0]['bbox'][0]=-1
        self.assertEqual(snapshot,frozen)
        decisions[0]['support_conflicts'][0]['bbox'][0]=-2
        self.assertEqual((apple,reader),before);self.assertEqual(words[0]['bbox'],apple['bbox'])


if __name__=='__main__':unittest.main()
