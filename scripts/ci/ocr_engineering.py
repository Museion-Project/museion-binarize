"""Source/contract CI only. No research panel, recognizer, cloud call or release.

Run from the repository root. The snapshot binds the selected current working
tree files, rather than treating a previous main-branch CI as candidate proof.
"""
import argparse
import hashlib
import json
import platform
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEST_MODULES = (
    'scripts.ocr.mvp.test_mvp',
    'scripts.ocr.mvp.test_reader_alternatives',
    'scripts.ocr.mvp.test_position_export_candidate',
    'scripts.ocr.mvp.test_pdf_consumer_audit',
    'scripts.ocr.mvp.test_physical_row_audit',
    'scripts.ocr.mvp.test_development_panel',
    'scripts.ocr.mvp.test_development_reuse',
    'scripts.ocr.mvp.test_worker_identity',
    'scripts.ocr.app_mvp_bridge.test_bridge',
    'scripts.ocr.app_mvp_bridge.test_runtime_contract',
    'scripts.ocr.app_mvp_bridge.test_document_binding',
    'scripts.ocr.app_mvp_bridge.test_atomic_save',
    'scripts.ocr.app_mvp_bridge.test_recovery',
    'scripts.distribution.test_local_candidate',
    'scripts.distribution.test_local_app_resources',
    'scripts.distribution.test_launcher_contract',
    'scripts.distribution.test_signature_repair',
    'scripts.distribution.test_resource_dossier',
    'scripts.ocr.paid_mvp.test_p9_contract',
    'scripts.ocr.paid_mvp.test_selective',
    'scripts.bookmarks.paid_mvp.test_source_identity',
    'scripts.bookmarks.paid_mvp.test_complete_source',
    'scripts.bookmarks.paid_mvp.test_complete_admission',
    'scripts.bookmarks.development_corpus.test_source_review_package',
)


def verify_snapshot():
    path = ROOT / 'distribution/ocr-engineering-source-snapshot.json'
    snapshot = json.loads(path.read_text())
    if snapshot['schema'] != 'ocr-engineering-source-snapshot/1':
        raise ValueError('SNAPSHOT_SCHEMA')
    for name, expected in snapshot['files'].items():
        rel = Path(name)
        if rel.is_absolute() or '..' in rel.parts:
            raise ValueError('SNAPSHOT_PATH_ESCAPE')
        file = ROOT / rel
        if file.is_symlink() or not file.is_file():
            raise ValueError('SNAPSHOT_FILE_MISSING: ' + name)
        if hashlib.sha256(file.read_bytes()).hexdigest() != expected:
            raise ValueError('SNAPSHOT_SOURCE_CHANGED: ' + name)
    from scripts.ocr.app_mvp_bridge.runtime_contract import SOURCE_FILES
    if not set(SOURCE_FILES).issubset(snapshot['files']):
        raise ValueError('LOCAL_RUNTIME_SOURCE_CLOSURE_MISSING')
    return dict(schema='ocr-engineering-ci-receipt/1', source_files=len(snapshot['files']),
                snapshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                python=sys.version, platform=platform.platform(),
                evidence_scope='current source + synthetic component/contracts; no product quality or GUI acceptance',
                recognizer_calls=0, provider_calls=0, signing_performed=False,
                independent_mac_release_gate='PENDING', quality_ready=False,
                app_admission=False, distribution_ready=False, release_ready=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify-only', action='store_true')
    args = parser.parse_args()
    receipt = verify_snapshot()
    if not args.verify_only:
        suite = unittest.defaultTestLoader.loadTestsFromNames(TEST_MODULES)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        receipt.update(tests_run=result.testsRun, skipped=len(result.skipped),
                       failures=len(result.failures), errors=len(result.errors),
                       success=result.wasSuccessful())
        if not result.wasSuccessful():
            print(json.dumps(receipt))
            return 1
    print(json.dumps(receipt))
    return 0


if __name__ == '__main__':
    sys.path.insert(0, str(ROOT))
    raise SystemExit(main())
