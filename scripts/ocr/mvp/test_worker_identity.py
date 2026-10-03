"""Real receipt structure and source mutation refusal, without recognition."""
import json
import os
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from . import local

REAL_POPEN=subprocess.Popen


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


class ParentWorkerLifecycleTests(unittest.TestCase):
    """Real bounded process fixtures, synthetic native PDF, no recognizer."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.source=self.root/'source.pdf';self.out=self.root/'out';self.owned=[]
        with local.fitz.open() as doc:
            for number in (1,2):doc.new_page().insert_text((72,72),f'Synthetic native transport control page {number}')
            doc.save(self.source)
        self.source_hash=local.sha(self.source)
        self.task=dict(operation_id='parent-worker-control',input_pdf=str(self.source),input_sha256=self.source_hash,
                       page_numbers=[1],mode='local',output_directory=str(self.out),config_version=local.CONFIG_VERSION)
        self.config=dict(apple_helper=str(self.source),tesseract=str(self.source),page_timeout_seconds=5)

    def tearDown(self):
        for proc in self.owned:
            if proc.returncode is None:
                try:os.killpg(proc.pid,signal.SIGKILL)
                except ProcessLookupError:pass
            proc.wait(timeout=5)
            for stream in (proc.stdout,proc.stderr):
                if stream is not None:stream.close()
        self.temp.cleanup()

    def spawn(self,args,**kwargs):
        self.assertTrue(kwargs['start_new_session'])
        proc=REAL_POPEN([sys.executable,'-c','import time; time.sleep(30)'],**kwargs)
        self.owned.append(proc);return proc

    def execute(self,factory,**extra):
        with patch.object(local.subprocess,'Popen',side_effect=factory),patch.object(local,'invoke') as reader:
            result=local.run_task(self.task,dict(self.config,**extra))
            reader.assert_not_called()
        self.assertEqual(local.sha(self.source),self.source_hash)
        return result

    def assert_reaped(self,proc):
        self.assertIsNotNone(proc.returncode)
        self.assertTrue(proc.stdout.closed);self.assertTrue(proc.stderr.closed)
        with self.assertRaises(ChildProcessError):os.waitpid(proc.pid,os.WNOHANG)

    def test_read_failure_reaps_owned_worker_and_preserves_failed_page(self):
        def factory(*args,**kwargs):
            proc=self.spawn(*args,**kwargs)
            proc.communicate=lambda *a,**k:(_ for _ in ()).throw(OSError('SYNTHETIC_READ_FAILURE'))
            return proc
        result=self.execute(factory)
        self.assertEqual(result['status'],'failed');self.assertIn('SYNTHETIC_READ_FAILURE',result['page_results'][0]['error'])
        self.assert_reaped(self.owned[0]);self.assertEqual(len(self.owned),1)
        self.assertEqual(local.read(self.out/'raw/page-0001/page-result.json')['status'],'FAILED')
        with local.fitz.open(result['artifacts']['searchable_pdf']) as saved,local.fitz.open(self.source) as source:
            self.assertEqual(len(saved),2)
            for a,b in zip(saved,source):self.assertEqual(a.get_pixmap().samples,b.get_pixmap().samples)

    def test_spawn_failure_is_persisted_without_starting_a_worker(self):
        result=self.execute(lambda *a,**k:(_ for _ in ()).throw(OSError('SYNTHETIC_SPAWN_FAILURE')))
        self.assertEqual(self.owned,[]);self.assertEqual(result['page_results'][0]['status'],'FAILED')
        self.assertIn('SYNTHETIC_SPAWN_FAILURE',local.read(self.out/'raw/page-0001/page-result.json')['error'])

    def test_existing_worker_bytes_survive_read_error_and_resume_without_a_worker(self):
        raw=self.out/'raw/page-0001/page-result.json';original=b'{"status":"NATIVE_PRESERVED","synthetic_raw_marker":1}\n'
        def factory(*args,**kwargs):
            raw.parent.mkdir(parents=True);raw.write_bytes(original)
            proc=self.spawn(*args,**kwargs)
            proc.communicate=lambda *a,**k:(_ for _ in ()).throw(OSError('SYNTHETIC_READ_FAILURE'))
            return proc
        with patch.object(local,'publish',side_effect=OSError('SYNTHETIC_EXPORT_INTERRUPTION')):
            with self.assertRaisesRegex(OSError,'SYNTHETIC_EXPORT_INTERRUPTION'):self.execute(factory)
        self.assert_reaped(self.owned[0]);self.assertEqual(raw.read_bytes(),original)
        receipt=local.read(raw.with_name('parent-result.json'))
        self.assertEqual(receipt['status'],'FAILED');self.assertEqual(receipt['raw_files']['raw/page-0001/page-result.json'],local.sha(raw))
        result=self.execute(lambda *a,**k:self.fail('Resume started a worker for an already failed page'))
        self.assertEqual(result['page_results'][0]['status'],'FAILED');self.assertEqual(raw.read_bytes(),original)
        self.assertEqual(len(self.owned),1);local.load_snapshot(self.out)

    def test_malformed_worker_result_is_preserved_when_parser_fails(self):
        raw=self.out/'raw/page-0001/page-result.json';original=b'{"partial":'
        def factory(args,**kwargs):
            raw.parent.mkdir(parents=True);raw.write_bytes(original)
            proc=REAL_POPEN([sys.executable,'-c','print("synthetic normal stdout")'],**kwargs)
            self.owned.append(proc);return proc
        result=self.execute(factory)
        self.assertEqual(result['page_results'][0]['status'],'FAILED');self.assertIn('JSONDecodeError',result['page_results'][0]['error'])
        self.assertEqual(raw.read_bytes(),original);self.assert_reaped(self.owned[0]);local.load_snapshot(self.out)
        self.assertIn('synthetic normal stdout',(self.out/'raw/worker-1.stdout').read_text())

    def test_normal_worker_result_and_output_remain_usable(self):
        raw=self.out/'raw/page-0001/page-result.json'
        page=dict(page=1,status='NATIVE_PRESERVED',route='native',words=[],source_sha256=self.source_hash)
        def factory(args,**kwargs):
            raw.parent.mkdir(parents=True);local.write(raw,page)
            proc=REAL_POPEN([sys.executable,'-c','import sys; print("control stdout"); print("control stderr",file=sys.stderr)'],**kwargs)
            self.owned.append(proc);return proc
        result=self.execute(factory)
        self.assertEqual(result['page_results'][0]['status'],'NATIVE_PRESERVED');self.assert_reaped(self.owned[0])
        self.assertFalse(raw.with_name('parent-result.json').exists())
        self.assertEqual((self.out/'raw/worker-1.stdout').read_text(),'control stdout\n')
        self.assertEqual((self.out/'raw/worker-1.stderr').read_text(),'control stderr\n')

    def test_active_cancel_preserves_existing_worker_result_bytes(self):
        raw=self.out/'raw/page-0001/page-result.json';original=b'{"synthetic_cancel_marker":1}\n'
        def factory(*args,**kwargs):
            raw.parent.mkdir(parents=True);raw.write_bytes(original);(self.out/'CANCEL').touch()
            return self.spawn(*args,**kwargs)
        result=self.execute(factory)
        self.assertEqual(result['status'],'cancelled');self.assertEqual(result['page_results'][0]['status'],'CANCELLED')
        self.assertEqual(raw.read_bytes(),original);self.assert_reaped(self.owned[0]);local.load_snapshot(self.out)

    def test_timeout_preserves_existing_worker_result_bytes(self):
        raw=self.out/'raw/page-0001/page-result.json';original=b'{"synthetic_timeout_marker":1}\n'
        def factory(*args,**kwargs):
            raw.parent.mkdir(parents=True);raw.write_bytes(original)
            return self.spawn(*args,**kwargs)
        result=self.execute(factory,page_timeout_seconds=1)
        self.assertEqual(result['page_results'][0]['error'],'PAGE_TIMEOUT')
        self.assertEqual(raw.read_bytes(),original);self.assert_reaped(self.owned[0]);local.load_snapshot(self.out)

    def test_unconfirmed_cleanup_stops_next_page_and_blocks_automatic_resume(self):
        self.task['page_numbers']=[1,2]
        with patch.object(local,'reap_page_worker',side_effect=local.PageWorkerCleanupError('SYNTHETIC_CLEANUP_UNCONFIRMED')):
            with self.assertRaisesRegex(local.PageWorkerCleanupError,'SYNTHETIC_CLEANUP_UNCONFIRMED'):
                self.execute(self.spawn,page_timeout_seconds=1)
        self.assertEqual(len(self.owned),1);self.assertIsNone(self.owned[0].poll())
        self.assertTrue(local.read(self.out/'worker-cleanup-failure.json')['resume_blocked'])
        self.assertFalse((self.out/'raw/worker-2.json').exists());self.assertFalse((self.out/'completion.json').exists())
        with self.assertRaisesRegex(local.PageWorkerCleanupError,'PAGE_WORKER_CLEANUP_UNVERIFIED'):
            self.execute(lambda *a,**k:self.fail('Resume ignored unconfirmed cleanup'),page_timeout_seconds=1)


if __name__=='__main__':unittest.main()
