"""Four synthetic mechanism families; zero natural-quality confirmation cases."""
import copy
import json
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

    def test_duplicate_complete_ownership_is_severe_independent_of_title_folio_and_order(self):
        for field,value in [('title','Unrelated source title'),('printed_page','999')]:
            for bad_first in (False,True):
                with self.subTest(field=field,bad_first=bad_first):
                    source=self.source(2);rows=self.rows(source)
                    bad=copy.deepcopy(rows[0]);bad['raw_index']=100;bad[field]=value
                    values=[bad,*rows] if bad_first else [*rows,bad]
                    frozen=copy.deepcopy((source,values));result=decide_complete(values,source,'s','r')
                    self.assertEqual(result['decision'],'abstain');self.assertTrue(result['export_locked'])
                    self.assertEqual(result['known_units'],2);self.assertEqual(result['known_missing'],0)
                    duplicate=[r for r in result['risks'] if r['code']=='duplicate_source_coverage']
                    self.assertEqual(len(duplicate),1)
                    self.assertEqual({duplicate[0]['raw_index'],duplicate[0]['previous_raw_index']},{0,100})
                    self.assertEqual((source,values),frozen)

    def test_duplicate_ownership_with_both_unresolved_titles_or_unknown_source_stays_severe(self):
        for source_unknown in (False,True):
            with self.subTest(source_unknown=source_unknown):
                source=self.source(2);rows=self.rows(source)
                duplicate=copy.deepcopy(rows[0]);duplicate['raw_index']=100
                rows[0]['title']=duplicate['title']='Unrelated source title'
                if source_unknown:
                    source['document']['entries'][0].update(state='UNKNOWN',printed_folio=None)
                result=decide_complete([duplicate,*rows],source,'s','r')
                self.assertEqual(result['decision'],'abstain')
                self.assertEqual(result['known_units'],1 if source_unknown else 2)
                self.assertEqual(result['known_valid'],1);self.assertEqual(result['known_missing'],0)
                self.assertEqual(len(result['source_entry_claims']),2)
                self.assertIn('duplicate_source_coverage',[r['code'] for r in result['risks']])
                # JSON persistence preserves the candidate lock; it never grants
                # a formal writer permission or natural-quality admission.
                loaded=json.loads(json.dumps(result))
                table=dict(entries=[],source_sha256='s',raw_model_sha256='r',revision=0)
                table['admission']=seal_tree(loaded,table)
                with self.assertRaisesRegex(ValueError,'SEVERE_TOC_EXPORT_LOCK'):verify(table)
                self.assertFalse(loaded['natural_safety_verified']);self.assertFalse(loaded['admission_ready'])

    def test_partial_stale_or_unbound_row_cannot_reserve_a_complete_source_claim(self):
        for field,value in [('word_ids',['t-0']),('source_sha256','stale'),('raw_model_sha256','stale')]:
            with self.subTest(field=field):
                source=self.source(2);rows=self.rows(source);bad=copy.deepcopy(rows[0]);bad['raw_index']=100
                bad['source_member_binding'][field]=value
                result=decide_complete([bad,*rows],source,'s','r')
                self.assertEqual(result['decision'],'review');self.assertEqual(result['known_valid'],2)
                self.assertEqual(result['known_missing'],0);self.assertEqual(result['known_coverage_unknown'],0)
                self.assertFalse(any(r['code']=='duplicate_source_coverage' for r in result['risks']))
                self.assertEqual({c['raw_index'] for c in result['source_entry_claims']},{0,1})
        source=self.source(2);rows=self.rows(source);bad=copy.deepcopy(rows[0]);bad['raw_index']=100
        bad.pop('source_member_binding');result=decide_complete([bad,*rows],source,'s','r')
        self.assertEqual(result['decision'],'review');self.assertEqual(result['known_valid'],2)
        self.assertFalse(any(r['code']=='duplicate_source_coverage' for r in result['risks']))

    def test_duplicate_inventory_never_changes_known_missing_denominator_or_config(self):
        source=self.source();rows=self.rows(source)[6:]
        source['document']['entries'] += [dict(entry_id=f'u-{n}',state='UNKNOWN',page=1,word_ids=[f'u-{n}'],
                                               title_literal='unknown',printed_folio=None) for n in range(100)]
        control=decide_complete(rows,source,'s','r')
        duplicates=[]
        for index in range(3):
            row=copy.deepcopy(rows[0]);row['raw_index']=1000+index;row['title']='Unrelated source title';duplicates.append(row)
        for values in [duplicates+rows,rows+duplicates]:
            result=decide_complete(values,source,'s','r')
            self.assertEqual(result['decision'],'abstain');self.assertEqual(result['known_units'],28)
            self.assertEqual(result['known_missing'],6);self.assertAlmostEqual(result['known_missing_fraction'],6/28)
            self.assertEqual(result['config_sha256'],control['config_sha256'])
            self.assertEqual(len([r for r in result['risks'] if r['code']=='duplicate_source_coverage']),3)
            self.assertEqual(len(result['unknown_source_entry_ids']),100)
            self.assertTrue(result['export_locked'])

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
