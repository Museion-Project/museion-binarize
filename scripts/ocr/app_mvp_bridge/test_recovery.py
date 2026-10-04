"""Actual disk/process crash tests with native fixtures. No OCR/provider/GUI."""
import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
import fitz
from .__main__ import main, sha
from .sessions import roots


class ContinuationTests(unittest.TestCase):
    """Explicit continuation and cancellation with preserved synthetic evidence.

    Negative fixtures render native pages and construct transparent saved
    evidence, without invoking a worker. Two positive cases use real native
    subprocesses. None of these fixtures exercises recognition or ordinary GUI.
    """
    def setUp(self):
        from scripts.ocr.mvp import local
        from .sessions import runtime_binding
        self.local = local
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'source.pdf'
        with fitz.open() as doc:
            for number in (1, 2, 3):
                doc.new_page().insert_textbox(fitz.Rect(20, 20, 550, 500),
                    f'Native continuation page {number} 314159 ' + 'preserve every source word without recognition ' * 8)
            doc.save(self.source)
        self.config = dict(session_root=str(self.root / 'sessions'), require_runtime_binding=True,
                           require_document_binding=True, local=dict(apple_helper=str(self.source), tesseract=str(self.source),
                           font=str(local.ROOT / 'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf')))
        self.request = dict(mode='local', input_pdf=str(self.source), input_sha256=sha(self.source),
                            document_id='reopened-source', document_page_count=3)
        self.folder = self.root / 'sessions' / ('a' * 32)
        self.folder.mkdir(parents=True)
        self.op = self.folder / 'operation'
        self.task = dict(operation_id=self.folder.name, input_pdf=str(self.source), input_sha256=sha(self.source),
                         page_numbers=[1, 2], mode='local', output_directory=str(self.op), config_version=local.CONFIG_VERSION)
        self.meta = dict(mode='local', source=str(self.source), source_sha256=sha(self.source),
                         client_operation_id='c' * 32, provenance='transparent-synthetic-evidence',
                         runtime_binding=runtime_binding(self.config), document_binding=dict(source_sha256=sha(self.source),
                         source_pdf=str(self.source), source_page_count=3, physical_pages=[1, 2]))
        from .__main__ import register_local_operation
        local.write(self.folder / 'desktop-session.json', self.meta)
        register_local_operation(self.folder, self.task, self.config['local'])
        directory = self.op / 'raw/page-0001'
        directory.mkdir(parents=True)
        with fitz.open(self.source) as doc:
            doc[0].get_pixmap().save(directory / 'source.png')
            text = doc[0].get_text()
        normalized = local.runtime_config(self.config['local'])
        runtime = local.worker_runtime_identity(normalized)
        local.write(directory / 'runtime-version.json', runtime)
        local.write(directory / 'runtime-after.json', dict(unchanged=True, identity=runtime,
                    before_sha256=sha(directory / 'runtime-version.json')))
        self.page = dict(page=1, status='NATIVE_PRESERVED', route='native', words=[], native_text=text,
                         source_sha256=sha(self.source), image_path=str(directory / 'source.png'),
                         image_sha256=sha(directory / 'source.png'), raw_files={str(p.relative_to(self.op)): sha(p) for p in directory.iterdir()})
        local.write(directory / 'page-result.json', self.page)
        local.write(self.op / 'progress.json', dict(schema_version=1, completed=1, total=2, page_results=[self.page]))

    def tearDown(self):
        self.temp.cleanup()

    def files(self):
        return {str(p.relative_to(self.folder)): sha(p) for p in self.folder.rglob('*') if p.is_file()}

    def request_for(self, token='d' * 32):
        observed = main(dict(self.request, action='reload', session_id=self.folder.name), self.config)
        self.assertTrue(observed['continuation_available'])
        return dict(self.request, action='continue', session_id=self.folder.name,
                    client_operation_id=token, continuation_state_sha256=observed['continuation_state_sha256'])

    def refuse(self, request, reason, config=None):
        from unittest.mock import patch
        before = self.files()
        with patch.object(self.local.subprocess, 'Popen', side_effect=AssertionError('Refusal started a worker')) as spawn:
            with self.assertRaisesRegex((ValueError, FileExistsError), reason):
                main(request, config or self.config)
            spawn.assert_not_called()
        self.assertEqual(before, self.files())

    def test_readonly_resume_has_no_processing_side_effects(self):
        from unittest.mock import patch
        before = self.files()
        with patch.object(self.local, 'run_task', side_effect=AssertionError('Read-only resume processed')):
            result = main(dict(self.request, action='resume'), self.config)
        self.assertTrue(result['continuation_available'])
        self.assertFalse(result['draft_published'])
        self.assertEqual([p['status'] for p in result['pages']], ['NATIVE_PRESERVED', 'NOT_PROCESSED'])
        self.assertEqual(before, self.files())

    def test_fresh_local_metadata_is_written_once_even_after_processing_returns(self):
        from unittest.mock import patch
        from . import __main__ as bridge
        write = bridge.write;writes = []
        def preserve_original(path, value):
            if Path(path).name == 'desktop-session.json':
                self.assertFalse(Path(path).exists(), 'Original metadata was rewritten after processing')
                writes.append(Path(path))
            write(path, value)
        with patch.object(bridge, 'write', side_effect=preserve_original), patch.object(self.local, 'run_task', return_value={}), patch.object(bridge, 'view', return_value={}):
            main(dict(self.request, action='start', pages=[1, 2], client_operation_id='b' * 32), self.config)
        self.assertEqual(len(writes), 1)

    def test_actual_parent_interruption_continues_only_remaining_native_page(self):
        from unittest.mock import patch
        write = self.local.write
        owned = []
        real_popen = subprocess.Popen
        class Interrupted(Exception): pass
        def checkpoint(path, value):
            write(path, value)
            if Path(path).name == 'progress.json' and value['completed'] == 1:
                raise Interrupted()
        def spawn(*a, **kw):
            proc = real_popen(*a, **kw);owned.append(proc);return proc
        with patch.object(self.local, 'write', side_effect=checkpoint), patch.object(self.local.subprocess, 'Popen', side_effect=spawn):
            with self.assertRaises(Interrupted):
                main(dict(self.request, action='start', pages=[1, 2], client_operation_id='b' * 32), self.config)
        self.folder = next(p.parent for p in (self.root / 'sessions').glob('*/desktop-session.json') if p.parent.name != 'a' * 32)
        self.op = self.folder / 'operation'
        originals = {n: sha(self.folder / n) for n in ('desktop-session.json', 'task-request.json', 'operation/job.json')}
        first = {str(p.relative_to(self.folder)): sha(p) for p in (self.op / 'raw/page-0001').iterdir()}
        with patch.object(self.local.subprocess, 'Popen', side_effect=spawn):
            result = main(self.request_for(), self.config)
        self.assertEqual(len(owned), 2)
        self.assertEqual([p['status'] for p in result['pages']], ['NATIVE_PRESERVED'] * 2)
        for n, h in {**originals, **first}.items(): self.assertEqual(sha(self.folder / n), h)
        for proc in owned:
            self.assertEqual(proc.returncode, 0)
            self.assertTrue(proc.stdout.closed and proc.stderr.closed)
            with self.assertRaises(ChildProcessError): os.waitpid(proc.pid, os.WNOHANG)
        self.assertFalse(list((self.op / 'raw').glob('**/*.call.json')))
        record = json.loads((self.folder / ('processing-attempt-' + 'd' * 32 + '.json')).read_text())
        self.assertEqual(record['remaining_pages'], [2])
        self.assertEqual(list(record['retained_pages']), ['1'])
        self.assertEqual(record['document_id'], 'reopened-source')
        self.refuse(dict(self.request, action='continue', session_id=self.folder.name,
                         client_operation_id='e' * 32, continuation_state_sha256='0' * 64), 'TASK_ALREADY_PUBLISHED')

    def test_changed_binding_and_missing_identity_are_rejected_without_writes(self):
        request = self.request_for()
        for extra, reason in ((dict(pages=[2]), 'PAGES_FIXED'), (dict(document_page_count=2), 'PAGE_COUNT_CHANGED'),
                              (dict(document_id=''), 'DOCUMENT_BINDING_REQUIRED'),
                              (dict(client_operation_id=None), 'OPERATION_REQUIRED'),
                              (dict(continuation_state_sha256=None), 'STATE_REQUIRED')):
            with self.subTest(extra=extra): self.refuse(dict(request, **extra), reason)
        changed = copy.deepcopy(self.config);changed['local']['max_residual_regions'] = 1
        self.refuse(request, 'RUNTIME_UNVERIFIED_OR_CHANGED', changed)
        self.source.write_bytes(self.source.read_bytes() + b'changed')
        self.refuse(request, 'HASH')

    def test_partial_corrupt_cancelled_and_stale_checkpoint_refuse_before_attempt(self):
        request = self.request_for()
        image = self.op / 'raw/page-0001/source.png';original = image.read_bytes()
        image.write_bytes(original + b'changed');self.refuse(request, 'NOT_AVAILABLE');image.write_bytes(original)
        worker = self.op / 'raw/worker-2.json';worker.write_text('{"prior":"preserve"}')
        self.refuse(self.request_for(), 'PARTIAL_WORKER_EVIDENCE');worker.unlink()
        checkpoint = self.op / 'progress.json';original = checkpoint.read_bytes()
        checkpoint.write_text('{"malformed":"preserve"}')
        self.refuse(self.request_for(), 'CHECKPOINT_UNVERIFIED');checkpoint.write_bytes(original)
        cancel = self.op / 'CANCEL';cancel.touch();self.refuse(request, 'NOT_AVAILABLE')
        self.assertFalse((self.folder / 'active-processing.json').exists())

    def test_whole_job_lease_rejects_duplicate_producer_before_attempt(self):
        request = self.request_for()
        with self.local.task_producer(self.op): self.refuse(request, 'TASK_PRODUCER_BUSY')
        self.assertFalse((self.folder / 'active-processing.json').exists())

    def test_old_token_cannot_cancel_new_attempt_and_new_pending_cancel_starts_no_worker(self):
        from unittest.mock import patch
        import io
        request = self.request_for();outer = self
        class CancelOnAdmission(io.StringIO):
            def write(self, value):
                if value.startswith('{'):
                    event = json.loads(value)
                    if event.get('event') == 'session':
                        outer.refuse(dict(outer.request, action='cancel', session_id=outer.folder.name,
                                         client_operation_id='c' * 32), 'CANCEL_OPERATION_MISMATCH')
                        main(dict(outer.request, action='cancel', session_id=outer.folder.name,
                                  client_operation_id=event['client_operation_id']), outer.config)
                return super().write(value)
        original = sha(self.folder / 'desktop-session.json')
        with patch.object(sys, 'stderr', CancelOnAdmission()), patch.object(self.local.subprocess, 'Popen', side_effect=AssertionError('Pending cancel started worker')) as spawn:
            result = main(request, self.config)
            spawn.assert_not_called()
        self.assertEqual(result['status'], 'cancelled')
        self.assertEqual(sha(self.folder / 'desktop-session.json'), original)
        self.assertTrue((self.op / 'CANCEL').exists())
        self.assertFalse(list((self.op / 'raw').glob('worker-2*')))

    def test_stale_display_and_reused_attempt_token_preserve_records(self):
        from unittest.mock import patch
        request = self.request_for()
        (self.op / 'raw/new-evidence.txt').write_text('preserve prior evidence')
        self.refuse(request, 'STATE_CHANGED')
        request = self.request_for()
        class AfterAdmission(Exception): pass
        def interrupted(*a, **kw):
            if kw.get('file') is sys.stderr: raise AfterAdmission()
        with patch('builtins.print', side_effect=interrupted):
            with self.assertRaises(AfterAdmission): main(request, self.config)
        self.assertTrue((self.folder / 'active-processing.json').exists())
        self.refuse(self.request_for(), 'TOKEN_REUSED')
        self.refuse(dict(self.request, action='cancel', session_id=self.folder.name), 'CANCEL_OPERATION_MISMATCH')

    def test_cancel_waiting_on_control_lock_cannot_apply_old_token_after_publication(self):
        from unittest.mock import patch
        from . import sessions
        import threading
        blocked = threading.Event();errors = []
        replace = os.replace;flock = sessions.fcntl.flock
        class AfterAdmission(Exception): pass
        def cancel():
            try:main(dict(self.request, action='cancel', session_id=self.folder.name, client_operation_id='c' * 32), self.config)
            except BaseException as exc:errors.append(str(exc))
        contender = threading.Thread(target=cancel, name='old-cancel-contender')
        def observe_flock(*a):
            try:return flock(*a)
            except BlockingIOError:
                if threading.current_thread() is contender:blocked.set()
                raise
        def publish(a, b):
            if Path(b) == self.folder / 'active-processing.json':
                contender.start();self.assertTrue(blocked.wait(1), 'Old cancellation did not contend on the real lock')
            return replace(a, b)
        def stop_before_worker(*a, **kw):
            if kw.get('file') is sys.stderr:raise AfterAdmission()
        try:
            with patch.object(sessions.fcntl, 'flock', side_effect=observe_flock), patch.object(os, 'replace', side_effect=publish), patch('builtins.print', side_effect=stop_before_worker):
                with self.assertRaises(AfterAdmission):main(self.request_for(), self.config)
                contender.join(2)
            self.assertFalse(contender.is_alive());self.assertEqual(errors, ['CANCEL_OPERATION_MISMATCH'])
            self.assertFalse((self.op / 'CANCEL').exists())
        finally:
            if contender.ident is not None:contender.join(3)

    def test_two_explicit_attempts_keep_prior_record_and_completed_page(self):
        from unittest.mock import patch
        self.task['page_numbers'] = [1, 2, 3]
        self.meta['document_binding']['physical_pages'] = [1, 2, 3]
        self.local.write(self.folder / 'desktop-session.json', self.meta)
        self.local.write(self.op / 'job.json', dict(schema_version=1, task=self.task,
                         config=self.local.runtime_config(self.config['local']), identity=self.local.digest(dict(task=self.task, config=self.local.runtime_config(self.config['local'])))))
        self.local.write(self.op / 'progress.json', dict(schema_version=1, completed=1, total=3, page_results=[self.page]))
        write = self.local.write;owned = [];real_popen = subprocess.Popen
        class Interrupted(Exception): pass
        def checkpoint(path, value):
            write(path, value)
            if Path(path) == self.op / 'progress.json' and value['completed'] == 2: raise Interrupted()
        def spawn(*a, **kw):
            proc = real_popen(*a, **kw);owned.append(proc);return proc
        with patch.object(self.local, 'write', side_effect=checkpoint), patch.object(self.local.subprocess, 'Popen', side_effect=spawn):
            with self.assertRaises(Interrupted): main(self.request_for(), self.config)
        first_record = self.folder / ('processing-attempt-' + 'd' * 32 + '.json');first_sha = sha(first_record)
        page2 = {str(p.relative_to(self.folder)): sha(p) for p in (self.op / 'raw/page-0002').iterdir()}
        with patch.object(self.local.subprocess, 'Popen', side_effect=spawn): result = main(self.request_for('e' * 32), self.config)
        self.assertEqual(len(owned), 2)
        self.assertEqual(sha(first_record), first_sha)
        for n, h in page2.items(): self.assertEqual(sha(self.folder / n), h)
        self.assertEqual([p['status'] for p in result['pages']], ['NATIVE_PRESERVED'] * 3)
        active = json.loads((self.folder / 'active-processing.json').read_text())
        record = json.loads((self.folder / active['record']).read_text())
        self.assertEqual(record['previous_active_record']['record_sha256'], first_sha)
        self.assertEqual(record['remaining_pages'], [3])
        self.assertEqual(record['previous_checkpoint']['completed'], 2)
        for proc in owned:
            self.assertEqual(proc.returncode, 0);self.assertTrue(proc.stdout.closed and proc.stderr.closed)
            with self.assertRaises(ChildProcessError): os.waitpid(proc.pid, os.WNOHANG)


