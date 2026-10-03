import json
import unittest
from pathlib import Path
from unittest.mock import patch

from . import package_local_app_resources as app
from . import package_local_candidate as pack
from . import test_local_candidate as local_fixture


class LocalAppResourcesTests(unittest.TestCase):
    def setUp(self):
        # An explicitly synthetic payload uses the normal producer and verifier.
        self.fixture = local_fixture.LocalCandidateTests()
        self.fixture.setUp()
        self.root, self.repo = self.fixture.root, self.fixture.repo
        self.local = self.root / 'local'
        pack.package(self.repo, self.fixture.freeze_path, self.fixture.runtime_path,
                     self.root / 'local-stage', self.local)
        for name in app.NOTICES:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('synthetic notice ' + name)
        self.library = self.root / 'libpdfium.dylib'
        self.library.write_bytes(b'synthetic non-executable library')
        self.manifest = self.root / 'pdfium.toml'
        self.asset = dict(target_triple=app.TARGET, os='macos', arch='aarch64',
                          library_filename='libpdfium.dylib', library_sha256=pack.sha(self.library))
        self.write_manifest()
        self.output = self.root / 'desktop'
        self.arguments = dict(repo=self.repo, local_resources=self.local,
                              local_manifest_sha256=pack.sha(self.local / 'runtime-manifest.json'),
                              pdfium_library=self.library, pdfium_manifest=self.manifest,
                              pdfium_manifest_sha256=pack.sha(self.manifest), target=app.TARGET,
                              staging=self.root / 'desktop-stage', output=self.output)

    def write_manifest(self, extra=''):
        self.manifest.write_text('schema = "mpdf-pdfium-manifest"\nschema_version = "1.0"\n[[asset]]\n' +
                                 '\n'.join(f'{k} = {json.dumps(v)}' for k, v in self.asset.items()) + '\n' + extra)

    def tearDown(self):
        self.fixture.tearDown()

    def test_composition_preserves_local_payload_and_maps_pdfium_to_resource_root(self):
        before = {str(p.relative_to(self.local)): pack.sha(p) for p in self.local.rglob('*') if p.is_file()}
        result = app.compose(**self.arguments)
        receipt = json.loads((self.output / 'resource-composition.json').read_text())
        overlay = json.loads((self.output / 'tauri.local-App.overlay.json').read_text())
        self.assertEqual(overlay['bundle']['macOS']['minimumSystemVersion'], '27.0')
        mappings = overlay['bundle']['resources']
        self.assertEqual(mappings[str(self.local)], 'local-ocr')
        self.assertEqual(mappings[str(self.output / 'pdfium' / '*')], './')
        self.assertIsNone(mappings['old-model.py'])
        self.assertEqual(len(receipt['copied_files']), 6)
        self.assertFalse(receipt['PDFium_loaded'])
        self.assertFalse(receipt['license_obligations_verified'])
        self.assertFalse(result['release_ready'])
        self.assertEqual(before, {str(p.relative_to(self.local)): pack.sha(p) for p in self.local.rglob('*') if p.is_file()})
        self.assertFalse(list(self.arguments['staging'].iterdir()))
        with self.assertRaisesRegex(ValueError, 'OUTPUT_NOT_NEW'):
            app.compose(**self.arguments)

    def test_manifest_library_target_and_declared_notices_are_required(self):
        for key, value, message in [('local_manifest_sha256', 'wrong', 'LOCAL_MANIFEST_HASH'),
                                    ('pdfium_manifest_sha256', 'wrong', 'PDFIUM_MANIFEST_HASH'),
                                    ('target', 'x86_64-apple-darwin', 'TARGET_UNVERIFIED')]:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, message):
                app.compose(**{**self.arguments, key: value})
        self.library.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'PDFIUM_LIBRARY_IDENTITY'):
            app.compose(**self.arguments)
        self.library.write_bytes(b'synthetic non-executable library')
        (self.repo / app.NOTICES[-1]).unlink()
        with self.assertRaisesRegex(ValueError, 'INPUT_REQUIRED'):
            app.compose(**self.arguments)
        self.assertFalse(self.output.exists())

    def test_wrong_architecture_and_duplicate_manifest_target_rejected(self):
        self.asset['arch'] = 'x86_64'; self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'PDFIUM_TARGET_IDENTITY'):
            app.compose(**{**self.arguments, 'pdfium_manifest_sha256': pack.sha(self.manifest)})
        self.asset['arch'] = 'aarch64'
        self.write_manifest('[[asset]]\ntarget_triple = "' + app.TARGET + '"\n')
        with self.assertRaisesRegex(ValueError, 'PDFIUM_TARGET_IDENTITY'):
            app.compose(**{**self.arguments, 'pdfium_manifest_sha256': pack.sha(self.manifest)})

    def test_changed_local_code_extra_files_and_input_symlinks_rejected(self):
        code = self.repo / pack.SOURCE_FILES[0]; original = code.read_bytes(); code.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'LOCAL_SOURCE_CHANGED'):
            app.compose(**self.arguments)
        code.write_bytes(original)
        extra = self.local / 'other-mode.py'; extra.touch()
        with self.assertRaisesRegex(ValueError, 'INVENTORY_MISSING_OR_EXTRA'):
            app.compose(**self.arguments)
        extra.unlink()
        saved = self.root / 'saved-library'; self.library.rename(saved); self.library.symlink_to(saved)
        with self.assertRaisesRegex(ValueError, 'INPUT_SYMLINK'):
            app.compose(**self.arguments)

    def test_input_or_config_change_during_copy_cannot_publish(self):
        original = self.library.read_bytes()
        def change_library(folder):
            self.library.write_bytes(b'changed after copy'); return []
        with patch.object(app, 'macho_audit', side_effect=change_library):
            with self.assertRaisesRegex(ValueError, 'INPUT_CHANGED'):
                app.compose(**self.arguments)
        self.assertFalse(self.output.exists())
        self.library.write_bytes(original)
        def change_config(folder):
            self.fixture.app_config.write_text('{"bundle":{"resources":{"new-model":"model"}}}'); return []
        with patch.object(app, 'macho_audit', side_effect=change_config):
            with self.assertRaisesRegex(ValueError, 'CONFIG_CHANGED'):
                app.compose(**self.arguments)
        self.assertFalse(self.output.exists())

    def test_copy_failure_and_output_race_preserve_inputs_and_competing_files(self):
        with patch.object(app.shutil, 'copyfile', side_effect=OSError('no space')):
            with self.assertRaisesRegex(OSError, 'no space'):
                app.compose(**self.arguments)
        self.assertFalse(self.output.exists())
        def competing(folder):
            self.output.mkdir(); (self.output / 'user-file').write_text('preserve'); return []
        with patch.object(app, 'macho_audit', side_effect=competing):
            with self.assertRaises(FileExistsError):
                app.compose(**self.arguments)
        self.assertEqual((self.output / 'user-file').read_text(), 'preserve')

    def test_overlapping_or_protected_output_rejected(self):
        for key, value, message in [('staging', self.local / 'new', 'SCOPE_OVERLAP'),
                                    ('output', self.local / 'new', 'SCOPE_OVERLAP'),
                                    ('output', self.root / 'evidence/new', 'PROTECTED_OUTPUT')]:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, message):
                app.compose(**{**self.arguments, key: value})


if __name__ == '__main__':
    unittest.main()
