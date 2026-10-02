import tempfile
import unittest
from pathlib import Path
from .development_panel import classify,qualify,sha


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
        self.assertTrue(result['data_eligible'])
        self.assertFalse(result['residual_on_allowed'])
        self.assertEqual(result['reuse']['state'],'UNVERIFIED')
        self.assertFalse(qualify(units,subset=['0'])['residual_on_allowed'])
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
