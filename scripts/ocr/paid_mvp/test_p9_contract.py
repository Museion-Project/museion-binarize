"""Synthetic only: provider shape/identity, pixel ownership and private state."""
import copy
import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from PIL import Image, ImageDraw
from .pipeline import BoundaryError, canonical, digest, observations, image_url
from .repair_units import VERSION, build, prove_lines
from .selector import select
from .selective import base_run, durable_request, mistral_manifest, request_shape
from .state import directory


class P9ContractTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.image = self.root / 'image.png'
        image = Image.new('RGB', (200, 500), 'white')
        ImageDraw.Draw(image).rectangle((25, 105, 90, 118), fill='black')
        image.save(self.image)
        self.response = dict(model='mistral-ocr-4-1', usage_info=dict(pages_processed=1), pages=[
            dict(index=0, markdown='Low confidence line', dimensions=dict(width=200, height=500),
                 blocks=[dict(type='text', content='Low confidence line', top_left_x=20,
                              top_left_y=100, bottom_right_x=100, bottom_right_y=125)],
                 confidence_scores=dict(average_page_confidence_score=.9,
                                        minimum_page_confidence_score=.1,
                                        word_confidence_scores=[
                                            dict(text='Low', start_index=0, confidence=.99),
                                            dict(text='confidence', start_index=4, confidence=.1),
                                            dict(text='line', start_index=15, confidence=.99)]))])
        self.ledger = self.root / 'shared-state'

    def tearDown(self):
        self.temp.cleanup()

    def page(self, response=None):
        return observations(response or self.response, digest(self.image), 1,
                            expected_model='mistral-ocr-4-1', expected_index=0,
                            expected_dimensions=[200, 500])

    def units(self, response=None):
        page = self.page(response)
        prove_lines(page, self.image)
        return page, build(page, 'synthetic-source', 'synthetic-response', 'round3')

    def task(self, count=1):
        import fitz
        pdf = self.root / 'source.pdf'
        with fitz.open() as doc:
            for _ in range(count):
                doc.new_page(width=200, height=500)
            doc.save(pdf)
        return dict(mode='paid', operation_id='synthetic-p9', input_pdf=str(pdf),
                    input_sha256=digest(pdf), page_numbers=list(range(1, count + 1)),
                    images=[dict(path=str(self.image), sha256=digest(self.image)) for _ in range(count)],
                    output_directory=str(self.root / 'operation'), config_version='round3',
                    region_protocol=VERSION)

    def approval(self, manifest):
        return dict(manifest_seal=manifest['seal'], combined_reservation_reconciled=True,
                    mistral_free_only=True, historical_spent_usd=.90, historical_unsettled_usd=.008,
                    combined_new_reserve_usd=manifest['worst_case_reservation_usd'], cap_usd=1,
                    approved_request_ids=[c['request_id'] for c in manifest['calls']],
                    approved_images=manifest['calls'], state_root=str(self.ledger))

    def test_word_confidence_binds_raw_offsets_without_word_boxes(self):
        original = copy.deepcopy(self.response)
        page, units = self.units()
        evidence = units[0]['confidence_evidence']
        self.assertEqual(evidence['precision'], 'word_offsets_complete_block')
        self.assertEqual(units[0]['confidence'], .1)
        self.assertEqual(evidence['word_members'][1]['raw_start'], 4)
        self.assertNotIn('bbox', evidence['word_members'][1])
        self.assertEqual(len(select(units)['selected']), 1)
        self.assertEqual(self.response, original)
        self.response['pages'][0]['markdown'] = 'mutated caller'
        self.assertEqual(page['raw_response'], original)

    def test_object_content_confidence_does_not_use_type_confidence(self):
        response = copy.deepcopy(self.response)
        response['pages'][0].pop('confidence_scores')
        response['pages'][0]['blocks'][0]['confidence_scores'] = dict(
            average_content_confidence_score=.7, minimum_content_confidence_score=.05,
            block_type_confidence_score=.99)
        _, units = self.units(response)
        self.assertEqual(units[0]['confidence'], .05)
        self.assertEqual(units[0]['confidence_evidence']['precision'], 'block_content')

    def test_invalid_word_data_remains_pending(self):
        for key, value in [('start_index', 5), ('start_index', True), ('confidence', float('nan')),
                           ('confidence', 1.1), ('text', 'wrong')]:
            with self.subTest(key=key, value=value):
                response = copy.deepcopy(self.response)
                response['pages'][0]['confidence_scores']['word_confidence_scores'][1][key] = value
                _, units = self.units(response)
                selection = select(units)
                self.assertFalse(selection['selected'])
                self.assertEqual(selection['pending'][0]['reason'], 'confidence_missing_or_invalid')

    def test_partial_word_coverage_is_unknown(self):
        response = copy.deepcopy(self.response)
        response['pages'][0]['confidence_scores']['word_confidence_scores'].pop()
        _, units = self.units(response)
        self.assertIsNone(units[0]['confidence'])
        self.assertEqual(len(select(units)['pending']), 1)

    def test_page_aggregate_does_not_become_line_confidence(self):
        response = copy.deepcopy(self.response)
        response['pages'][0]['confidence_scores'].pop('word_confidence_scores')
        _, units = self.units(response)
        self.assertIsNone(units[0]['confidence'])
        self.assertFalse(select(units)['selected'])

    def test_repeated_words_use_offsets_and_not_literal_first_match(self):
        response = copy.deepcopy(self.response)
        page = response['pages'][0]
        page['markdown'] = page['blocks'][0]['content'] = 'line line'
        page['confidence_scores']['word_confidence_scores'] = [
            dict(text='line', start_index=0, confidence=.99),
            dict(text='line', start_index=5, confidence=.1)]
        _, units = self.units(response)
        self.assertEqual(units[0]['confidence'], .1)
        self.assertEqual(units[0]['confidence_evidence']['word_members'][1]['raw_start'], 5)

    def test_duplicate_word_offsets_are_unknown(self):
        response = copy.deepcopy(self.response)
        words = response['pages'][0]['confidence_scores']['word_confidence_scores']
        words.insert(1, copy.deepcopy(words[0]))
        _, units = self.units(response)
        self.assertIsNone(units[0]['confidence'])

    def test_multiple_ink_rows_do_not_prove_single_line(self):
        image = Image.new('RGB', (200, 500), 'white')
        draw = ImageDraw.Draw(image)
        draw.rectangle((25, 105, 90, 108), fill='black')
        draw.rectangle((25, 115, 90, 118), fill='black')
        image.save(self.image)
        page, units = self.units()
        self.assertEqual(units[0]['locator'], 'UNKNOWN')
        self.assertEqual(page['regions'][0]['line_proof_reason'], 'multiple_or_disconnected_ink_bands')

    def test_changed_image_invalidates_prior_proof(self):
        page, _ = self.units()
        self.assertIn('line_proof', page['regions'][0])
        Image.new('RGB', (200, 500), 'white').save(self.image)
        prove_lines(page, self.image)
        self.assertNotIn('line_proof', page['regions'][0])
        self.assertEqual(build(page, 's', 'r', 'e')[0]['locator'], 'UNKNOWN')

    def test_response_identity_mismatches_preserve_base_but_block_units(self):
        for field, value in [('model', 'another-model'), ('index', True), ('index', 1),
                             ('dimensions', dict(width=201, height=500))]:
            response = copy.deepcopy(self.response)
            (response if field == 'model' else response['pages'][0])[field] = value
            page, units = self.units(response)
            self.assertEqual(page['text'], 'Low confidence line')
            self.assertEqual(page['response_binding']['state'], 'UNKNOWN')
            self.assertFalse(select(units)['selected'])

    def test_no_state_root_fails_before_transport_or_operation_write(self):
        manifest = mistral_manifest(self.task())
        calls = []
        with self.assertRaises(BoundaryError):
            base_run(manifest, self.approval(manifest), None,
                     lambda *args: calls.append(args))
        self.assertFalse(calls)
        self.assertFalse(Path(manifest['task']['output_directory']).exists())

    def test_legacy_unknown_state_blocks_relocation_without_mutation(self):
        legacy = self.root / 'old-ledger'
        legacy.mkdir()
        raw = legacy / 'state.sqlite'
        raw.write_bytes(b'preserve UNKNOWN state and raw')
        before = digest(raw)
        with patch('scripts.ocr.paid_mvp.state.legacy_directory', return_value=legacy):
            with self.assertRaisesRegex(BoundaryError, 'LEGACY_LEDGER_RELOCATION_REQUIRED'):
                directory(self.ledger)
        self.assertEqual(digest(raw), before)
        self.assertFalse(self.ledger.exists())

    def test_unsafe_state_roots_rejected(self):
        for path in [Path('.'), Path(__file__).resolve().parent / '.request-ledger',
                     self.root / 'test.app' / 'state']:
            with self.assertRaises(BoundaryError):
                directory(path)
        public = self.root / 'public'
        public.mkdir(mode=0o755)
        with self.assertRaises(BoundaryError):
            directory(public)
        link = self.root / 'link'
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(BoundaryError):
            directory(link / 'ledger')

    def test_frozen_crop_cannot_be_overwritten(self):
        page, units = self.units()
        shape = request_shape(page, units, self.image, self.root / 'crops')
        path = Path(shape['audit'][0]['crop_path'])
        path.write_bytes(b'frozen conflict')
        with self.assertRaises(BoundaryError):
            request_shape(page, units, self.image, self.root / 'crops')
        self.assertEqual(path.read_bytes(), b'frozen conflict')

    def test_identity_failure_stops_next_page_and_restart_send(self):
        manifest = mistral_manifest(self.task(2))
        response = copy.deepcopy(self.response)
        response['model'] = 'wrong-model'
        sent = []

        def transport(*args):
            sent.append(args[0])
            return 200, canonical(response)

        result = base_run(manifest, self.approval(manifest), None, transport, state_root=self.ledger)
        self.assertEqual(len(sent), 1)
        self.assertEqual(result['page_results'][0]['observation']['text'], 'Low confidence line')
        self.assertEqual(result['page_results'][1]['status'], 'not_sent')
        call = manifest['calls'][0]
        body = dict(model=call['model'], document=dict(type='image_url', image_url=image_url(self.image)),
                    include_blocks=True, confidence_scores_granularity='word', include_image_base64=False)
        with self.assertRaises(BoundaryError):
            durable_request(call, body, None, self.approval(manifest), transport, state_root=self.ledger)
        self.assertEqual(len(sent), 1)
        self.assertTrue((self.ledger / 'raw' / call['request_id'] / 'dispatch-halt.json').exists())
        with sqlite3.connect(self.ledger / 'state.sqlite') as db:
            self.assertEqual(db.execute('SELECT state FROM calls').fetchone()[0], 'identity_failed')

    def test_nonobject_response_preserves_receipt_and_unknown_hold(self):
        manifest = mistral_manifest(self.task())
        result = base_run(manifest, self.approval(manifest), None,
                          lambda *args: (200, canonical(['malformed provider response'])),
                          state_root=self.ledger)
        receipt = result['page_results'][0]['base_receipt']
        self.assertEqual(receipt['response'], ['malformed provider response'])
        self.assertIsNone(receipt['list_price_estimate_usd'])
        self.assertEqual(receipt['unsettled_reserve_usd'], .004)
        self.assertTrue(Path(receipt['raw_path']).exists())

    def test_received_response_blocks_other_identity_until_validation(self):
        from .pipeline import MISTRAL
        call = mistral_manifest(self.task())['calls'][0]
        body = dict(model=call['model'], document=dict(type='image_url', image_url=image_url(self.image)),
                    include_blocks=True, confidence_scores_granularity='word', include_image_base64=False)
        self.assertEqual(call['destination'], MISTRAL)
        sent = []
        def transport(*args):
            sent.append(args[0])
            return 200, canonical(self.response)
        receipt = durable_request(call, body, None, {}, transport, state_root=self.ledger)
        self.assertEqual(receipt['http_status'], 200)
        other_body = copy.deepcopy(body)
        other_body['include_image_base64'] = True
        other = copy.deepcopy(call)
        other['body_sha256'] = hashlib.sha256(canonical(other_body)).hexdigest()
        other['request_id'] = hashlib.sha256(canonical({k: other[k] for k in
            ['image_sha256', 'destination', 'model', 'body_sha256']})).hexdigest()
        with self.assertRaises(BoundaryError):
            durable_request(other, other_body, None, {}, transport, state_root=self.ledger)
        self.assertEqual(len(sent), 1)
        with sqlite3.connect(self.ledger / 'state.sqlite') as db:
            self.assertEqual(db.execute('SELECT state FROM calls').fetchone()[0], 'received')

    def test_malformed_blocks_preserve_markdown_and_stop_new_send(self):
        manifest = mistral_manifest(self.task())
        response = copy.deepcopy(self.response)
        response['pages'][0]['blocks'] = 'malformed'
        result = base_run(manifest, self.approval(manifest), None,
                          lambda *args: (200, canonical(response)), state_root=self.ledger)
        page = result['page_results'][0]
        self.assertEqual(page['status'], 'failed')
        self.assertEqual(page['raw_markdown_pages'][0]['markdown'], 'Low confidence line')
        self.assertTrue(Path(page['base_receipt']['raw_path']).exists())
        with sqlite3.connect(self.ledger / 'state.sqlite') as db:
            self.assertEqual(db.execute('SELECT state FROM calls').fetchone()[0], 'identity_failed')

    def test_unicode_offsets_are_literal_codepoints(self):
        response = copy.deepcopy(self.response)
        page = response['pages'][0]
        page['markdown'] = page['blocks'][0]['content'] = 'λόγος a\u0301'
        page['confidence_scores']['word_confidence_scores'] = [
            dict(text='λόγος', start_index=0, confidence=.95),
            dict(text='a\u0301', start_index=6, confidence=.1)]
        _, units = self.units(response)
        self.assertEqual(units[0]['confidence'], .1)
        page['confidence_scores']['word_confidence_scores'][1]['start_index'] = 11 # UTF-8 byte offset
        _, units = self.units(response)
        self.assertIsNone(units[0]['confidence'])

    def test_live_root_binding_rejects_before_credential_or_send(self):
        manifest = mistral_manifest(self.task())
        call = manifest['calls'][0]
        body = dict(model=call['model'], document=dict(type='image_url', image_url=image_url(self.image)),
                    include_blocks=True, confidence_scores_granularity='word', include_image_base64=False)
        with patch('scripts.ocr.paid_mvp.pipeline.credential', side_effect=AssertionError('credential read')):
            with self.assertRaisesRegex(BoundaryError, 'exact approved persistent state root'):
                durable_request(call, body, None, {}, state_root=self.ledger)
        self.assertFalse(self.ledger.exists())

    def test_timeout_hold_survives_process_restart_and_changed_body(self):
        page, units = self.units()
        shape = request_shape(page, units, self.image, self.root / 'crops')
        from .selective import repair_manifest
        call = repair_manifest(shape, {}, 1)['calls'][0]
        path = self.root / 'uncertain.json'
        path.write_text(json.dumps(dict(call=call, body=shape['body'], state_root=str(self.ledger))))
        script = """
import hashlib,json,sys
from scripts.ocr.paid_mvp.selective import durable_request
from scripts.ocr.paid_mvp.pipeline import canonical,BoundaryError
p=json.load(open(sys.argv[1]));second=sys.argv[2]=='second'
if second:
 p['body']['max_output_tokens']=4000
 p['call']['body_sha256']=hashlib.sha256(canonical(p['body'])).hexdigest()
 p['call']['request_id']=hashlib.sha256(canonical({k:p['call'][k] for k in ['image_sha256','destination','model','body_sha256']})).hexdigest()
def transport(*args):
 if second:raise AssertionError('restart cleared uncertain hold')
 raise TimeoutError('synthetic timeout')
try:
 r=durable_request(p['call'],p['body'],None,{},transport,state_root=p['state_root'])
 print(r['error'],r['unsettled_reserve_usd'])
except BoundaryError:
 print('blocked')
"""
        first = subprocess.run([sys.executable, '-c', script, str(path), 'first'], check=True,
                               text=True, capture_output=True, timeout=10)
        second = subprocess.run([sys.executable, '-c', script, str(path), 'second'], check=True,
                                text=True, capture_output=True, timeout=10)
        self.assertEqual(first.stdout.strip(), 'TimeoutError 0.05')
        self.assertEqual(second.stdout.strip(), 'blocked')
        with sqlite3.connect(self.ledger / 'state.sqlite') as db:
            self.assertEqual(db.execute('SELECT state,reserve,cost FROM calls').fetchall(), [('failed', .05, None)])

    def test_same_request_reused_across_fresh_processes(self):
        # Real disk/SQLite, synthetic response. Second process cannot call transport.
        page, units = self.units()
        shape = request_shape(page, units, self.image, self.root / 'crops')
        from .selective import repair_manifest
        call = repair_manifest(shape, {}, 1)['calls'][0]
        payload = dict(call=call, body=shape['body'], state_root=str(self.ledger))
        path = self.root / 'request.json'
        path.write_text(json.dumps(payload))
        script = """
import json,sys
from scripts.ocr.paid_mvp.selective import durable_request
from scripts.ocr.paid_mvp.pipeline import canonical
p=json.load(open(sys.argv[1]));second=sys.argv[2]=='second'
def transport(*args):
 if second:raise AssertionError('second process tried resend')
 return 200,canonical(dict(usage=dict(input_tokens=1,output_tokens=1)))
r=durable_request(p['call'],p['body'],None,{},transport,state_root=p['state_root'])
print(r['response_sha256'])
"""
        first = subprocess.run([sys.executable, '-c', script, str(path), 'first'], check=True,
                               text=True, capture_output=True, timeout=10)
        second = subprocess.run([sys.executable, '-c', script, str(path), 'second'], check=True,
                                text=True, capture_output=True, timeout=10)
        self.assertEqual(first.stdout, second.stdout)
        self.assertEqual(self.ledger.stat().st_mode & 0o077, 0)


if __name__ == '__main__':
    unittest.main()
