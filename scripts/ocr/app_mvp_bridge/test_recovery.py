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


if __name__ == '__main__':
    unittest.main()
