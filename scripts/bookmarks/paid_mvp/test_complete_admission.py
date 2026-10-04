"""Four synthetic mechanism families; zero natural-quality confirmation cases."""
import copy
import unittest
from .complete_admission import decide_complete
from .failure_detection import seal_tree,verify


class CompleteAdmissionTests(unittest.TestCase):
    def source(self,count=28):
        entries=[dict(entry_id=str(n),state='OBSERVED',page=1,word_ids=[f't-{n}',f'p-{n}'],
                      title_literal=f'Topic {n}',printed_folio=dict(literal=str(n+1))) for n in range(count)]
        return dict(source_sha256='s',pages=[dict(page_number=1)],
                    document=dict(entries=entries,ownership=dict(state='PASS'),group_ownership=dict(state='PASS'),
                                  unresolved=[],complete_identity_ready=True))
    def rows(self,source):
        return [dict(raw_index=n,source_page=e['page'],title=e['title_literal'],printed_page=e['printed_folio']['literal'],
                     source_member_binding=dict(entry_id=e['entry_id'],word_ids=e['word_ids'],source_sha256='s',raw_model_sha256='r'))
                for n,e in enumerate(source['document']['entries']) if e['state']=='OBSERVED']

    def test_complete_bound_draft_keeps_quality_and_export_authority_separate(self):
        source=self.source();result=decide_complete(self.rows(source),source,'s','r')
        self.assertEqual(result['decision'],'accept-draft');self.assertEqual(result['known_valid'],28)
        self.assertFalse(result['admission_ready']);self.assertFalse(result['natural_safety_verified'])
        self.assertTrue(result['export_locked']);self.assertEqual(result['export_authority'],'CANDIDATE_ONLY')
        table=dict(entries=[],source_sha256='s',raw_model_sha256='r',revision=0)
        table['admission']=seal_tree(result,table)
        with self.assertRaisesRegex(ValueError,'SEVERE_TOC_EXPORT_LOCK'):verify(table)

    def test_six_missing_of_28_cannot_be_diluted_by_100_unknown_source_entries(self):
        source=self.source();rows=self.rows(source)[6:]
        source['document']['entries'] += [dict(entry_id=f'u-{n}',state='UNKNOWN',page=1,word_ids=[f'u-{n}'],
                                               title_literal='unknown',printed_folio=None) for n in range(100)]
        result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['known_units'],28);self.assertEqual(result['known_missing'],6)
        self.assertAlmostEqual(result['known_missing_fraction'],6/28);self.assertEqual(result['decision'],'abstain')
        for n,e in enumerate(source['document']['entries'][28:]):
            rows.append(dict(raw_index=1000+n,source_page=1,title='unknown',printed_page=None,
                             source_member_binding=dict(entry_id=e['entry_id'],word_ids=e['word_ids'],source_sha256='s',raw_model_sha256='r')))
        result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['known_missing'],6);self.assertAlmostEqual(result['known_missing_fraction'],6/28)
        self.assertEqual(result['decision'],'abstain')
        source=self.source();source['document']['groups']=[dict(group_id='g',entry_ids=['0','1'])]
        result=decide_complete(self.rows(source)[2:],source,'s','r')
        self.assertLess(result['known_missing_fraction'],.2)
        self.assertEqual(result['decision'],'abstain')
        self.assertIn('entire_source_group_missing',[r['code'] for r in result['risks']])

    def test_duplicate_or_wrong_page_member_binding_is_a_severe_lock(self):
        source=self.source();rows=self.rows(source)
        duplicate=copy.deepcopy(rows[0]);duplicate['raw_index']=100
        result=decide_complete(rows+[duplicate],source,'s','r')
        self.assertEqual(result['decision'],'abstain');self.assertTrue(result['export_locked'])
        rows[0]['source_page']=2;result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['decision'],'abstain')
        rows=self.rows(source);rows[1]['continuation_of']=0
        result=decide_complete(rows,source,'s','r')
        self.assertIn('continuation_page_conflict',[r['code'] for r in result['risks']])
        rows=self.rows(source);rows[0]['parent_index']=27;rows[27]['source_page']=2
        result=decide_complete(rows,source,'s','r')
        self.assertIn('source_order_conflict',[r['code'] for r in result['risks']])

    def test_partial_or_stale_binding_cannot_certify_a_known_source_unit(self):
        source=self.source(2);rows=self.rows(source);rows[0]['source_member_binding']['word_ids']=rows[0]['source_member_binding']['word_ids'][:1]
        result=decide_complete(rows,source,'s','r');self.assertEqual(result['known_valid'],1)
        self.assertEqual(result['known_missing'],0);self.assertEqual(result['known_coverage_unknown'],1)
        self.assertEqual(result['decision'],'review')
        rows=self.rows(source);rows[0]['source_member_binding']['raw_model_sha256']='stale'
        self.assertEqual(decide_complete(rows,source,'s','r')['known_valid'],1)

    def test_no_known_denominator_or_incomplete_observer_cannot_auto_accept(self):
        source=self.source(0);result=decide_complete([],source,'s','r')
        self.assertIsNone(result['known_missing_fraction']);self.assertEqual(result['decision'],'review')
        source=self.source();rows=self.rows(source)
        for row in rows:row.pop('source_member_binding')
        result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['known_missing'],0);self.assertEqual(result['known_coverage_unknown'],28)
        self.assertEqual(result['decision'],'review')
        self.assertFalse(any(r['severity']=='severe' for r in result['risks']))
        source=self.source();rows=self.rows(source);source['document']['complete_identity_ready']=False
        self.assertEqual(decide_complete(rows,source,'s','r')['decision'],'review')
        rows[0]['continuation_of']=1000;rows[1]['parent_index']=2000
        result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['decision'],'review');self.assertEqual(len(result['unresolved_predictions']),2)

    def test_reader_unknown_blocks_even_with_stale_complete_flag(self):
        for field,value in [('reader_alternatives',dict(line_id='extra',page=1,text='Other 30')),
                            ('reader_unresolved_members',dict(word_id='extra-word',page=1,reason='region unresolved'))]:
            with self.subTest(field=field):
                source=self.source(2);source['document'][field]=[value]
                result=decide_complete(self.rows(source),source,'s','r')
                self.assertEqual(result['decision'],'review');self.assertTrue(result['export_locked'])
                self.assertEqual(result['known_units'],2);self.assertEqual(result['known_valid'],2)
                self.assertEqual(result['known_missing'],0)
                self.assertIn('independent_reader_coverage_unknown',[r['code'] for r in result['risks']])

    def test_persisted_page_alternatives_survive_missing_old_document_aggregate(self):
        source=self.source(2);extra=dict(line_id='extra',page=1,text='Other 30')
        source['pages'][0]['reader_comparison']=dict(alternative_only=[extra],unresolved=[])
        result=decide_complete(self.rows(source),source,'s','r')
        self.assertEqual(result['decision'],'review');self.assertEqual(result['unknown_reader_alternatives'],[extra])
        source['document']['reader_alternatives']=[copy.deepcopy(extra)]
        result=decide_complete(self.rows(source),source,'s','r')
        self.assertEqual(result['unknown_reader_alternatives'],[extra])
        source['pages'][0]['reader_comparison']['unresolved']=[dict(word_id='u',reason='region missing')]
        self.assertEqual(len(decide_complete(self.rows(source),source,'s','r')['unknown_reader_members']),1)

    def test_reader_unknown_never_dilutes_six_of_28_severe_missing(self):
        source=self.source();rows=self.rows(source)[6:]
        source['document']['reader_alternatives']=[dict(line_id=f'u{n}',page=1,text='unverified') for n in range(100)]
        result=decide_complete(rows,source,'s','r')
        self.assertEqual(result['known_units'],28);self.assertEqual(result['known_missing'],6)
        self.assertAlmostEqual(result['known_missing_fraction'],6/28)
        self.assertEqual(result['decision'],'abstain');self.assertEqual(len(result['unknown_reader_alternatives']),100)