class ProcessingRecoveryTests(unittest.TestCase):
    """Transparent synthetic cache, actual native retries, no recognition/GUI."""
    setUp = ContinuationTests.setUp
    tearDown = ContinuationTests.tearDown
    files = ContinuationTests.files

    def request_for_recovery(self):
        result = main(dict(self.request, action='reload', session_id=self.folder.name), self.config)
        self.assertTrue(result['processing_recovery_available'], result.get('processing_recovery_blocker'))
        return dict(self.request, action='recover-processing', session_id=self.folder.name,
                    client_operation_id='f' * 32, continuation_state_sha256=result['processing_recovery_state_sha256'])

    def assert_old_preserved(self, before):
        after = self.files()
        self.assertEqual({n: after[n] for n in before}, before)
        self.assertEqual(set(after) - set(before), {'PROCESSING_SUCCESSOR.json'})

    def run_one_native_retry(self, request):
        from unittest.mock import patch
        real = subprocess.Popen;owned = []
        def spawn(*a, **kw):
            proc = real(*a, **kw)
            if '--worker' in a[0]:owned.append(proc)
            return proc
        with patch.object(self.local.subprocess, 'Popen', side_effect=spawn):
            result = main(request, self.config)
        self.assertEqual(len(owned), 1)
        for proc in owned:
            self.assertEqual(proc.returncode, 0)
            self.assertTrue(proc.stdout.closed and proc.stderr.closed)
            with self.assertRaises(ChildProcessError):os.waitpid(proc.pid, os.WNOHANG)
        if os.environ.get('MUSEION_RECOVERY_TEST_EVIDENCE'):
            path = Path(os.environ['MUSEION_RECOVERY_TEST_EVIDENCE'])
            with path.open('a') as out:
                out.write(json.dumps(dict(test=self.id(), worker_pid=owned[0].pid, returncode=0,
                    stdout_closed=True, stderr_closed=True, reaped=True, recognition=0)) + '\n')
        return result

    def test_cancelled_unpublished_recovery_preserves_original_and_only_runs_missing_page(self):
        (self.op / 'CANCEL').touch()
        request = self.request_for_recovery();before = self.files()
        result = self.run_one_native_retry(request)
        self.assertEqual([p['status'] for p in result['pages']], ['NATIVE_PRESERVED'] * 2)
        self.assertNotEqual(result['session_id'], self.folder.name)
        self.assert_old_preserved(before)
        child = self.folder.parent / result['session_id']
        self.assertTrue((self.op / 'CANCEL').is_file())
        self.assertFalse((child / 'operation/CANCEL').exists())
        self.assertEqual([p.name for p in (child / 'operation/raw').glob('worker-*.json')], ['worker-2.json'])
        self.assertEqual(result['inherited_review']['source_session_id'], self.folder.name)
        self.assertEqual(result['completion_record_state'], 'recorded')
        self.assertFalse(list((child / 'operation/raw').glob('**/*.call.json')))

    def test_failed_first_page_reuses_sparse_reviewed_second_page_and_keeps_receipts(self):
        from scripts.ocr.mvp.store import publish, review_save
        from scripts.ocr.mvp.core import CONSUMER_POLICY
        import shutil
        directory = self.op / 'raw/page-0002';directory.mkdir()
        for p in (self.op / 'raw/page-0001').iterdir():
            if p.name != 'page-result.json':shutil.copyfile(p, directory / p.name)
        with fitz.open(self.source) as doc:doc[1].get_pixmap().save(directory / 'source.png')
        good = dict(self.page, page=2, route='ocr', status='OCR_DRAFT', width=1000, height=1000,
                    image_path=str(directory / 'source.png'), image_sha256=sha(directory / 'source.png'),
                    native_text='', words=[dict(id='manual-word', text='before', bbox=[1, 1, 30, 20],
                    engine='synthetic-test-seed', source_members=[], review=True)],
                    raw_files={str(p.relative_to(self.op)):sha(p) for p in directory.iterdir()},
                    fixture_provenance='Handmade word fixture, not OCR or source-quality evidence')
        self.local.write(directory / 'page-result.json', good)
        bad = dict(page=1, status='FAILED', route='failed', words=[], source_sha256=sha(self.source),
                   error='synthetic known failure', review_reasons=[])
        self.local.parent_page_result(self.op, self.op / 'raw/page-0001/page-result.json', bad)
        bad = self.local.saved_page_result(self.op, 1, self.task, self.local.runtime_config(self.config['local']))
        self.local.write(self.op / 'progress.json', dict(schema_version=1, completed=2, total=2, page_results=[bad, good]))
        snapshot = dict(schema_version=1, operation_id=self.folder.name, mode='local', source_pdf=str(self.source),
                        input_sha256=sha(self.source), revision=0, pages=[bad, good], receipts=[],
                        config_version=self.local.CONFIG_VERSION, consumer_policy=CONSUMER_POLICY,
                        font_path=self.config['local']['font'], fallback_font_paths=[])
        revision = publish(self.op, snapshot)
        self.local.write(self.op / 'completion.json', dict(operation_id=self.folder.name, mode='local',
            input_sha256=sha(self.source), revision=0, status='failed', page_results=snapshot['pages'],
            artifacts=dict(searchable_pdf=str(revision / 'searchable.pdf'))))
        review_save(self.op, dict(expected_revision=0, input_sha256=sha(self.source),
            actions=[dict(page=2, member_id='manual-word', action='change', text='corrected')]))
        request = self.request_for_recovery();before = self.files()
        result = self.run_one_native_retry(request)
        self.assert_old_preserved(before)
        self.assertEqual(result['pages'][1]['words'][0]['text'], 'corrected')
        self.assertEqual(result['pages'][1]['words'][0]['review_status'], 'user_action_recorded')
        self.assertEqual(result['inherited_review']['source_revision'], 1)
        self.assertEqual(result['inherited_review']['receipts'][0]['actions'][0]['text'], 'corrected')
        child = self.folder.parent / result['session_id']
        self.assertEqual([p.name for p in (child / 'operation/raw').glob('worker-*.json')], ['worker-1.json'])
        self.assertEqual(result['pages'][1]['reused_result']['previous_session_id'], self.folder.name)

    def test_readonly_cancelled_view_does_not_create_attempt(self):
        (self.op / 'CANCEL').touch();before = self.files()
        self.request_for_recovery()
        self.assertEqual(before, self.files())
        self.assertEqual(len(list(self.folder.parent.iterdir())), 1)

    def refuse_recovery(self, request, reason):
        from unittest.mock import patch
        before = self.files();sessions = set(self.folder.parent.iterdir())
        with patch.object(self.local.subprocess, 'Popen', side_effect=AssertionError('Refusal started a worker')):
            with self.assertRaisesRegex((ValueError, FileExistsError), reason):main(request, self.config)
        self.assertEqual(self.files(), before)
        self.assertEqual(set(self.folder.parent.iterdir()), sessions)

    def test_stale_state_original_producer_and_reused_token_refuse_before_new_session(self):
        (self.op / 'CANCEL').touch();request = self.request_for_recovery()
        self.refuse_recovery(dict(request, client_operation_id='c' * 32), 'TOKEN_REUSED')
        with self.local.task_producer(self.op):self.refuse_recovery(request, 'PRODUCER_BUSY')
        (self.op / 'extra-preserved-note').write_text('new prior evidence')
        self.refuse_recovery(request, 'STATE_CHANGED')

    def test_partial_worker_and_cleanup_failure_remain_unverified(self):
        (self.op / 'CANCEL').touch()
        request = self.request_for_recovery()
        (self.op / 'raw/worker-2.json').write_text('{}')
        self.refuse_recovery(request, 'PARTIAL_WORKER_UNVERIFIED')
        (self.op / 'worker-cleanup-failure.json').write_text('{}')
        self.refuse_recovery(request, 'CLEANUP_UNVERIFIED')

    def test_explicit_new_attempt_can_be_cancelled_only_with_its_new_identity(self):
        from unittest.mock import patch
        (self.op / 'CANCEL').touch();request = self.request_for_recovery();before = self.files()
        outer = self;admitted = []
        class CancelOnAdmission:
            def write(self, text):
                if text.strip():
                    event = json.loads(text);admitted.append(event)
                    outer.assertEqual(event['previous_session_id'], outer.folder.name)
                    wrong = dict(outer.request, action='cancel', session_id=event['session_id'], client_operation_id='c' * 32)
                    with outer.assertRaisesRegex(ValueError, 'CANCEL_OPERATION_MISMATCH'):main(wrong, outer.config)
                    main(dict(wrong, client_operation_id=request['client_operation_id']), outer.config)
                return len(text)
            def flush(self):pass
        with patch.object(sys, 'stderr', CancelOnAdmission()), patch.object(self.local.subprocess, 'Popen', side_effect=AssertionError('Pending cancellation launched a worker')):
            result = main(request, self.config)
        self.assertEqual(result['status'], 'cancelled');self.assertEqual(len(admitted), 1)
        self.assert_old_preserved(before)
        self.refuse_recovery(dict(request, client_operation_id='d' * 32), 'SUCCESSOR_EXISTS')
        old_view = main(dict(self.request, action='reload', session_id=self.folder.name), self.config)
        self.assertFalse(old_view['processing_recovery_available'])
        self.assertEqual(old_view['processing_recovery_next_session_id'], result['session_id'])
        from .sessions import processing_recovery_plan
        child = self.folder.parent / result['session_id']
        plan = processing_recovery_plan(child, json.loads((child / 'desktop-session.json').read_text()), self.config)
        self.assertEqual(plan['inherited_review']['previous'], result['inherited_review'])
        self.assertEqual(plan['inherited_review']['previous']['source_session_id'], self.folder.name)
        stale = dict(self.config, local=dict(self.config['local'], max_residual_regions=1))
        viewed = main(dict(self.request, action='reload', session_id=child.name), stale)
        self.assertFalse(viewed['runtime_compatible'])
        self.assertEqual(viewed['inherited_review'], result['inherited_review'])
        self.assertFalse(viewed['processing_recovery_available'])

    def test_tampered_reuse_manifest_refuses_without_reprocessing(self):
        from unittest.mock import patch
        (self.op / 'CANCEL').touch();request = self.request_for_recovery()
        class BeforeWorker(Exception):pass
        with patch.object(self.local, 'run_task', side_effect=BeforeWorker):
            with self.assertRaises(BeforeWorker):main(request, self.config)
        successor = json.loads((self.folder / 'PROCESSING_SUCCESSOR.json').read_text())['session_id']
        child = self.folder.parent / successor
        path = child / 'operation/reused-pages.json';path.write_bytes(path.read_bytes() + b' ')
        with patch.object(self.local.subprocess, 'Popen', side_effect=AssertionError('Tampered cache reprocessed')):
            with self.assertRaisesRegex(ValueError, 'MANIFEST_CHANGED'):
                self.local.run_task(json.loads((child / 'operation/job.json').read_text())['task'], self.config['local'])

    def test_changed_completed_checkpoint_preserves_old_records_and_refuses_recovery(self):
        (self.op / 'CANCEL').touch();request = self.request_for_recovery()
        checkpoint = json.loads((self.op / 'progress.json').read_text())
        checkpoint['page_results'][0]['native_text'] += ' changed checkpoint'
        self.local.write(self.op / 'progress.json', checkpoint)
        self.refuse_recovery(request, 'CHECKPOINT_MISMATCH')


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'native-source.pdf'
        with fitz.open() as doc:
            for _ in range(2):
                doc.new_page().insert_textbox(fitz.Rect(20, 20, 550, 500),
                    'Native restart 314159 ' + 'preserve every source word without recognition ' * 8)
            doc.save(self.source)
        repo = Path(__file__).resolve().parents[3]
        self.config = dict(session_root=str(self.root / 'private-sessions'), persistent_sessions=True,
                           require_runtime_binding=True, local=dict(
                               apple_helper=str(self.source),tesseract=str(self.source),
                               font=str(repo / 'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf')))
        self.request = dict(mode='local', input_pdf=str(self.source), input_sha256=sha(self.source))
        self.dest = self.root / 'saved.pdf'

    def tearDown(self):
        self.temp.cleanup()

    def start(self):
        result=main(dict(self.request, action='start', pages=[1, 2], client_operation_id='c' * 32), self.config)
        self.assertTrue(all(p['status']=='NATIVE_PRESERVED' for p in result['pages']))
        return result

    def crash(self, *, receipt_present=False):
        path = self.root / 'crash-input.json'
        path.write_text(json.dumps(dict(config=self.config, request=self.request, dest=str(self.dest),
                                       receipt_present=receipt_present)))
        script = """
import json,os,sys
from pathlib import Path
from scripts.ocr.app_mvp_bridge.__main__ import main
p=json.load(open(sys.argv[1]));r=p['request'];c=p['config']
s=main(dict(r,action='start',pages=[1,2],client_operation_id='c'*32),c)
assert all(page['status']=='NATIVE_PRESERVED' for page in s['pages'])
link=os.link
def crash_link(a,b):
 if Path(b).name.startswith('save-'):
  if p['receipt_present']:link(a,b)
  os._exit(88)
 return link(a,b)
os.link=crash_link
main(dict(r,action='save',session_id=s['session_id'],expected_revision=s['revision'],output_path=p['dest']),c)
raise AssertionError('crash did not fire')
"""
        proc = subprocess.run([sys.executable, '-B', '-c', script, str(path)], text=True,
                              capture_output=True, timeout=15)
        self.assertEqual(proc.returncode, 88, proc.stderr + proc.stdout)
        resumed = main(dict(self.request, action='resume'), self.config)
        self.assertEqual(len(resumed['pending_saves']), 1)
        return resumed

    def recover_request(self, resumed):
        pending = resumed['pending_saves'][0]
        return dict(self.request, action='recover-save', session_id=resumed['session_id'],
                    expected_revision=resumed['revision'], save_journal_id=pending['journal_id'],
                    save_journal_sha256=pending['journal_sha256'])

    def test_crash_copy_recovers_receipt_and_literal_search_without_ocr(self):
        resumed = self.crash()
        self.assertTrue(resumed['runtime_compatible'])
        pending = resumed['pending_saves'][0]
        self.assertEqual(pending['status'], 'complete_copy_pending_receipt')
        self.assertTrue(pending['recoverable'])
        before = sha(self.dest)
        folder = Path(self.config['session_root']) / resumed['session_id']
        journals = {p.name: sha(p) for p in folder.glob('save-pending-*.json')}
        request = self.recover_request(resumed)
        recovered = main(request, self.config)
        self.assertTrue(recovered['save_recovered'])
        self.assertEqual(sha(self.dest), before)
        self.assertEqual(journals, {p.name: sha(p) for p in folder.glob('save-pending-*.json')})
        self.assertEqual(recovered['pending_saves'][0]['status'], 'recovered')
        # Repeat the exact operation: no overwrite and no duplicate recovery marker.
        main(request, self.config)
        self.assertEqual(len(list(folder.glob('save-recovery-*.json'))), 1)
        self.assertFalse(list((folder / 'operation/raw').glob('**/*.call.json')))
        hits = main(dict(self.request, action='search', session_id=resumed['session_id'],
                         saved_pdf=str(self.dest), query='314159'), self.config)
        self.assertEqual(sum(h['consumer'] == 'PDF exact Unicode' for h in hits['hits']), 2)
        with fitz.open(self.dest) as doc:
            self.assertEqual(len(doc), 2)
            self.assertTrue(all('314159' in p.get_text() for p in doc))

    def test_crash_after_receipt_preserves_receipt_and_records_recovery(self):
        resumed = self.crash(receipt_present=True)
        self.assertEqual(resumed['pending_saves'][0]['status'], 'receipt_present')
        folder = Path(self.config['session_root']) / resumed['session_id']
        receipts = {p.name: sha(p) for p in folder.glob('save-*.json')}
        main(self.recover_request(resumed), self.config)
        self.assertTrue(all(sha(folder / name) == value for name, value in receipts.items()))

    def test_same_bytes_replacement_is_unresolved_and_never_deleted(self):
        resumed = self.crash()
        data = self.dest.read_bytes()
        replacement = self.root / 'replacement.pdf'
        replacement.write_bytes(data)
        replacement.replace(self.dest)
        latest = main(dict(self.request, action='reload', session_id=resumed['session_id']), self.config)
        self.assertFalse(latest['pending_saves'][0]['recoverable'])
        with self.assertRaisesRegex(ValueError, 'OUTPUT_CHANGED_OR_REPLACED'):
            main(self.recover_request(resumed), self.config)
        self.assertEqual(self.dest.read_bytes(), data)

    def test_missing_or_modified_output_is_not_recreated(self):
        resumed = self.crash()
        self.dest.unlink()
        with self.assertRaisesRegex(ValueError, 'OUTPUT_MISSING'):
            main(self.recover_request(resumed), self.config)
        self.assertFalse(self.dest.exists())
        self.dest.write_bytes(b'user content')
        with self.assertRaises(ValueError):
            main(self.recover_request(resumed), self.config)
        self.assertEqual(self.dest.read_bytes(), b'user content')

    def test_journal_token_and_receipt_scope_are_bound(self):
        resumed = self.crash()
        request = self.recover_request(resumed)
        with self.assertRaisesRegex(ValueError, 'JOURNAL_ID'):
            main(dict(request, save_journal_id='../outside'), self.config)
        with self.assertRaisesRegex(ValueError, 'JOURNAL_CHANGED'):
            main(dict(request, save_journal_sha256='0' * 64), self.config)
        folder = Path(self.config['session_root']) / resumed['session_id']
        journal = folder / request['save_journal_id']
        data = json.loads(journal.read_text())
        data['receipt_path'] = str(self.root / 'outside-receipt.json')
        journal.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'RECEIPT_SCOPE'):
            main(dict(request, save_journal_sha256=sha(journal)), self.config)
        self.assertFalse((self.root / 'outside-receipt.json').exists())

    def test_cancellation_revision_or_source_change_blocks_recovery(self):
        resumed = self.crash()
        request = self.recover_request(resumed)
        with self.assertRaisesRegex(ValueError, 'STALE_REVISION'):
            main(dict(request, expected_revision=resumed['revision'] + 1), self.config)
        folder = Path(self.config['session_root']) / resumed['session_id']
        (folder / 'operation/CANCEL').touch()
        with self.assertRaisesRegex(ValueError, 'CANCEL_CHANGED'):
            main(request, self.config)
        self.source.write_bytes(b'changed original')
        with self.assertRaisesRegex(ValueError, 'SOURCE_HASH'):
            main(request, self.config)

    def test_actual_review_revision_change_does_not_rebind_an_old_journal(self):
        resumed = self.crash()
        folder = Path(self.config['session_root']) / resumed['session_id']
        from scripts.ocr.mvp.store import review_save
        review_save(folder / 'operation', dict(expected_revision=resumed['revision'],
                    input_sha256=self.request['input_sha256'], actions=[]))
        latest = main(dict(self.request, action='reload', session_id=resumed['session_id']), self.config)
        self.assertEqual(latest['revision'], resumed['revision'] + 1)
        self.assertEqual(latest['pending_saves'][0]['reason'], 'RECOVERY_STALE_REVISION')
        request = self.recover_request(resumed)
        with self.assertRaisesRegex(ValueError, 'RECOVERY_STALE_REVISION'):
            main(dict(request, expected_revision=latest['revision']), self.config)

    def test_conflicting_receipt_stays_unchanged(self):
        resumed = self.crash()
        folder = Path(self.config['session_root']) / resumed['session_id']
        journal = json.loads((folder / resumed['pending_saves'][0]['journal_id']).read_text())
        receipt = Path(journal['receipt_path'])
        receipt.write_text('{"user":"preserve"}')
        with self.assertRaisesRegex(ValueError, 'RECEIPT_CONFLICT'):
            main(self.recover_request(resumed), self.config)
        self.assertEqual(receipt.read_text(), '{"user":"preserve"}')

    def test_changed_runtime_is_readonly_and_no_implicit_restart(self):
        started = self.start()
        changed = copy.deepcopy(self.config)
        changed['local']['max_residual_regions'] = 1
        resumed = main(dict(self.request, action='resume'), changed)
        self.assertFalse(resumed['runtime_compatible'])
        for action in ('save', 'review', 'recover-save'):
            with self.assertRaisesRegex(ValueError, 'RUNTIME_UNVERIFIED_OR_CHANGED'):
                main(dict(self.request, action=action, session_id=started['session_id']), changed)
        self.assertFalse(self.dest.exists())

    def test_old_root_resume_and_followup_keep_history_without_migration(self):
        started = self.start()
        old_root = Path(self.config['session_root'])
        config = dict(self.config, session_root=str(self.root / 'new-private'),
                      legacy_session_roots=[str(old_root)])
        resumed = main(dict(self.request, action='resume'), config)
        self.assertEqual(resumed['session_id'], started['session_id'])
        main(dict(self.request, action='reload', session_id=started['session_id']), config)
        self.assertTrue((old_root / started['session_id']).exists())
        self.assertFalse(Path(config['session_root']).exists())

    def test_malformed_journal_does_not_hide_other_recovery_records(self):
        started = self.start()
        folder = Path(self.config['session_root']) / started['session_id']
        for index, payload in enumerate(['{broken', '{"result":[]}', '[]']):
            (folder / ('save-pending-' + str(index) * 32 + '.json')).write_text(payload)
        result = main(dict(self.request, action='reload', session_id=started['session_id']), self.config)
        self.assertEqual(len(result['pending_saves']), 3)
        self.assertTrue(all(not row['recoverable'] and row['reason'] for row in result['pending_saves']))

    def test_model_environment_change_invalidates_the_recorded_runtime(self):
        from unittest.mock import patch
        with patch.dict(os.environ, {'TESSDATA_PREFIX': str(self.root / 'model-root-one')}):
            started = self.start()
        with patch.dict(os.environ, {'TESSDATA_PREFIX': str(self.root / 'model-root-two')}):
            resumed = main(dict(self.request, action='resume'), self.config)
        self.assertFalse(resumed['runtime_compatible'])

    def test_private_root_and_ambiguous_session_fail_closed(self):
        started = self.start()
        root = Path(self.config['session_root'])
        self.assertEqual(root.stat().st_mode & 0o077, 0)
        self.assertEqual((root / started['session_id']).stat().st_mode & 0o077, 0)
        link = self.root / 'link'
        link.symlink_to(root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'SYMLINK'):
            roots(dict(self.config, session_root=str(link)))
        second = self.root / 'another-private'
        second.mkdir(mode=0o700)
        import shutil
        shutil.copytree(root / started['session_id'], second / started['session_id'])
        with self.assertRaisesRegex(ValueError, 'AMBIGUOUS'):
            main(dict(self.request, action='reload', session_id=started['session_id']),
                 dict(self.config, legacy_session_roots=[str(second)]))

    def test_legacy_runtime_missing_stays_readonly_and_old_journal_visible(self):
        started = self.start()
        folder = Path(self.config['session_root']) / started['session_id']
        meta_path = folder / 'desktop-session.json'
        meta = json.loads(meta_path.read_text())
        meta.pop('runtime_binding')
        meta.pop('session_storage')
        meta_path.write_text(json.dumps(meta))
        (folder / ('save-pending-' + 'e' * 32 + '.json')).write_text(json.dumps(
            dict(state='PREPARED', result=dict(output_pdf=str(self.dest)))))
        resumed = main(dict(self.request, action='resume'), self.config)
        self.assertFalse(resumed['runtime_compatible'])
        self.assertEqual(resumed['session_storage'], 'legacy-temporary')
        self.assertFalse(resumed['pending_saves'][0]['recoverable'])
        with self.assertRaisesRegex(ValueError, 'RUNTIME_UNVERIFIED_OR_CHANGED'):
            main(dict(self.request, action='save', session_id=started['session_id']), self.config)

    def test_user_edit_after_successful_save_cannot_be_searched_as_verified_output(self):
        started = self.start()
        saved = main(dict(self.request, action='save', session_id=started['session_id'],
                          expected_revision=started['revision'], output_path=str(self.dest)), self.config)
        with fitz.open() as doc:
            doc.new_page().insert_text((30, 40), 'different user content')
            alternate = self.root / 'different.pdf'
            doc.save(alternate)
        alternate.replace(self.dest)
        with self.assertRaisesRegex(ValueError, 'SAVED_OUTPUT_CHANGED_OR_UNBOUND'):
            main(dict(self.request, action='search', session_id=started['session_id'],
                      saved_pdf=saved['saved']['output_pdf'], query='different'), self.config)


