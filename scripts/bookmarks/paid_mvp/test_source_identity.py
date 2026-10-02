import unittest
from .source_identity import bind_words,physical_lines,ownership,compare_readers,entry_identity


class SourceIdentityTests(unittest.TestCase):
    def words(self):
        return bind_words([dict(text='Logic',bbox=[10,10,40,20]),dict(text='28',bbox=[70,10,80,20]),
                           dict(text='Topics',bbox=[110,10,145,20]),dict(text='I',bbox=[170,10,180,20])],
                          source_sha256='s',image_sha256='i',page=1,reader='native')
    def regions(self):
        return [dict(id='left',bbox=[0,0,90,100],page=1,source_sha256='s',image_sha256='i'),
                dict(id='right',bbox=[100,0,190,100],page=1,source_sha256='s',image_sha256='i')]
    def test_same_baseline_columns_never_join(self):
        result=physical_lines(self.words(),self.regions())
        self.assertEqual([l['text'] for l in result['lines']],['Logic 28','Topics I'])
        self.assertTrue(result['identity_ready'])
    def test_no_columns_is_explicitly_unresolved(self):
        result=physical_lines(self.words(),[])
        self.assertEqual(len(result['unresolved']),4)
        self.assertEqual(result['ownership']['state'],'PASS')
        self.assertFalse(result['identity_ready'])
    def test_counter_catches_duplicate_members_despite_set_equality(self):
        result=ownership(['a','b'],[['a','b'],['b']])
        self.assertEqual(result['state'],'FAIL')
        self.assertEqual(result['duplicate_or_unknown'],['b'])
    def test_alternative_reader_does_not_increase_formal_denominator(self):
        lines=physical_lines(self.words(),self.regions())['lines']
        result=compare_readers(lines,lines+[dict(lines[0],line_id='extra')])
        self.assertFalse(result['adds_formal_entries'])
        self.assertEqual(result['primary'][0]['state'],'UNKNOWN')
    def test_title_revision_does_not_change_entry_identity(self):
        lines=physical_lines(self.words(),self.regions())['lines']
        revised=[dict(lines[0],text='Corrected Logic 28')]
        self.assertEqual(entry_identity(lines[:1]),entry_identity(revised))
    def test_literal_roman_is_preserved(self):
        self.assertEqual(self.words()[-1]['text'],'I')
        self.assertEqual(physical_lines(self.words(),self.regions())['lines'][1]['text'],'Topics I')
    def test_same_geometry_from_other_image_cannot_be_reader_agreement(self):
        import copy
        lines=physical_lines(self.words(),self.regions())['lines'];alternate=copy.deepcopy(lines)
        for line in alternate:
            for word in line['words']:word['identity']['image_sha256']='different-source-image'
        result=compare_readers(lines,alternate)
        self.assertTrue(all(r['state']=='UNKNOWN' for r in result['primary']))
        self.assertEqual(len(result['alternative_only']),len(alternate))
    def test_unknown_input_or_duplicate_entry_rejected(self):
        self.assertEqual(ownership(['a'],[['x']])['state'],'FAIL')
        line=physical_lines(self.words(),self.regions())['lines'][0]
        with self.assertRaisesRegex(ValueError,'DUPLICATE_ENTRY_WORD'):
            entry_identity([line,line])

    def test_source_page_hash_and_reader_duplicates_do_not_merge(self):
        regions=[dict(r,page=2) for r in self.regions()]
        self.assertFalse(physical_lines(self.words(),regions)['identity_ready'])
        changed=[dict(r,image_sha256='other') for r in self.regions()]
        self.assertFalse(physical_lines(self.words(),changed)['identity_ready'])
        w=dict(text='x',bbox=[0,0,10,10],source_word_id=0)
        with self.assertRaisesRegex(ValueError,'DUPLICATE_READER_MEMBER'):
            bind_words([w,w],source_sha256='s',image_sha256='i',page=1,reader='native')


if __name__=='__main__':unittest.main()
