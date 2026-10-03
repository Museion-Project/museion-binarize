"""Inventory/notice evidence never implies release; inspect has no subprocess."""
import json
import copy
from pathlib import Path
import unittest
from unittest.mock import patch
from . import test_local_candidate as fixtures
from . import package_local_candidate as pack
from .local_resource_dossier import inspect,font_names


class ResourceDossierTests(unittest.TestCase):
    def setUp(self):
        self.fixture=fixtures.LocalCandidateTests();self.fixture.setUp();self.root=self.fixture.root
        pack.package(self.fixture.repo,self.fixture.freeze_path,self.fixture.runtime_path,
                     self.root/'stage',self.root/'resource')
    def tearDown(self):self.fixture.tearDown()

    def test_static_dossier_preserves_candidate_and_has_no_runtime_or_release_claim(self):
        manifest=self.root/'resource/runtime-manifest.json';before=manifest.read_bytes()
        with patch('subprocess.run',side_effect=AssertionError('no binaries allowed')):
            result=inspect(self.root/'resource')
        self.assertEqual(manifest.read_bytes(),before);self.assertEqual(result['executed_binaries'],0)
        self.assertFalse(result['legal_admission']);self.assertFalse(result['distribution_ready'])
        self.assertFalse(result['release_ready']);self.assertTrue(result['unresolved'])

    def test_changed_resource_rejected_without_repair_or_overwrite(self):
        font=self.root/'resource/fonts/test.ttf';font.write_bytes(b'changed')
        with self.assertRaises(ValueError):inspect(self.root/'resource')
        self.assertEqual(font.read_bytes(),b'changed')

    def test_literal_package_license_does_not_grant_combined_work_license(self):
        resource=self.root/'resource';path=resource/'python/lib/example-1.dist-info/METADATA'
        path.parent.mkdir(parents=True);path.write_text('Name: example\nVersion: 1\nLicense-Expression: AGPL-3.0-or-later\n')
        manifest=resource/'runtime-manifest.json';data=json.loads(manifest.read_text())
        data['runtime_files'][str(path.relative_to(resource))]=pack.sha(path);manifest.write_text(json.dumps(data))
        result=inspect(resource);self.assertEqual(result['metadata'][0]['license_expression'],'AGPL-3.0-or-later')
        self.assertEqual(result['metadata'][0]['original_installation_provenance'],'UNKNOWN')
        self.assertFalse(result['legal_admission'])

    def test_unsupported_font_is_unknown_not_fabricated_origin(self):
        self.assertEqual(font_names(self.root/'resource/fonts/test.ttf')['state'],'UNKNOWN')

    def test_reference_byte_match_does_not_fabricate_original_installation_event(self):
        resource=self.root/'resource';font=resource/'fonts/test.ttf';reference=self.root/'reference.ttf'
        reference.write_bytes(font.read_bytes());record=self.root/'reference.json'
        data=dict(schema='font-upstream-content-provenance/1',state='CONTENT_MATCH',component_path='fonts/test.ttf',
                  packaged_sha256=pack.sha(font),upstream_sha256=pack.sha(reference),upstream_file=str(reference),
                  commit='synthetic-reference',url='https://example.invalid/synthetic-font')
        record.write_text(json.dumps(data));result=inspect(resource,[record])
        self.assertEqual(result['fonts'][0]['upstream_content']['record']['state'],'CONTENT_MATCH')
        self.assertEqual(result['fonts'][0]['original_installation_provenance'],'UNKNOWN')
        self.assertFalse(result['legal_admission'])
        reference.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'CONTENT_PROVENANCE_IDENTITY'):inspect(resource,[record])

    def native_record(self):
        resource=self.root/'resource'
        manifest=resource/'runtime-manifest.json';inventory=json.loads(manifest.read_text())
        objects=[]
        for name in ['native/library.dylib','python/lib/module.so']:
            path=resource/name;path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(bytes.fromhex('cffaedfe')+name.encode())
            digest=pack.sha(path);inventory['runtime_files'][name]=digest
            objects.append(dict(path='Contents/Resources/local-ocr/'+name,sha256=digest,
                source_reference_kind='EXACT_PUBLIC_ARTIFACT_MEMBER_TO_RECORDED_INPUT_AND_CURRENT_APP',
                upstream_content_correspondence_verified=True,
                current_local_transform_graph=dict(original_input_sha256='1'*64,current_App_sha256=digest,
                    content_hash_path=['1'*64,digest])))
        manifest.write_text(json.dumps(inventory))
        # Main/PDFium are deliberately outside this runtime-only inspection.
        objects.append(dict(path='Contents/MacOS/main',sha256='2'*64))
        record=self.root/'native-record.json'
        data=dict(schema='current-App-public-artifact-content-reference-SBOM/5',objects=objects)
        record.write_text(json.dumps(data))
        return resource,record,data

    def test_native_record_binds_scope_and_keeps_history_and_admission_unknown(self):
        resource,record,data=self.native_record()
        with patch('subprocess.run',side_effect=AssertionError('no binaries allowed')):
            result=inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))
        binding=result['native_content_record_binding']
        self.assertEqual(binding['scope_native_objects'],2)
        self.assertEqual(binding['recorded_public_content_correspondences'],2)
        self.assertEqual(binding['outside_scope_record_paths'],['Contents/MacOS/main'])
        self.assertFalse(binding['upstream_prediction_replayed'])
        self.assertFalse(binding['original_build_install_events_verified'])
        self.assertTrue(all(x['original_build_provenance']=='UNKNOWN' for x in result['native_objects']))
        self.assertFalse(result['legal_admission']);self.assertFalse(result['release_ready'])

    def test_native_record_requires_explicit_matching_digest(self):
        resource,record,_=self.native_record()
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORD_AND_HASH_REQUIRED'):
            inspect(resource,native_content_record=record)
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORD_HASH'):
            inspect(resource,native_content_record=record,native_content_record_sha256='0'*64)

    def test_native_record_changed_or_foreign_package_digest_rejected(self):
        resource,record,data=self.native_record();before=pack.sha(record)
        data['objects'][0]['sha256']='3'*64;record.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORD_HASH'):
            inspect(resource,native_content_record=record,native_content_record_sha256=before)
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_PACKAGE_IDENTITY'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))

    def test_native_record_missing_or_extra_scoped_object_rejected(self):
        resource,record,data=self.native_record();bad=copy.deepcopy(data);bad['objects'].pop(0)
        record.write_text(json.dumps(bad))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_SCOPE_COVERAGE'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))
        bad=copy.deepcopy(data);bad['objects'][0]['path']='Contents/Resources/local-ocr/native/extra.dylib'
        record.write_text(json.dumps(bad))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_PACKAGE_IDENTITY'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))

    def test_native_record_duplicate_or_path_escape_rejected(self):
        resource,record,data=self.native_record();bad=copy.deepcopy(data);bad['objects'].append(bad['objects'][0])
        record.write_text(json.dumps(bad))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORD_DUPLICATE'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))
        bad=copy.deepcopy(data);bad['objects'][0]['path']='Contents/Resources/local-ocr/../escape'
        record.write_text(json.dumps(bad))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORD_PATH'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))

    def test_native_record_broken_recorded_chain_rejected(self):
        resource,record,data=self.native_record()
        data['objects'][0]['current_local_transform_graph']['content_hash_path'][-1]='4'*64
        record.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORDED_CHAIN'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))

    def test_native_record_unchanged_input_has_one_hash_node_but_empty_chain_rejected(self):
        resource,record,data=self.native_record()
        obj=data['objects'][0];graph=obj['current_local_transform_graph']
        graph['original_input_sha256']=obj['sha256'];graph['content_hash_path']=[obj['sha256']]
        record.write_text(json.dumps(data))
        result=inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))
        self.assertEqual(result['native_content_record_binding']['recorded_public_content_correspondences'],2)
        graph['content_hash_path']=[];record.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError,'NATIVE_CONTENT_RECORDED_CHAIN'):
            inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))

    def test_native_record_claimed_readiness_is_not_imported(self):
        resource,record,data=self.native_record()
        data.update(quality_ready=True,app_admission=True,distribution_ready=True,release_ready=True,legal_admission=True)
        record.write_text(json.dumps(data))
        result=inspect(resource,native_content_record=record,native_content_record_sha256=pack.sha(record))
        for name in ['quality_ready','app_admission','distribution_ready','release_ready','legal_admission']:
            self.assertFalse(result[name]);self.assertFalse(result['native_content_record_binding'][name])

    def test_native_record_without_evidence_stays_explicitly_unknown(self):
        resource,_,_=self.native_record();result=inspect(resource)
        self.assertIsNone(result['native_content_record_binding'])
        self.assertIn('native upstream content record not supplied',result['unresolved'])


if __name__=='__main__':unittest.main()
