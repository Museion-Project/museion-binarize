"""Saved record visibility/security contracts; synthetic PDFs, no recognition."""
import copy
import json
import tempfile
import unittest
from html.parser import HTMLParser
from pathlib import Path

import fitz

from scripts.ocr.app_mvp_bridge.__main__ import view
from . import core, store


def word(member, text='νοῦς', engine='tesseract'):
    return dict(id=member, text=text, engine=engine, bbox=[10, 10, 50, 25], confidence=10)


def snapshot():
    return dict(schema_version=1, revision=0, input_sha256='a'*64,
                pages=[dict(page=1, route='ocr', status='OCR_DRAFT', width=200,
                            height=100, image_sha256='b'*64, image_path='source.png',
                            words=[word('kept')], original_apple=[],
                            independent_reader=[word('kept'), word('lost')],
                            residual_reader=[], decisions=[])], receipts=[])


class ReaderAlternativeTests(unittest.TestCase):
    def test_exact_identity_never_text_or_prefix_matching(self):
        data=snapshot();page=data['pages'][0]
        page['independent_reader'] += [word('kept-longer'), word('same-text')]
        page['words'] += [word('same-text',engine='apple')]
        result=store.reader_alternatives(data)
        self.assertEqual([r['raw_member_id'] for r in result['rows']], ['lost','kept-longer','same-text'])
        self.assertEqual(result['coverage'][0]['raw_records'],4)
        self.assertEqual(result['coverage'][0]['adopted_records'],1)

    def test_no_decision_remains_visible_and_is_not_an_inferred_gap(self):
        row=store.reader_alternatives(snapshot())['rows'][0]
        self.assertEqual(row['decision_status'],'NOT_RECORDED')
        self.assertEqual(row['geometry_status'],'READER_OBSERVATION_UNVERIFIED')
        self.assertTrue(row['read_only']);self.assertNotIn('source_correct',row)

    def test_recorded_decisions_preserve_complete_record(self):
        data=snapshot();decision=dict(state='REVIEW',reader_id='lost',reason='ambiguous_overlap')
        data['pages'][0]['decisions']=[decision,dict(state='REVIEW',reader_id='other')]
        row=store.reader_alternatives(data)['rows'][0]
        self.assertEqual(row['decisions'],[decision]);self.assertEqual(row['decision_status'],'RECORDED')

    def test_page_source_and_image_bound_identity_with_separate_revision(self):
        data=snapshot();second=copy.deepcopy(data['pages'][0]);second['page']=2;data['pages'].append(second)
        result=store.reader_alternatives(data);a,b=result['rows']
        self.assertNotEqual(a['alternative_id'],b['alternative_id'])
        revised=copy.deepcopy(data);revised['revision']=1
        self.assertEqual(store.reader_alternatives(revised)['rows'][0]['alternative_id'],a['alternative_id'])
        self.assertEqual(store.reader_alternatives(revised)['rows'][0]['revision'],1)
        revised['input_sha256']='c'*64
        self.assertNotEqual(store.reader_alternatives(revised)['rows'][0]['alternative_id'],a['alternative_id'])
        revised=copy.deepcopy(data);revised['pages'][0]['image_sha256']='d'*64
        self.assertNotEqual(store.reader_alternatives(revised)['rows'][0]['alternative_id'],a['alternative_id'])

    def test_duplicate_ids_across_streams_are_not_silently_removed(self):
        data=snapshot();data['pages'][0]['residual_reader']=[word('kept'),word('lost')]
        rows=store.reader_alternatives(data)['rows']
        self.assertEqual(len(rows),4);self.assertEqual(len({r['alternative_id'] for r in rows}),4)
        self.assertTrue(all(r['identity_status']=='AMBIGUOUS' for r in rows))

    def test_derivation_and_returned_records_cannot_mutate_raw(self):
        data=snapshot();before=copy.deepcopy(data);result=store.reader_alternatives(data)
        self.assertEqual(data,before)
        result['rows'][0]['raw_record']['text']='changed'
        self.assertEqual(data,before)

    def test_missing_and_empty_raw_streams_are_distinct(self):
        data=snapshot();page=data['pages'][0];page.pop('independent_reader')
        result=store.reader_alternatives(data)
        self.assertEqual(result['rows'],[])
        self.assertEqual(result['coverage'][0]['unavailable_streams'],['independent_reader'])
        page['independent_reader']=[]
        self.assertEqual(store.reader_alternatives(data)['coverage'][0]['unavailable_streams'],[])

    def test_missing_ids_and_nonrecord_raw_are_retained_as_ambiguous(self):
        data=snapshot();data['pages'][0]['independent_reader']=[dict(text='reading'), 'unknown raw']
        rows=store.reader_alternatives(data)['rows'];self.assertEqual(len(rows),2)
        self.assertTrue(all(row['identity_status']=='AMBIGUOUS' for row in rows))
        self.assertEqual(rows[1]['raw_record'],'unknown raw')

    def test_html_escape_and_no_actions_for_raw_alternatives(self):
        data=snapshot();data['pages'][0]['independent_reader'][1]['text']='</pre><input data-id="lost"><script>alert(1)</script>'
        output=store.review_html(data)
        class Inputs(HTMLParser):
            def __init__(self):super().__init__();self.ids=[]
            def handle_starttag(self,tag,attrs):
                if tag=='input':self.ids.append(dict(attrs).get('data-id'))
        parser=Inputs();parser.feed(output)
        self.assertEqual(parser.ids,['kept'])
        self.assertIn('&lt;script&gt;',output);self.assertNotIn('<script>alert(1)',output)

    def operation(self,root):
        source=root/'source.pdf';doc=fitz.open();doc.new_page(width=200,height=100);doc.save(source);doc.close()
        data=snapshot();data.update(source_pdf=str(source),input_sha256=core.sha(source))
        folder=root/('e'*32);folder.mkdir();store.publish(folder/'operation',data)
        meta=dict(mode='local',source=str(source),source_sha256=data['input_sha256'],provenance='synthetic')
        return folder,meta

    def test_bridge_view_uses_verified_snapshot_and_never_writes(self):
        with tempfile.TemporaryDirectory() as temp:
            folder,meta=self.operation(Path(temp))
            before={str(p):core.sha(p) for p in folder.rglob('*') if p.is_file()}
            result=view(folder,meta,{})
            self.assertEqual(result['reader_alternatives']['source_sha256'],meta['source_sha256'])
            self.assertEqual(result['reader_alternatives']['revision'],result['revision'])
            self.assertEqual(result['reader_alternatives']['rows'][0]['raw_member_id'],'lost')
            self.assertFalse(result['quality_ready']);self.assertFalse(result['runtime_compatible'])
            self.assertEqual({str(p):core.sha(p) for p in folder.rglob('*') if p.is_file()},before)
            wrong=dict(meta,source_sha256='c'*64)
            with self.assertRaisesRegex(ValueError,'SNAPSHOT_SOURCE_MISMATCH'):view(folder,wrong,{})
            duplicate=Path(temp)/'duplicate.pdf';duplicate.write_bytes(Path(meta['source']).read_bytes())
            with self.assertRaisesRegex(ValueError,'SNAPSHOT_SOURCE_MISMATCH'):view(folder,dict(meta,source=str(duplicate)),{})

    def test_raw_only_actions_rejected_and_recorded_rejection_survives(self):
        with tempfile.TemporaryDirectory() as temp:
            folder,meta=self.operation(Path(temp));root=folder/'operation'
            before=core.sha(root/'CURRENT.json')
            for action in ('accept','reject','change'):
                with self.assertRaisesRegex(ValueError,'UNKNOWN_MEMBER'):
                    store.review_save(root,dict(expected_revision=0,input_sha256=meta['source_sha256'],actions=[dict(page=1,member_id='lost',action=action,text='changed')]))
                self.assertEqual(core.sha(root/'CURRENT.json'),before)
            store.review_save(root,dict(expected_revision=0,input_sha256=meta['source_sha256'],actions=[dict(page=1,member_id='kept',action='reject')]))
            result=view(folder,meta,{})['reader_alternatives']
            kept=next(r for r in result['rows'] if r['raw_member_id']=='kept')
            self.assertEqual(kept['review_actions'],[dict(revision=1,action='reject')])
            self.assertEqual(kept['raw_record']['text'],'νοῦς')

    def test_bridge_rejects_corrupted_raw_before_exposing_readings(self):
        with tempfile.TemporaryDirectory() as temp:
            folder,meta=self.operation(Path(temp));root=folder/'operation'
            raw=root/'raw.json';raw.write_text('immutable raw')
            data,_=store.load_snapshot(root);data['revision']=1
            data['pages'][0]['raw_files']={'raw.json':core.sha(raw)}
            store.publish(root,data,expected_revision=0);raw.write_text('changed')
            with self.assertRaisesRegex(ValueError,'RAW_EVIDENCE_CORRUPT'):view(folder,meta,{})

    def test_mixed_page_source_is_rejected_even_with_valid_artifact_hashes(self):
        with tempfile.TemporaryDirectory() as temp:
            folder,meta=self.operation(Path(temp));root=folder/'operation'
            data,_=store.load_snapshot(root);data['revision']=1
            data['pages'][0]['source_sha256']='c'*64
            store.publish(root,data,expected_revision=0)
            with self.assertRaisesRegex(ValueError,'SNAPSHOT_SOURCE_MISMATCH'):view(folder,meta,{})


if __name__=='__main__':unittest.main()
