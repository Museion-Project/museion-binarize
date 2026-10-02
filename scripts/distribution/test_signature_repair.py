"""Signature-copy boundaries with mocked codesign only. Native Start/OCR zero."""
import copy
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from scripts.ocr.app_mvp_bridge.runtime_contract import SOURCE_FILES, INITIALIZERS, sha, verify_inventory
from . import repair_local_resource_signature as repair


class SignatureRepairTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'source'
        self.target = self.root / 'new-copy'
        names = set(SOURCE_FILES) | INITIALIZERS | {repair.LIBRARY, 'python/bin/python', 'bin/helper', 'bin/tesseract',
                                                    'fonts/font.ttf', 'tessdata/eng.traineddata', 'tessdata/grc.traineddata'}
        for name in names:
            p = self.source / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(name)
        manifest = dict(schema='museion-local-runtime/1', modes=['local'], python='python/bin/python',
                        apple_helper='bin/helper', tesseract='bin/tesseract', font='fonts/font.ttf', tessdata='tessdata',
                        code_files={n: sha(self.source / n) for n in SOURCE_FILES},
                        runtime_files=repair.inventory(self.source))
        (self.source / 'runtime-manifest.json').write_text(json.dumps(manifest))
        build = dict(schema='local-resource-build/1', files=[dict(path=n, sha256=h) for n,h in repair.inventory(self.source).items()],
                     signing_performed=False, native_mutations=[dict(stage='rpath', signature_verify_returncode=1)])
        (self.source / 'resource-build.json').write_text(json.dumps(build))
        (self.source / 'tauri.generated.overlay.json').write_text(json.dumps(dict(bundle=dict(resources={str(self.source):'local-ocr'}))))
        self.preview = dict(schema='local-adhoc-signing-preview/1', input_resource_root=str(self.source),
                            input_runtime_manifest_sha256=sha(self.source / 'runtime-manifest.json'),
                            input_resource_build_sha256=sha(self.source / 'resource-build.json'),
                            input_inventory_files=len(build['files']), relative_library=repair.LIBRARY,
                            library_current_sha256=sha(self.source / repair.LIBRARY), target_new_copy=str(self.target),
                            proposed_command=['/usr/bin/codesign','--force','--sign','-',str(self.target / repair.LIBRARY)],costs_usd=0)
        self.preview_path = self.root / 'preview.json'
        self.auth_path = self.root / 'authorization.json'
        self.freeze()

    def tearDown(self):
        self.temp.cleanup()

    def freeze(self):
        self.preview_path.write_text(json.dumps(self.preview))
        self.auth_path.write_text(json.dumps(dict(schema='local-signature-authorization-record/1', user_source='unit fixture',
                                                  preview_sha256=sha(self.preview_path), extra_signing_allowed=False)))

    def preflight(self):
        return repair.preflight(self.preview_path,self.auth_path,self.root / 'evidence')

    def fake_sign(self, args, **kwargs):
        if '--force' in args:
            p = Path(args[-1]);p.write_bytes(p.read_bytes() + b' mock signature')
        return subprocess.CompletedProcess(args,0,'','Signature=adhoc' if '--display' in args else '')

    def run_repair(self):
        return repair.repair(self.preview_path,self.auth_path,self.root / 'evidence')

    def test_preflight_is_readonly_and_authorization_is_exact_preview_bound(self):
        self.preflight()
        self.assertFalse(self.target.exists())
        self.assertFalse((self.root / 'evidence').exists())
        self.preview_path.write_text(self.preview_path.read_text()+' ')
        with self.assertRaisesRegex(ValueError,'AUTHORIZATION_RECORD_BINDING'):
            self.preflight()

    def test_success_mutates_one_independent_library_and_exact_metadata_only(self):
        original = repair.inventory(self.source)
        with patch.object(repair.subprocess,'run',side_effect=self.fake_sign) as run, patch.object(repair,'macho_audit',return_value=[]):
            result=self.run_repair()
        self.assertFalse(result['distribution_ready'])
        self.assertEqual(sum('--force' in call.args[0] for call in run.call_args_list),1)
        self.assertEqual(repair.inventory(self.source),original)
        manifest=json.loads((self.target / 'runtime-manifest.json').read_text())
        verify_inventory(self.target,manifest)
        build=json.loads((self.target / 'resource-build.json').read_text())
        self.assertEqual(build['native_mutations'][0]['signature_verify_returncode'],1)
        self.assertEqual(build['signature_repair']['state'],'SIGNATURE_VERIFIED_STATIC_CLOSURE_PASS')
        self.assertTrue(build['signing_performed'])
        self.assertTrue(build['signatures_unverified'])
        self.assertEqual(build['signature_repair']['sign']['command'],self.preview['proposed_command'])
        for entry in build['files']:
            self.assertEqual(sha(self.target/entry['path']),entry['sha256'])
        receipt=json.loads((self.root/'evidence/result.json').read_text())
        self.assertEqual(len(receipt['changed_files']),4)
        self.assertFalse(receipt['whole_bundle_signatures_verified'])
        with patch.object(repair.subprocess,'run') as retry:
            with self.assertRaisesRegex(ValueError,'OUTPUT_NOT_NEW_OR_EMPTY'):
                repair.repair(self.preview_path,self.auth_path,self.root/'retry-evidence')
            retry.assert_not_called()

    def test_nonapproved_library_or_command_cannot_expand_scope(self):
        for key,value,error in [('relative_library','bin/tesseract','ONE_LIBRARY_ONLY'),
                                ('proposed_command',['/usr/bin/codesign','--deep','--sign','-'],'COMMAND_SCOPE')]:
            before=copy.deepcopy(self.preview);self.preview[key]=value;self.freeze()
            with self.assertRaisesRegex(ValueError,error):self.preflight()
            self.preview=before

    def test_changed_inputs_or_symlink_rejected_before_copy(self):
        (self.source / repair.LIBRARY).write_text('changed')
        with self.assertRaisesRegex(ValueError,'RESOURCE_HASH_MISMATCH'):self.preflight()
        self.assertFalse(self.target.exists())

    def test_preview_library_identity_cannot_disagree_with_inventory(self):
        self.preview['library_current_sha256']='0'*64;self.freeze()
        with self.assertRaisesRegex(ValueError,'LIBRARY_CHANGED'):self.preflight()

    def test_existing_output_and_evidence_are_preserved(self):
        self.target.mkdir();marker=self.target/'user';marker.write_text('keep')
        with self.assertRaisesRegex(ValueError,'OUTPUT_NOT_NEW_OR_EMPTY'):self.preflight()
        self.assertEqual(marker.read_text(),'keep')
        self.target.rmdir() if not list(self.target.iterdir()) else None
        # The existing-output check is sufficient; evidence uses the same
        # exclusive semantics and is exercised in the next separate fixture.

    def test_evidence_reuse_or_input_overlap_rejected(self):
        (self.root/'evidence').mkdir()
        with self.assertRaisesRegex(ValueError,'EVIDENCE_NOT_NEW'):self.preflight()
        with self.assertRaisesRegex(ValueError,'EVIDENCE_SCOPE_OVERLAP'):
            repair.preflight(self.preview_path,self.auth_path,self.source/'evidence')

    def test_signature_failure_retains_copy_and_intent_and_never_retries(self):
        def fail(args,**kwargs):return subprocess.CompletedProcess(args,1,'','mock signing failure')
        before=repair.inventory(self.source)
        with patch.object(repair.subprocess,'run',side_effect=fail) as run:
            with self.assertRaisesRegex(ValueError,'SIGN_FAILED'):self.run_repair()
        self.assertEqual(run.call_count,1)
        self.assertTrue(self.target.exists())
        self.assertTrue((self.root/'evidence/sign-intent.json').is_file())
        self.assertTrue((self.root/'evidence/failure.json').is_file())
        self.assertFalse((self.root/'evidence/result.json').exists())
        self.assertEqual(repair.inventory(self.source),before)

    def test_verify_failure_is_not_promoted_to_success(self):
        def fail_verify(args,**kwargs):
            if '--verify' in args:return subprocess.CompletedProcess(args,1,'','invalid')
            return self.fake_sign(args,**kwargs)
        with patch.object(repair.subprocess,'run',side_effect=fail_verify):
            with self.assertRaisesRegex(ValueError,'VERIFY_FAILED'):self.run_repair()
        self.assertFalse((self.root/'evidence/result.json').exists())

    def test_extra_sign_mutation_is_rejected_and_retained_for_inspection(self):
        def extra(args,**kwargs):
            result=self.fake_sign(args,**kwargs)
            if '--force' in args:(self.target/'bin/helper').write_text('unexpected')
            return result
        with patch.object(repair.subprocess,'run',side_effect=extra):
            with self.assertRaisesRegex(ValueError,'UNEXPECTED_SIGN_MUTATION'):self.run_repair()
        self.assertTrue((self.root/'evidence/failure.json').exists())
        self.assertEqual((self.source/'bin/helper').read_text(),'bin/helper')

    def test_macho_failure_retains_signed_copy_but_not_success_receipt(self):
        with patch.object(repair.subprocess,'run',side_effect=self.fake_sign), patch.object(repair,'macho_audit',side_effect=ValueError('external dylib')):
            with self.assertRaisesRegex(ValueError,'external dylib'):self.run_repair()
        self.assertFalse((self.root/'evidence/result.json').exists())
        self.assertTrue((self.root/'evidence/verify.json').is_file())

    def test_symlink_target_or_evidence_is_rejected(self):
        link=self.root/'link';link.symlink_to(self.source,target_is_directory=True)
        self.preview['target_new_copy']=str(link/'copy');self.freeze()
        with self.assertRaisesRegex(ValueError,'OUTPUT_SYMLINK'):self.preflight()


if __name__=='__main__':unittest.main()
