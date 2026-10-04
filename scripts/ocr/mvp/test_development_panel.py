import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from .development_panel import classify,complete_greek_word,qualify,sha


class DevelopmentPanelTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.crop=Path(self.temp.name)/'crop';self.crop.write_bytes(b'source')
    def tearDown(self):self.temp.cleanup()
    def unit(self,identity='u',book='b',structure='body'):
        return dict(unit_id=identity,book=book,structure=structure,page=1,crop=str(self.crop),crop_sha256=sha(self.crop),
                    pg_scorable=True,source_visual_transcription='the the missing',genuine_lexical_gap=True,
                    baseline_members=dict(independent_reader=[dict(text='the'),dict(text='the')],words=[dict(text='the')]))
    def test_raw_complete_is_not_residual_gain(self):
        unit=self.unit();unit['baseline_members']['independent_reader'].append(dict(text='missing'))
        self.assertEqual(classify(unit)['state'],'PROJECTION_ONLY')
    def test_repeated_source_words_require_multiplicity(self):
        unit=self.unit();unit['source_visual_transcription']='the the'
        unit['baseline_members']['independent_reader']=[dict(text='the')]
        self.assertEqual(classify(unit)['missing'],['the'])
    def test_frozen_subset_itself_must_qualify(self):
        units=[self.unit(str(n),str(n%2),'footnote' if n%2 else 'body') for n in range(12)]
        result=qualify(units,subset=[u['unit_id'] for u in units])
        self.assertFalse(result['data_eligible'])
        self.assertFalse(result['residual_on_allowed'])
        self.assertEqual(result['reuse']['state'],'UNVERIFIED')
        self.assertFalse(qualify(units,subset=['0'])['residual_on_allowed'])
    def greek_units(self, count):
        units=[]
        for n in range(count):
            u=self.unit(str(n),str(n%2),'footnote' if n%2 else 'body')
            u.update(bbox=[n*3,0,n*3+2,2],source_visual_transcription='λόγος',
                     source_literal='λόγος',complete_printed_word=True,
                     baseline_members=dict(independent_reader=[],words=[]))
            if n>=12:
                u['baseline_members']=dict(independent_reader=[dict(text='λόγος')],words=[dict(text='λόγος')])
            units.append(u)
        return units
    def test_verified_reuse_cannot_bypass_english_or_99_greek_words(self):
        panels=[[self.unit(str(n),str(n%2),'footnote' if n%2 else 'body') for n in range(12)],self.greek_units(99)]
        with patch('scripts.ocr.mvp.development_panel.verify_reuse',return_value=dict(state='VERIFIED',reasons=[])):
            for units in panels:
                result=qualify(units,subset=[u['unit_id'] for u in units])
                self.assertFalse(result['data_eligible']);self.assertFalse(result['residual_on_allowed'])
                self.assertLess(result['Greek_complete_word_count'],100)
    def test_exactly_100_source_words_preserve_repeats_and_reuse_gate(self):
        units=self.greek_units(100);result=qualify(units,subset=[u['unit_id'] for u in units])
        self.assertTrue(result['data_eligible']);self.assertFalse(result['residual_on_allowed'])
        self.assertEqual(result['Greek_complete_word_count'],100)
        self.assertEqual(result['Greek_complete_word_unit_ids'],[u['unit_id'] for u in units])
        with patch('scripts.ocr.mvp.development_panel.verify_reuse',return_value=dict(state='VERIFIED',reasons=[])):
            self.assertTrue(qualify(units,subset=[u['unit_id'] for u in units])['residual_on_allowed'])
    def test_pool_words_outside_frozen_subset_do_not_count(self):
        units=self.greek_units(100);result=qualify(units,subset=[u['unit_id'] for u in units[:99]])
        self.assertFalse(result['data_eligible']);self.assertEqual(result['Greek_complete_word_count'],99)
    def test_unknown_mixed_fragment_and_unconfirmed_words_do_not_count(self):
        units=self.greek_units(100)
        units[0]['genuine_lexical_gap']=False
        for n,text in [(1,'Greekλόγος'),(2,'λό-'),(3,'λόγος λόγος'),(4,'λόγοςʹ')]:
            units[n]['source_literal']=units[n]['source_visual_transcription']=text
        units[5]['complete_printed_word']=False
        units[6]['source_literal']='ἄλλος'
        result=qualify(units,subset=[u['unit_id'] for u in units])
        self.assertEqual(result['Greek_complete_word_count'],93);self.assertFalse(result['data_eligible'])
    def test_greek_marks_and_punctuation_keep_literal_bytes(self):
        for text in ['λόγος,','λόγος','“ἄνθρωπος”',"ἀλλ’"]:
            self.assertTrue(complete_greek_word(text),text)
        for text in ['·','\u0301λόγος','123','λόγος1','ἀλλ-','Latinλόγος','λόγος λόγος','αʹ','λόγος\u05b0']:
            self.assertFalse(complete_greek_word(text),text)
    def test_new_crop_hash_cannot_duplicate_a_physical_word(self):
        first,second=self.greek_units(2);second.update(book=first['book'],bbox=first['bbox'],crop_sha256='different')
        with self.assertRaisesRegex(ValueError,'DUPLICATE_PHYSICAL_SOURCE_UNIT'):
            qualify([first,second])
    def test_no_subset_or_second_structure_blocks(self):
        units=[self.unit(str(n),str(n%2)) for n in range(12)]
        self.assertFalse(qualify(units)['residual_on_allowed'])
        self.assertFalse(qualify(units,subset=[u['unit_id'] for u in units])['residual_on_allowed'])
    def test_reference_label_without_raw_remains_unknown(self):
        unit=self.unit();unit.pop('baseline_members')
        self.assertEqual(classify(unit)['state'],'UNKNOWN')
    def test_duplicate_units_and_changed_source_fail(self):
        with self.assertRaisesRegex(ValueError,'DUPLICATE_SOURCE_UNIT'):qualify([self.unit(),self.unit()])
        unit=self.unit();self.crop.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'SOURCE_CROP_CHANGED'):classify(unit)


if __name__=='__main__':unittest.main()