class CompletionGapTests(unittest.TestCase):
    setUp=RecoveryTests.setUp
    tearDown=RecoveryTests.tearDown
    start=RecoveryTests.start
    def test_actual_publish_before_receipt_exit_is_visible_and_read_only(self):
        from unittest.mock import patch
        from scripts.ocr.mvp import local
        payload=self.root/'completion-crash-input.json'
        payload.write_text(json.dumps(dict(request=self.request,config=self.config)))
        script='''import json,os,sys
from pathlib import Path
from scripts.ocr.app_mvp_bridge.__main__ import main
from scripts.ocr.mvp import local
p=json.load(open(sys.argv[1]));original=local.write
def exit_at_completion(path,value):
 if Path(path).name=='completion.json':
  assert (Path(path).parent/'CURRENT.json').is_file()
  assert all(page['status']=='NATIVE_PRESERVED' for page in value['page_results'])
  os._exit(88)
 return original(path,value)
local.write=exit_at_completion
main(dict(p['request'],action='start',pages=[1,2],client_operation_id='8'*32),p['config'])
raise AssertionError('crash not reached')
'''
        proc=subprocess.run([sys.executable,'-B','-c',script,str(payload)],capture_output=True,text=True,timeout=20)
        self.assertEqual(proc.returncode,88,proc.stderr+proc.stdout)
        folder=next(Path(self.config['session_root']).glob('*/desktop-session.json')).parent;op=folder/'operation'
        before={str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()}
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Resume started a worker')):
            resumed=main(dict(self.request,action='resume'),self.config)
            reloaded=main(dict(self.request,action='reload',session_id=folder.name),self.config)
        self.assertEqual(resumed['status'],'completion_unverified');self.assertEqual(resumed['completion_record_state'],'missing')
        self.assertEqual(reloaded['status'],'completion_unverified');self.assertTrue(resumed['runtime_compatible'])
        self.assertFalse((op/'completion.json').exists());self.assertFalse(list((op/'raw').glob('**/*.call.json')))
        self.assertEqual(before,{str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()})

    def test_unreadable_completion_survives_resume_without_becoming_success(self):
        from unittest.mock import patch
        from scripts.ocr.mvp import local
        started=self.start();folder=Path(self.config['session_root'])/started['session_id'];op=folder/'operation'
        completion=op/'completion.json';original=b'{"partial":';completion.write_bytes(original)
        before={str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()}
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Resume started a worker')):
            resumed=main(dict(self.request,action='resume'),self.config)
        self.assertEqual(resumed['status'],'completion_unverified');self.assertEqual(resumed['completion_record_state'],'unreadable')
        self.assertEqual(original,completion.read_bytes());self.assertEqual(before,{str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()})

    def test_unreadable_completion_review_advances_bound_draft_and_keeps_failure_bytes(self):
        from unittest.mock import patch
        from scripts.ocr.mvp import local
        from scripts.ocr.mvp.store import review_save
        started=self.start();folder=Path(self.config['session_root'])/started['session_id'];op=folder/'operation'
        completion=op/'completion.json';original=b'{"partial":';completion.write_bytes(original)
        raw={str(p.relative_to(op)):sha(p) for p in (op/'raw').rglob('*') if p.is_file()}
        current,_=local.load_snapshot(op)
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Review started a worker')):
            receipt=review_save(op,dict(expected_revision=current['revision'],input_sha256=self.request['input_sha256'],actions=[]))
            latest=main(dict(self.request,action='reload',session_id=folder.name),self.config)
        self.assertEqual(receipt['revision'],current['revision']+1)
        self.assertFalse(receipt['receipt']['human_approval_claimed'])
        self.assertEqual(latest['revision'],receipt['revision']);self.assertEqual(latest['status'],'completion_unverified')
        self.assertEqual(completion.read_bytes(),original)
        self.assertEqual(raw,{str(p.relative_to(op)):sha(p) for p in (op/'raw').rglob('*') if p.is_file()})

    def test_foreign_completion_refuses_review_before_new_revision(self):
        from scripts.ocr.mvp import local
        from scripts.ocr.mvp.store import review_save
        started=self.start();folder=Path(self.config['session_root'])/started['session_id'];op=folder/'operation'
        completion=op/'completion.json';data=json.loads(completion.read_text());data['input_sha256']='0'*64;completion.write_text(json.dumps(data))
        before={str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()}
        with self.assertRaisesRegex(ValueError,'COMPLETION_BINDING_MISMATCH'):
            review_save(op,dict(expected_revision=started['revision'],input_sha256=self.request['input_sha256'],actions=[]))
        self.assertEqual(local.load_snapshot(op)[0]['revision'],started['revision'])
        self.assertEqual(before,{str(p.relative_to(op)):sha(p) for p in op.rglob('*') if p.is_file()})


