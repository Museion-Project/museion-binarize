"""Real receipt structure and source mutation refusal, without recognition."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from . import local


class WorkerIdentityTests(unittest.TestCase):
    def test_receipt_uses_actual_loaded_source_and_keeps_eval_out(self):
        modules=local.loaded_source_identity()
        self.assertEqual(set(modules),{'scripts.ocr.mvp.core','scripts.ocr.mvp.local','scripts.ocr.mvp.store','scripts.ocr.free_local.pipeline'})
        self.assertTrue(all(local.sha(m['path'])==m['sha256'] for m in modules.values()))

    def test_receipt_rechecks_existing_tessdata_without_extra_reader_call(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'input';path.write_bytes(b'fixture')
            config=dict(apple_helper=str(path),tesseract=str(path),font=str(path))
            trained=[dict(name='eng',path=str(path),sha256=local.sha(path))]
            with patch.object(local,'model_dependencies',return_value=trained) as languages:
                before=local.worker_runtime_identity(config)
                after=local.worker_runtime_identity(config,before['tessdata'])
            self.assertEqual(languages.call_count,1)
            self.assertEqual(before,after)
            path.write_bytes(b'changed')
            self.assertNotEqual(before,local.worker_runtime_identity(config,before['tessdata']))

    def test_source_change_is_a_persisted_failed_page_with_no_reader(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source.pdf';source.write_bytes(b'changed source')
            request=dict(input_pdf=str(source),input_sha256='0'*64,page=1,output=str(root/'out'),config={},mode='local')
            with patch.object(local,'worker_runtime_identity') as identity,patch.object(local,'invoke') as reader:
                result=local.worker(request)
            identity.assert_not_called();reader.assert_not_called()
            self.assertEqual(result['status'],'FAILED');self.assertEqual(result['route'],'failed')
            self.assertEqual(result['words'],[])
            persisted=json.loads((root/'out/raw/page-0001/page-result.json').read_text())
            self.assertIn('WORKER_SOURCE_CHANGED',persisted['error'])


if __name__=='__main__':unittest.main()
