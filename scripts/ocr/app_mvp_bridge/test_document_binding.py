"""Source/page admission before any local task or worker. No OCR/provider calls."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import fitz
from .__main__ import main, sha


class DocumentBindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.source = self.root / 'source.pdf'
        with fitz.open() as doc:
            doc.new_page(); doc.new_page(); doc.save(self.source)
        self.config = dict(session_root=str(self.root / 'sessions'), local={},
                           require_document_binding=True)
        self.request = dict(action='start', mode='local', input_pdf=str(self.source),
                            input_sha256=sha(self.source), pages=[2, 1],
                            document_id='doc-1', document_page_count=2,
                            client_operation_id='a' * 32)

    def tearDown(self):
        self.temp.cleanup()

    def reject(self, request, message):
        with patch('scripts.ocr.mvp.local.run_task') as run, \
             patch('scripts.ocr.app_mvp_bridge.__main__.register_local_operation') as register:
            with self.assertRaisesRegex(ValueError, message):
                main(request, self.config)
            register.assert_not_called(); run.assert_not_called()
        self.assertFalse((self.root / 'sessions').exists())

    def test_source_changed_during_runtime_preparation_rejected_before_state(self):
        self.source.write_bytes(b'changed source')
        self.reject(self.request, 'SOURCE_HASH_MISMATCH')

    def test_missing_document_and_changed_page_count_rejected_before_state(self):
        self.reject(dict(self.request, document_id=None), 'DOCUMENT_BINDING_REQUIRED')
        self.reject(dict(self.request, document_page_count=3), 'DOCUMENT_PAGE_COUNT_CHANGED')
        self.reject(dict(self.request, document_page_count=True), 'DOCUMENT_PAGE_COUNT_CHANGED')

    def test_distinct_typed_physical_pages_required_before_state(self):
        for pages in (None, [], [True], [1.0], [0], [3], [1] * 41):
            self.reject(dict(self.request, pages=pages), 'INVALID_PHYSICAL_PAGES')
        self.reject(dict(self.request, pages=[1, 1]), 'DUPLICATE_PHYSICAL_PAGES')

    def test_admitted_request_keeps_order_and_binding_in_immutable_session(self):
        with patch('scripts.ocr.mvp.local.run_task') as run, \
             patch('scripts.ocr.app_mvp_bridge.__main__.register_local_operation') as register, \
             patch('scripts.ocr.app_mvp_bridge.__main__.view', return_value={'component': True}):
            main(self.request, self.config)
            task = register.call_args.args[1]
            self.assertEqual(task['page_numbers'], [2, 1])
            self.assertEqual(run.call_args.args[0], task)
        meta = json.loads(next((self.root / 'sessions').glob('*/desktop-session.json')).read_text())
        self.assertEqual(meta['document_binding']['physical_pages'], [2, 1])
        self.assertEqual(meta['document_binding']['source_page_count'], 2)
        self.assertEqual(meta['document_binding']['document_id'], 'doc-1')
        self.assertTrue(meta['document_binding']['source_checked_before_session'])
        self.assertTrue(meta['document_binding']['binding_required'])


if __name__ == '__main__':
    unittest.main()