class PrepublicationTests(unittest.TestCase):
    setUp=RecoveryTests.setUp
    tearDown=RecoveryTests.tearDown

    def incomplete(self):
        """Synthetic raw page for refusal probes; not a recognition result."""
        from .__main__ import register_local_operation, write
        from .sessions import runtime_binding
        from scripts.ocr.mvp.core import CONFIG_VERSION
        root=roots(self.config,create=True)[0];folder=root/('d'*32);folder.mkdir(mode=0o700)
        op=folder/'operation'
        task=dict(operation_id=folder.name,input_pdf=str(self.source),input_sha256=sha(self.source),
                  page_numbers=[1,2],mode='local',output_directory=str(op),config_version=CONFIG_VERSION)
        register_local_operation(folder,task,self.config['local'])
        meta=dict(mode='local',source=str(self.source),source_sha256=sha(self.source),provenance='synthetic-contract-fixture',
                  runtime_binding=runtime_binding(self.config),session_storage='persistent-private')
        write(folder/'desktop-session.json',meta)
        raw=op/'raw/page-0001';raw.mkdir(parents=True)
        with fitz.open(self.source) as doc:doc[0].get_pixmap().save(raw/'source.png')
        page=dict(page=1,status='OCR_DRAFT',source_sha256=sha(self.source),words=[dict(id='synthetic',text='alpha',bbox=[20,20,50,40])],
                  image_path=str(raw/'source.png'),image_sha256=sha(raw/'source.png'),review_reasons=[],raw_files={'raw/page-0001/source.png':sha(raw/'source.png')})
        write(raw/'page-result.json',page)
        return folder,page

    def inspect(self,folder):
        from unittest.mock import patch
        from scripts.ocr.mvp import local
        before={str(p.relative_to(folder)):sha(p) for p in folder.rglob('*') if p.is_file() and not p.is_symlink()}
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Read-only inspection started a worker')):
            result=main(dict(self.request,action='reload',session_id=folder.name),self.config)
        self.assertEqual(before,{str(p.relative_to(folder)):sha(p) for p in folder.rglob('*') if p.is_file() and not p.is_symlink()})
        self.assertFalse(result['draft_published']);self.assertIsNone(result['revision']);self.assertNotIn('output_pdf',result)
        self.assertEqual(result['status'],'processing_unverified')
        self.assertFalse((folder/'operation/CURRENT.json').exists());self.assertFalse((folder/'operation/completion.json').exists())
        return result

    def test_actual_first_native_worker_reaped_then_parent_exit_retains_partial_task(self):
        payload=self.root/'prepublication-crash.json';payload.write_text(json.dumps(dict(request=self.request,config=self.config)))
        script='''import json,os,sys
from pathlib import Path
from scripts.ocr.app_mvp_bridge.__main__ import main
from scripts.ocr.mvp import local
p=json.load(open(sys.argv[1]));children=[];launch=local.subprocess.Popen;write=local.write
def observed(*a,**k):
 proc=launch(*a,**k);children.append(proc);return proc
def abrupt(path,value):
 write(path,value)
 if Path(path).name=='progress.json' and value['completed']==1:
  assert len(children)==1 and children[0].returncode==0
  assert children[0].stdout.closed and children[0].stderr.closed
  assert value['page_results'][0]['status']=='NATIVE_PRESERVED'
  assert not (Path(path).parent/'CURRENT.json').exists()
  os._exit(88)
local.subprocess.Popen=observed;local.write=abrupt
main(dict(p['request'],action='start',pages=[1,2],client_operation_id='9'*32),p['config'])
raise AssertionError('controlled exit not reached')
'''
        proc=subprocess.run([sys.executable,'-B','-c',script,str(payload)],capture_output=True,text=True,timeout=20)
        self.assertEqual(proc.returncode,88,proc.stderr+proc.stdout)
        folder=next(Path(self.config['session_root']).glob('*/desktop-session.json')).parent
        from unittest.mock import patch
        from scripts.ocr.mvp import local
        with patch.object(local.subprocess,'Popen',side_effect=AssertionError('Resume started a worker')):
            resumed=main(dict(self.request,action='resume'),self.config)
        self.assertEqual(self.inspect(folder),resumed);self.assertTrue(resumed['runtime_compatible'])
        self.assertEqual([p['status'] for p in resumed['pages']],['NATIVE_PRESERVED','NOT_PROCESSED'])
        self.assertIn('314159',resumed['pages'][0]['native_text'])
        self.assertFalse(list((folder/'operation/raw').glob('**/*.call.json')))
        self.assertEqual(sha(self.source),self.request['input_sha256'])

    def test_registered_task_without_results_is_inspectable_not_declared_dead(self):
        folder,_=self.incomplete();raw=folder/'operation/raw/page-0001'
        (raw/'page-result.json').unlink()
        observed=self.inspect(folder)
        self.assertEqual([p['status'] for p in observed['pages']],['UNVERIFIED','NOT_PROCESSED'])
        self.assertEqual(observed['completion_record_state'],'not_published')

    def test_unpublished_draft_rejects_mutation_even_with_matching_runtime(self):
        folder,_=self.incomplete();self.assertTrue(self.inspect(folder)['runtime_compatible'])
        for action in ('review','save','recover-save'):
            with self.subTest(action=action),self.assertRaisesRegex(ValueError,'DRAFT_NOT_PUBLISHED'):
                main(dict(self.request,action=action,session_id=folder.name,expected_revision=None,actions=[],output_path=str(self.dest)),self.config)
        self.assertFalse(self.dest.exists())

    def test_wrong_source_page_raw_hash_and_word_shape_are_unverified(self):
        folder,page=self.incomplete();path=folder/'operation/raw/page-0001/page-result.json'
        cases=[dict(source_sha256='0'*64),dict(page=2),dict(raw_files={'raw/page-0001/source.png':'0'*64}),dict(image_sha256=None,raw_files={},words=[]),dict(words=[dict(id='bad',text='x',bbox=[0,0,float('nan'),1])]),dict(review_reasons='bad')]
        for change in cases:
            with self.subTest(change=str(change)):
                path.write_text(json.dumps(dict(page,**change)))
                observed=self.inspect(folder)['pages'][0]
                self.assertEqual(observed['status'],'UNVERIFIED');self.assertEqual(observed['words'],[])

    def test_raw_paths_and_symlinks_are_rejected_without_reading_targets(self):
        folder,page=self.incomplete();path=folder/'operation/raw/page-0001/page-result.json';outside=self.root/'outside';outside.write_bytes(b'preserve')
        cases=[{'../outside':sha(outside)},{str(outside):sha(outside)},{'raw/page-0002/foreign':sha(outside)}]
        for raw in cases:
            with self.subTest(raw=raw):
                path.write_text(json.dumps(dict(page,raw_files=raw)))
                self.assertEqual(self.inspect(folder)['pages'][0]['status'],'UNVERIFIED')
        link=path.parent/'link';link.symlink_to(outside)
        path.write_text(json.dumps(dict(page,raw_files={'raw/page-0001/link':sha(outside)})))
        self.assertEqual(self.inspect(folder)['pages'][0]['status'],'UNVERIFIED')
        path.unlink();path.symlink_to(outside)
        self.assertEqual(self.inspect(folder)['pages'][0]['status'],'UNVERIFIED')
        self.assertEqual(outside.read_bytes(),b'preserve')
        current=folder/'operation/CURRENT.json';current.symlink_to(outside)
        with self.assertRaisesRegex(ValueError,'LOCAL_DRAFT_PATH_SYMLINK'):
            main(dict(self.request,action='reload',session_id=folder.name),self.config)

    def test_parent_failure_precedes_worker_result_and_preserves_both(self):
        from scripts.ocr.mvp.local import parent_page_result
        folder,_=self.incomplete();op=folder/'operation';worker=op/'raw/page-0001/page-result.json'
        original=worker.read_bytes()
        parent_page_result(op,worker,dict(page=1,status='FAILED',source_sha256=sha(self.source),words=[],error='synthetic cleanup failure',review_reasons=[]))
        self.assertEqual(self.inspect(folder)['pages'][0]['status'],'FAILED');self.assertEqual(worker.read_bytes(),original)
        (worker.parent/'parent-result.json').write_bytes(b'{"partial":')
        self.assertEqual(self.inspect(folder)['pages'][0]['status'],'UNVERIFIED')

    def test_foreign_job_or_document_binding_refuses_entire_partial_view(self):
        from scripts.ocr.mvp.core import digest
        folder,_=self.incomplete();path=folder/'operation/job.json';original=json.loads(path.read_text())
        for field,value in [('operation_id','e'*32),('input_sha256','0'*64),('output_directory',str(self.root)),('page_numbers',[1,3])]:
            with self.subTest(field=field):
                job=copy.deepcopy(original);job['task'][field]=value;job['identity']=digest(dict(task=job['task'],config=job['config']));path.write_text(json.dumps(job))
                with self.assertRaises(ValueError):self.inspect(folder)
        path.write_text(json.dumps(original));meta_path=folder/'desktop-session.json';meta=json.loads(meta_path.read_text());meta['document_binding']=dict(source_sha256=sha(self.source),source_pdf=str(self.source),source_page_count=2,physical_pages=[2,1]);meta_path.write_text(json.dumps(meta))
        with self.assertRaisesRegex(ValueError,'INCOMPLETE_DOCUMENT_BINDING'):self.inspect(folder)

    def test_changed_or_missing_runtime_is_not_rebound_and_corrupt_current_never_falls_back(self):
        folder,_=self.incomplete();meta_path=folder/'desktop-session.json';meta=json.loads(meta_path.read_text());meta.pop('runtime_binding');meta_path.write_text(json.dumps(meta))
        self.assertFalse(self.inspect(folder)['runtime_compatible'])
        current=folder/'operation/CURRENT.json';current.write_bytes(b'{"partial":')
        with self.assertRaises(ValueError):main(dict(self.request,action='reload',session_id=folder.name),self.config)
        self.assertEqual(current.read_bytes(),b'{"partial":')


if __name__ == '__main__':
    unittest.main()
