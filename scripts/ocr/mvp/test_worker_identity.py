"""Real receipt structure and source mutation refusal, without recognition."""
import json
import fcntl
import os
import signal
import subprocess
import sys
import tempfile
import time
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


class TaskProducerOwnershipTests(unittest.TestCase):
    """Real process locks and synthetic native PDF; no recognition or PID guesses."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.source=self.root/'source.pdf';self.out=self.root/'operation';self.out.mkdir()
        with local.fitz.open() as doc:
            page=doc.new_page()
            for row in range(4):page.insert_text((30,40+20*row),f'Synthetic native producer ownership control row {row} preserves source bytes 314159.')
            doc.save(self.source)
        self.task=dict(operation_id='producer-ownership-control',input_pdf=str(self.source),
                       input_sha256=local.sha(self.source),page_numbers=[1],mode='local',
                       output_directory=str(self.out),config_version=local.CONFIG_VERSION)
        self.config=dict(apple_helper=str(self.source),tesseract=str(self.source),
                         font=str(local.ROOT/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf'))
        normalized=local.runtime_config(self.config)
        local.write(self.out/'job.json',dict(schema_version=1,identity=local.digest(dict(task=self.task,config=normalized)),
                                            task=self.task,config=normalized))

    def tearDown(self):self.temp.cleanup()

    def files(self):return {str(p.relative_to(self.out)):local.sha(p) for p in self.out.rglob('*') if p.is_file()}

    def test_another_process_cannot_start_same_task_or_change_saved_bytes(self):
        code='''import sys
from scripts.ocr.mvp import local
with local.task_producer(sys.argv[1]):
 print('OWNED',flush=True)
 sys.stdin.readline()
'''
        proc=REAL_POPEN([sys.executable,'-c',code,str(self.out)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=local.ROOT)
        try:
            import selectors
            with selectors.DefaultSelector() as selector:
                selector.register(proc.stdout,selectors.EVENT_READ);self.assertTrue(selector.select(8))
            self.assertEqual(proc.stdout.readline().strip(),'OWNED');self.assertIsNone(proc.poll())
            before=self.files()
            with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Second producer spawned a worker')) as worker:
                with self.assertRaisesRegex(ValueError,'TASK_PRODUCER_BUSY'):local.run_task(self.task,self.config)
                worker.assert_not_called()
            self.assertEqual(self.files(),before)
        finally:
            proc.communicate('release\n',timeout=8)
            self.assertEqual(proc.returncode,0)
            self.assertTrue(all(p.closed for p in (proc.stdin,proc.stdout,proc.stderr)))
            with self.assertRaises(ChildProcessError):os.waitpid(proc.pid,os.WNOHANG)

    def test_owner_exception_releases_lock_then_native_task_can_finish(self):
        class InterruptedBeforeWorker(BaseException):pass
        with patch.object(local.subprocess,'Popen',side_effect=InterruptedBeforeWorker):
            with self.assertRaises(InterruptedBeforeWorker):local.run_task(self.task,self.config)
        self.assertFalse((self.out/'CURRENT.json').exists())
        result=local.run_task(self.task,self.config)
        self.assertEqual(result['page_results'][0]['status'],'NATIVE_PRESERVED')
        before=self.files()
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Completed native page restarted')):
            self.assertEqual(local.run_task(self.task,self.config),result)
        self.assertEqual(self.files(),before)

    def test_crashed_owner_releases_lock_without_stale_marker_repair(self):
        code='''import os,sys
from scripts.ocr.mvp import local
with local.task_producer(sys.argv[1]):os._exit(88)
'''
        before=self.files()
        proc=REAL_POPEN([sys.executable,'-c',code,str(self.out)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=local.ROOT)
        stdout,stderr=proc.communicate(timeout=8)
        self.assertEqual(proc.returncode,88);self.assertEqual((stdout,stderr),('',''))
        self.assertEqual(self.files(),before)
        self.assertTrue(proc.stdout.closed and proc.stderr.closed)
        with self.assertRaises(ChildProcessError):os.waitpid(proc.pid,os.WNOHANG)
        with local.task_producer(self.out):self.assertEqual(self.files(),before)

    def test_lock_descriptor_is_readonly_noninheritable_and_closed_on_error(self):
        opened=[];real_open=local.os.open
        def capture(*a,**k):
            fd=real_open(*a,**k);opened.append(fd);return fd
        with patch.object(local.os,'open',side_effect=capture):
            with self.assertRaisesRegex(RuntimeError,'synthetic owner error'):
                with local.task_producer(self.out):
                    self.assertEqual(len(opened),1);fd=opened[0]
                    self.assertFalse(os.get_inheritable(fd))
                    self.assertEqual(fcntl.fcntl(fd,fcntl.F_GETFL)&os.O_ACCMODE,os.O_RDONLY)
                    raise RuntimeError('synthetic owner error')
        with self.assertRaises(OSError):os.fstat(opened[0])

    def test_different_output_task_is_independent_of_held_task(self):
        second=self.root/'second';task=dict(self.task,operation_id='independent-producer',output_directory=str(second))
        class StoppedAtWorker(BaseException):pass
        with local.task_producer(self.out):
            with patch.object(local.subprocess,'Popen',side_effect=StoppedAtWorker) as worker:
                with self.assertRaises(StoppedAtWorker):local.run_task(task,self.config)
                self.assertEqual(worker.call_count,1)
        self.assertTrue((second/'job.json').is_file());self.assertFalse((second/'CURRENT.json').exists())

    def test_linked_or_nonregular_job_is_refused_before_worker(self):
        job=self.out/'job.json';original=job.read_bytes()
        for kind,reason in (('symlink','TASK_JOB_PATH_SYMLINK'),('hardlink','TASK_JOB_FILE_UNSAFE'),('directory','TASK_JOB_FILE_UNSAFE'),('fifo','TASK_JOB_FILE_UNSAFE')):
            with self.subTest(kind=kind):
                job.unlink();other=self.root/('other-'+kind);other.write_bytes(original)
                if kind=='symlink':job.symlink_to(other)
                elif kind=='hardlink':os.link(other,job)
                elif kind=='directory':job.mkdir()
                else:os.mkfifo(job)
                with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Unsafe job started worker')) as worker:
                    with self.assertRaisesRegex(ValueError,reason):local.run_task(self.task,self.config)
                    worker.assert_not_called()
                self.assertEqual(other.read_bytes(),original)
                if job.is_dir():job.rmdir()
                else:job.unlink()
                job.write_bytes(original)

    def test_job_replacement_while_acquiring_ownership_is_refused(self):
        job=self.out/'job.json';original=job.read_bytes();real_flock=local.fcntl.flock
        def replace(fd,flags):
            real_flock(fd,flags);job.rename(self.out/'original-job.json');job.write_bytes(original)
        with patch.object(local.fcntl,'flock',side_effect=replace),patch.object(local.subprocess,'Popen',side_effect=AssertionError('Changed job started worker')) as worker:
            with self.assertRaisesRegex(ValueError,'TASK_JOB_FILE_CHANGED'):local.run_task(self.task,self.config)
            worker.assert_not_called()
        self.assertEqual((self.out/'original-job.json').read_bytes(),original)
        self.assertEqual(job.read_bytes(),original)
        self.assertFalse((self.out/'raw').exists())


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


class WorkerParentLifelineTests(unittest.TestCase):
    """Actual private worker main, synthetic body, bounded inherited-group child."""
    program='''import json,os,subprocess,sys,time
from pathlib import Path
from scripts.ocr.mvp import local
def fixture(request):
 root=Path(request['output'])
 (root/'partial.raw').write_bytes(b'synthetic partial raw preserved\\n')
 helper=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']) if request.get('helper') else None
 (root/'ready.json').write_text(json.dumps(dict(pid=os.getpid(),group=os.getpgrp(),helper_pid=helper.pid if helper else None)))
 if request.get('finish'):
  (root/'completed.raw').write_bytes(b'synthetic completed result preserved\\n');print('completed',flush=True)
 else:time.sleep(30)
local.worker=fixture
local.main()
'''

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.proc=None;self.isolated=True
        self.fds=[];self.request=self.root/'request.json'

    def tearDown(self):
        if self.proc is not None:
            if self.proc.returncode is None:
                if self.isolated:
                    try:os.killpg(self.proc.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
                else:self.proc.kill()
            self.proc.wait(timeout=5)
            for stream in (self.proc.stdout,self.proc.stderr):
                if stream is not None:stream.close()
        for fd in self.fds:
            try:os.close(fd)
            except OSError:pass
        self.temp.cleanup()

    def pipe(self):
        read_fd,write_fd=os.pipe();self.fds.extend((read_fd,write_fd));return read_fd,write_fd

    def close_fd(self,fd):os.close(fd);self.fds.remove(fd)

    def spawn(self,fd=None,*,helper=False,finish=False,isolated=True):
        local.write(self.request,dict(output=str(self.root),helper=helper,finish=finish))
        args=[sys.executable,'-c',self.program]
        if fd is not None:args.extend(['--parent-lifeline-fd',str(fd)])
        args.extend(['--worker',str(self.request)]);self.isolated=isolated
        self.proc=REAL_POPEN(args,cwd=local.ROOT,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,
                             start_new_session=isolated,pass_fds=() if fd is None else (fd,))
        return self.proc

    def wait_ready(self):
        deadline=time.monotonic()+5;path=self.root/'ready.json'
        while time.monotonic()<deadline:
            if path.exists():
                try:return local.read(path)
                except json.JSONDecodeError:pass
            if self.proc.poll() is not None:
                self.fail('Worker exited before its synthetic body: '+self.proc.stderr.read())
            time.sleep(.01)
        self.fail('Bounded synthetic worker did not signal readiness')

    def test_parent_pipe_loss_stops_worker_and_inherited_child_output(self):
        read_fd,write_fd=self.pipe();proc=self.spawn(read_fd,helper=True);self.close_fd(read_fd)
        ready=self.wait_ready();self.assertEqual(ready['group'],proc.pid);self.assertIsNotNone(ready['helper_pid'])
        raw=(self.root/'partial.raw').read_bytes();started=time.monotonic();self.close_fd(write_fd)
        # The sleeping child holds inherited stdout/stderr open for 30s.
        # Bounded EOF proves that it no longer holds these live connections.
        proc.communicate(timeout=5)
        self.assertEqual(proc.returncode,-signal.SIGKILL);self.assertLess(time.monotonic()-started,5)
        self.assertEqual((self.root/'partial.raw').read_bytes(),raw)
        with self.assertRaises(ChildProcessError):os.waitpid(proc.pid,os.WNOHANG)

    def test_connected_normal_worker_completes_without_being_killed(self):
        read_fd,write_fd=self.pipe();proc=self.spawn(read_fd,finish=True);self.close_fd(read_fd)
        stdout,stderr=proc.communicate(timeout=5)
        self.assertEqual(proc.returncode,0);self.assertEqual(stdout,'completed\n');self.assertEqual(stderr,'')
        self.assertEqual((self.root/'completed.raw').read_bytes(),b'synthetic completed result preserved\n')
        self.close_fd(write_fd)

    def test_already_disconnected_parent_refuses_worker_body(self):
        read_fd,write_fd=self.pipe();self.close_fd(write_fd);proc=self.spawn(read_fd);self.close_fd(read_fd)
        proc.communicate(timeout=5)
        self.assertEqual(proc.returncode,-signal.SIGKILL);self.assertFalse((self.root/'ready.json').exists())
        self.assertFalse((self.root/'partial.raw').exists())

    def test_missing_lifeline_refuses_unmanaged_worker(self):
        proc=self.spawn();_,stderr=proc.communicate(timeout=5)
        self.assertNotEqual(proc.returncode,0);self.assertIn('WORKER_PARENT_LIFELINE_REQUIRED',stderr)
        self.assertFalse((self.root/'ready.json').exists())

    def test_regular_file_descriptor_is_refused_before_body(self):
        path=self.root/'regular';path.write_bytes(b'not a pipe');fd=os.open(path,os.O_RDONLY);self.fds.append(fd)
        proc=self.spawn(fd);_,stderr=proc.communicate(timeout=5)
        self.assertNotEqual(proc.returncode,0);self.assertIn('WORKER_LIFELINE_PIPE_REQUIRED',stderr)
        self.assertFalse((self.root/'ready.json').exists())

    def test_write_end_is_refused_before_body(self):
        _,write_fd=self.pipe();proc=self.spawn(write_fd);_,stderr=proc.communicate(timeout=5)
        self.assertNotEqual(proc.returncode,0);self.assertIn('WORKER_LIFELINE_READ_END_REQUIRED',stderr)
        self.assertFalse((self.root/'ready.json').exists())

    def test_shared_process_group_is_refused_before_body(self):
        read_fd,_=self.pipe();proc=self.spawn(read_fd,isolated=False);_,stderr=proc.communicate(timeout=5)
        self.assertNotEqual(proc.returncode,0);self.assertIn('WORKER_ISOLATED_GROUP_REQUIRED',stderr)
        self.assertFalse((self.root/'ready.json').exists())

    def test_parent_dispatch_closes_pipe_when_starting_worker_fails(self):
        source=self.root/'source.pdf'
        with local.fitz.open() as doc:doc.new_page().insert_text((72,72),'Synthetic failed launch source');doc.save(source)
        task=dict(operation_id='synthetic-pipe-cleanup',input_pdf=str(source),input_sha256=local.sha(source),page_numbers=[1],
                  mode='local',output_directory=str(self.root/'operation'),config_version=local.CONFIG_VERSION)
        created=[];real_pipe=os.pipe
        def pipe():
            fds=real_pipe();created.extend(fds);return fds
        with patch.object(local.os,'pipe',side_effect=pipe),patch.object(local.subprocess,'Popen',side_effect=OSError('SYNTHETIC_START_FAILURE')),patch.object(local,'invoke') as reader:
            result=local.run_task(task,dict(apple_helper=str(source),tesseract=str(source)));reader.assert_not_called()
        self.assertEqual(result['page_results'][0]['status'],'FAILED');self.assertEqual(len(created),2)
        for fd in created:
            with self.assertRaises(OSError):os.fstat(fd)


class PublishedDraftResumeTests(unittest.TestCase):
    """Real immutable publication with synthetic native pages; no worker replay."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.source=self.root/'source.pdf';self.out=self.root/'operation';self.out.mkdir()
        with local.fitz.open() as doc:
            for number in (1,2):doc.new_page().insert_text((20,30),f'Synthetic published source page {number} 314159')
            doc.save(self.source)
        self.task=dict(operation_id='published-draft-recovery',input_pdf=str(self.source),input_sha256=local.sha(self.source),
                       page_numbers=[1,2],mode='local',output_directory=str(self.out),config_version=local.CONFIG_VERSION)
        self.config=dict(apple_helper=str(self.source),tesseract=str(self.source),
                         font=str(local.ROOT/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf'))
        self.normalized=local.runtime_config(self.config)
        local.write(self.out/'job.json',dict(schema_version=1,identity=local.digest(dict(task=self.task,config=self.normalized)),
                                            task=self.task,config=self.normalized))
        raw=self.out/'raw/page-0001/native.raw';raw.parent.mkdir(parents=True);raw.write_bytes(b'synthetic saved raw evidence\n')
        self.raw=raw
        pages=[dict(page=number,status='NATIVE_PRESERVED',route='native',words=[],source_sha256=self.task['input_sha256'],
                    raw_files={str(raw.relative_to(self.out)):local.sha(raw)} if number==1 else {}) for number in (1,2)]
        snapshot=dict(schema_version=1,operation_id=self.task['operation_id'],mode='local',source_pdf=str(self.source),
                      input_sha256=self.task['input_sha256'],revision=0,pages=pages,receipts=[],config_version=local.CONFIG_VERSION,
                      consumer_policy=local.CONSUMER_POLICY,font_path=self.config['font'],fallback_font_paths=[])
        self.folder=local.publish(self.out,snapshot)

    def tearDown(self):self.temp.cleanup()

    def files(self):return {str(p.relative_to(self.out)):local.sha(p) for p in self.out.rglob('*') if p.is_file()}

    def resume_without_worker(self):
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Completed pages must not restart')),patch.object(local,'invoke') as reader:
            result=local.run_task(self.task,self.config);reader.assert_not_called()
        return result

    def completion(self):
        result=local.load_task_completion(self.out)[0]
        result=dict(result,status='review_required');result.pop('completion_record_state',None)
        return result

    def test_missing_completion_restores_verified_draft_without_writing_a_receipt(self):
        before=self.files();result=self.resume_without_worker()
        self.assertEqual(result['status'],'completion_unverified');self.assertEqual(result['completion_record_state'],'missing')
        self.assertEqual(before,self.files());self.assertFalse((self.out/'completion.json').exists())
        self.assertEqual(result['page_results'],local.load_snapshot(self.out)[0]['pages'])
        with local.fitz.open(result['artifacts']['searchable_pdf']) as saved:
            self.assertEqual(len(saved),2);self.assertTrue(all('314159' in page.get_text() for page in saved))

    def test_partial_or_non_object_completion_bytes_are_preserved(self):
        for data in (b'{"partial":',b'[]',b'\xff'):
            with self.subTest(data=data):
                path=self.out/'completion.json';path.write_bytes(data);before=self.files()
                result=self.resume_without_worker()
                self.assertEqual(result['status'],'completion_unverified');self.assertEqual(result['completion_record_state'],'unreadable')
                self.assertEqual(path.read_bytes(),data);self.assertEqual(before,self.files())

    def test_recorded_completion_is_returned_unchanged_and_foreign_bindings_refuse(self):
        completion=self.completion();path=self.out/'completion.json';local.write(path,completion)
        self.assertEqual(self.resume_without_worker(),completion)
        for changed in (dict(input_sha256='0'*64),dict(operation_id='other'),dict(mode='paid'),dict(revision=False),dict(page_results=[]),dict(status='success')):
            with self.subTest(changed=changed):
                local.write(path,dict(completion,**changed));before=self.files()
                with self.assertRaisesRegex(ValueError,'COMPLETION_BINDING_MISMATCH'):self.resume_without_worker()
                self.assertEqual(before,self.files())

    def test_changed_source_config_and_task_pages_do_not_authorize_replay(self):
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Refusal must precede worker spawn')):
            with self.assertRaises(FileExistsError):local.run_task(self.task,dict(self.config,max_residual_regions=1))
            changed=dict(self.task,page_numbers=[2,1])
            local.write(self.out/'job.json',dict(schema_version=1,identity=local.digest(dict(task=changed,config=self.normalized)),task=changed,config=self.normalized))
            with self.assertRaisesRegex(ValueError,'CURRENT_TASK_BINDING_MISMATCH'):local.run_task(changed,self.config)
            self.source.write_bytes(b'changed source')
            with self.assertRaisesRegex(ValueError,'SOURCE_HASH_MISMATCH'):local.run_task(changed,self.config)

    def test_corrupt_published_artifact_or_raw_evidence_refuses_without_repair(self):
        for path,reason in ((self.folder/'text.txt','ARTIFACT_CORRUPT'),(self.raw,'RAW_EVIDENCE_CORRUPT')):
            original=path.read_bytes();path.write_bytes(b'tampered bytes remain visible')
            with self.assertRaisesRegex(ValueError,reason):self.resume_without_worker()
            self.assertEqual(path.read_bytes(),b'tampered bytes remain visible');path.write_bytes(original)

    def test_completion_symlink_never_reads_an_external_file(self):
        external=self.root/'outside.json';external.write_bytes(b'{"private":"never read"}')
        (self.out/'completion.json').symlink_to(external)
        with patch.object(local,'read',side_effect=AssertionError('Symlink contents must not be read')):
            snapshot,folder=local.load_snapshot(self.out)
            with self.assertRaisesRegex(ValueError,'COMPLETION_PATH_ESCAPE'):local.load_task_completion(self.out,snapshot,folder)
        self.assertEqual(external.read_bytes(),b'{"private":"never read"}')

    def test_legacy_identity_gaps_remain_unverified_and_never_use_a_success_record(self):
        snapshot,folder=local.load_snapshot(self.out)
        local.write(self.out/'completion.json',self.completion())
        for name in ('operation_id','mode'):
            legacy=dict(snapshot);legacy.pop(name)
            with patch.object(local,'read',side_effect=AssertionError('An unbound snapshot cannot establish a recorded success')):
                result,state=local.load_task_completion(self.out,legacy,folder)
            self.assertEqual(state,'identity_unverified');self.assertEqual(result['status'],'completion_unverified')


class SavedPageAdmissionTests(unittest.TestCase):
    """Real native bytes and durable checkpoints; no recognizer/provider/GUI."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name).resolve()
        self.source=self.root/'source.pdf';self.out=self.root/'operation';self.out.mkdir()
        self.identity=self.root/'non-executable-identity';self.identity.write_bytes(b'native fixture identity')
        with local.fitz.open() as doc:
            for number in (1,2):
                doc.new_page().insert_textbox(local.fitz.Rect(20,20,550,500),
                    f'Native durable source page {number} 314159 preserve every word without recognition. '*10)
            doc.save(self.source)
        self.task=dict(operation_id='saved-page-admission',input_pdf=str(self.source),input_sha256=local.sha(self.source),
                       page_numbers=[1],mode='local',output_directory=str(self.out),config_version=local.CONFIG_VERSION)
        self.config=dict(apple_helper=str(self.identity),tesseract=str(self.identity),
                         font=str(local.ROOT/'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf'))
        self.normalized=local.runtime_config(self.config)
        local.write(self.out/'job.json',dict(schema_version=1,task=self.task,config=self.normalized,
                    identity=local.digest(dict(task=self.task,config=self.normalized))))
        request=dict(input_pdf=str(self.source),input_sha256=self.task['input_sha256'],page=1,
                     mode='local',output=str(self.out),config=self.config)
        with patch.object(local,'invoke',side_effect=AssertionError('Native fixture cannot recognize')):
            self.page=local.worker(request)
        self.assertEqual(self.page['status'],'NATIVE_PRESERVED')
        self.resultpath=self.out/'raw/page-0001/page-result.json';self.image=self.resultpath.parent/'source.png'
        self.checkpoint(self.page)

    def tearDown(self):self.temp.cleanup()

    def checkpoint(self,page):
        local.write(self.out/'progress.json',dict(schema_version=1,completed=1,total=len(self.task['page_numbers']),page_results=[page]))

    def files(self):return {str(p.relative_to(self.out)):local.sha(p) for p in self.out.rglob('*') if p.is_file()}

    def refuse(self,reason):
        before=self.files()
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Refusal started a worker')) as spawn,\
             patch.object(local,'publish',side_effect=AssertionError('Refusal published a draft')) as publish:
            with self.assertRaisesRegex((ValueError,FileExistsError),reason):local.run_task(self.task,self.config)
            spawn.assert_not_called();publish.assert_not_called()
        self.assertEqual(before,self.files());self.assertEqual(local.sha(self.source),self.task['input_sha256'])
        self.assertFalse((self.out/'CURRENT.json').exists());self.assertFalse((self.out/'completion.json').exists())

    def test_actual_interruption_reuses_first_native_page_and_starts_only_remaining_page(self):
        out=self.root/'actual-operation';task=dict(self.task,output_directory=str(out),page_numbers=[1,2])
        owned=[];write=local.write
        class StopAfterFirstCheckpoint(Exception):pass
        def checkpoint(path,value):
            write(path,value)
            if Path(path)==out/'progress.json' and value['completed']==1:raise StopAfterFirstCheckpoint()
        def spawn(*a,**kw):
            proc=REAL_POPEN(*a,**kw);owned.append(proc);return proc
        with patch.object(local,'write',side_effect=checkpoint),patch.object(local.subprocess,'Popen',side_effect=spawn):
            with self.assertRaises(StopAfterFirstCheckpoint):local.run_task(task,self.config)
        self.assertEqual(len(owned),1);self.assertEqual(owned[0].returncode,0)
        self.assertTrue(owned[0].stdout.closed and owned[0].stderr.closed)
        with self.assertRaises(ChildProcessError):os.waitpid(owned[0].pid,os.WNOHANG)
        raw={str(p.relative_to(out)):local.sha(p) for p in (out/'raw/page-0001').rglob('*') if p.is_file()}
        job=local.sha(out/'job.json')
        with patch.object(local.subprocess,'Popen',side_effect=spawn):result=local.run_task(task,self.config)
        self.assertEqual(len(owned),2);self.assertEqual([p['status'] for p in result['page_results']],['NATIVE_PRESERVED']*2)
        self.assertEqual(raw,{str(p.relative_to(out)):local.sha(p) for p in (out/'raw/page-0001').rglob('*') if p.is_file()})
        self.assertEqual(local.sha(out/'job.json'),job);local.load_snapshot(out)
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Completed task replayed')):
            self.assertEqual(local.run_task(task,self.config),result)
        self.assertFalse(list((out/'raw').glob('**/*.call.json')))

    def test_changed_raw_refuses_before_any_state_write_or_publication(self):
        self.image.write_bytes(self.image.read_bytes()+b'changed cached bytes')
        self.refuse('SAVED_PAGE_RAW_CHANGED')

    def test_page_identity_status_image_and_word_shape_refuse_without_repair(self):
        original=self.resultpath.read_bytes()
        variants=[dict(page=2),dict(source_sha256='0'*64),dict(status='success'),dict(route='ocr'),
                  dict(image_sha256='0'*64),dict(native_text=1),
                  dict(words=[dict(id='w',text='x',bbox=[0,0,float('nan'),2])])]
        for changed in variants:
            with self.subTest(changed=changed):
                # Only this newly generated synthetic result is deliberately changed.
                self.resultpath.write_text(json.dumps(dict(self.page,**changed)))
                self.refuse('SAVED_PAGE_')
        self.resultpath.write_bytes(original)

    def test_checkpoint_mismatch_or_missing_parent_record_never_certifies_cached_text(self):
        local.write(self.resultpath,dict(self.page,native_text='edited cached text'))
        self.refuse('SAVED_PAGE_CHECKPOINT_MISMATCH')
        local.write(self.resultpath,self.page);(self.out/'progress.json').unlink()
        self.refuse('SAVED_PAGE_CHECKPOINT_UNVERIFIED')

    def test_job_body_cannot_claim_another_task_using_the_expected_digest(self):
        job=local.read(self.out/'job.json');job['task']=dict(self.task,page_numbers=[2])
        local.write(self.out/'job.json',job);self.refuse('TASK_JOB_BINDING_MISMATCH')

    def test_partial_and_later_invalid_page_refuse_before_an_earlier_missing_worker(self):
        self.task=dict(self.task,page_numbers=[2,1])
        local.write(self.out/'job.json',dict(schema_version=1,task=self.task,config=self.normalized,
                    identity=local.digest(dict(task=self.task,config=self.normalized))))
        self.image.write_bytes(self.image.read_bytes()+b'changed later page')
        self.refuse('SAVED_PAGE_RAW_CHANGED')
        self.resultpath.unlink();self.refuse('SAVED_PAGE_PARTIAL_UNVERIFIED')

    def test_paths_inventory_and_symlinks_refuse_without_reading_external_bytes(self):
        outside=self.root/'outside';outside.write_bytes(b'preserve outside')
        for manifest in ({'../outside':local.sha(outside)},{str(outside):local.sha(outside)},
                         {'raw/page-0002/foreign':local.sha(outside)}):
            local.write(self.resultpath,dict(self.page,raw_files=manifest));self.refuse('SAVED_PAGE_PATH_ESCAPE')
        local.write(self.resultpath,self.page)
        extra=self.resultpath.parent/'unregistered';extra.write_bytes(b'keep this file')
        self.refuse('SAVED_PAGE_RAW_INVENTORY_MISMATCH');extra.unlink()
        self.resultpath.unlink();self.resultpath.symlink_to(outside)
        self.refuse('SAVED_PAGE_PATH_ESCAPE');self.assertEqual(outside.read_bytes(),b'preserve outside')

    def test_runtime_change_and_saved_config_mismatch_are_readonly_refusals(self):
        self.identity.write_bytes(b'changed executable identity')
        self.refuse('SAVED_PAGE_RUNTIME_MISMATCH')
        self.identity.write_bytes(b'native fixture identity')
        beforepath=self.resultpath.parent/'runtime-version.json';afterpath=self.resultpath.parent/'runtime-after.json'
        before=local.read(beforepath);before['config']=dict(self.normalized,max_residual_regions=1)
        local.write(beforepath,before);local.write(afterpath,dict(before_sha256=local.sha(beforepath),identity=before,unchanged=True))
        raw=dict(self.page['raw_files']);raw.update({str(p.relative_to(self.out)):local.sha(p) for p in (beforepath,afterpath)})
        local.write(self.resultpath,dict(self.page,raw_files=raw));self.refuse('SAVED_PAGE_CONFIG_MISMATCH')

    def test_parent_failure_is_retained_and_never_replaced_by_worker_success(self):
        failed=dict(page=1,source_sha256=self.task['input_sha256'],status='FAILED',route='failed',words=[],error='parent control failure')
        local.parent_page_result(self.out,self.resultpath,failed);self.checkpoint(failed)
        raw={str(p.relative_to(self.out)):local.sha(p) for p in self.resultpath.parent.rglob('*') if p.is_file()}
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Failed page replayed')):
            result=local.run_task(self.task,self.config)
        self.assertEqual(result['status'],'failed');self.assertEqual(result['page_results'][0]['error'],'parent control failure')
        self.assertEqual(raw,{str(p.relative_to(self.out)):local.sha(p) for p in self.resultpath.parent.rglob('*') if p.is_file()})
        local.load_snapshot(self.out)

    def test_cancel_preserves_completed_cached_page_without_processing_missing_page(self):
        self.task=dict(self.task,page_numbers=[1,2])
        local.write(self.out/'job.json',dict(schema_version=1,task=self.task,config=self.normalized,
                    identity=local.digest(dict(task=self.task,config=self.normalized))))
        self.checkpoint(self.page);(self.out/'CANCEL').touch()
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Cancelled missing page started')):
            result=local.run_task(self.task,self.config)
        self.assertEqual(result['status'],'cancelled')
        self.assertEqual([p['status'] for p in result['page_results']],['NATIVE_PRESERVED','CANCELLED'])
        self.assertEqual(local.read(self.resultpath),self.page);local.load_snapshot(self.out)

    def test_cached_raw_change_during_remaining_native_worker_refuses_before_publish(self):
        self.task=dict(self.task,page_numbers=[1,2])
        local.write(self.out/'job.json',dict(schema_version=1,task=self.task,config=self.normalized,
                    identity=local.digest(dict(task=self.task,config=self.normalized))))
        self.checkpoint(self.page);owned=[];changed=[]
        def spawn(*a,**kw):
            proc=REAL_POPEN(*a,**kw);owned.append(proc);communicate=proc.communicate
            def collect(*args,**kwargs):
                result=communicate(*args,**kwargs)
                if proc.returncode==0 and not changed:
                    self.image.write_bytes(self.image.read_bytes()+b'changed while next native page completed')
                    changed.append(True)
                return result
            proc.communicate=collect;return proc
        with patch.object(local.subprocess,'Popen',side_effect=spawn),\
             patch.object(local,'publish',side_effect=AssertionError('Changed cached raw was published')) as publish:
            with self.assertRaisesRegex(ValueError,'SAVED_PAGE_RAW_CHANGED'):local.run_task(self.task,self.config)
            publish.assert_not_called()
        self.assertEqual(len(owned),1);self.assertEqual(owned[0].returncode,0)
        self.assertTrue(owned[0].stdout.closed and owned[0].stderr.closed)
        with self.assertRaises(ChildProcessError):os.waitpid(owned[0].pid,os.WNOHANG)
        self.assertEqual(local.read(self.out/'raw/page-0002/page-result.json')['status'],'NATIVE_PRESERVED')
        self.assertFalse((self.out/'CURRENT.json').exists());self.assertFalse((self.out/'completion.json').exists())
        self.assertEqual(local.sha(self.source),self.task['input_sha256'])


if __name__=='__main__':unittest.main()
