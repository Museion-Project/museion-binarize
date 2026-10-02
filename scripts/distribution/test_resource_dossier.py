"""Inventory/notice evidence never implies release; inspect has no subprocess."""
import json
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


if __name__=='__main__':unittest.main()
