"""Resource contract faults, synthetic files only; no OCR or release evidence."""
import copy
import json
import tempfile
import types
import unittest
from pathlib import Path
from .runtime_contract import SOURCE_FILES, INITIALIZERS, verify_inventory, loaded_proof, sha


class RuntimeContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        for name in set(SOURCE_FILES) | INITIALIZERS | {'python/bin/python', 'bin/helper', 'bin/tesseract',
                                                       'fonts/font.ttf', 'tessdata/eng.traineddata', 'tessdata/grc.traineddata'}:
            file = self.root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(name)
        self.manifest = dict(schema='museion-local-runtime/1', modes=['local'], python='python/bin/python',
                             apple_helper='bin/helper', tesseract='bin/tesseract', font='fonts/font.ttf', tessdata='tessdata',
                             code_files={p: sha(self.root / p) for p in SOURCE_FILES},
                             runtime_files={p.relative_to(self.root).as_posix(): sha(p) for p in self.root.rglob('*') if p.is_file()})
        self.write_manifest()

    def tearDown(self):
        self.temp.cleanup()

    def write_manifest(self):
        (self.root / 'runtime-manifest.json').write_text(json.dumps(self.manifest))
        self.config = dict(package_root=str(self.root), resource_manifest_sha256=sha(self.root / 'runtime-manifest.json'))

    def verify(self):
        return verify_inventory(self.root, self.manifest)

    def proof(self, **kwargs):
        return loaded_proof(self.config, executable=kwargs.get('executable', self.root / 'python/bin/python'),
                            modules=kwargs.get('modules', {'core': types.SimpleNamespace(__file__=str(self.root / SOURCE_FILES[0])),
                                                          'frozen': types.SimpleNamespace()}))

    def test_complete_inventory_and_observed_loaded_identity_are_separate(self):
        result = self.verify()
        self.assertFalse(result['manifest_trusted'])
        proof = self.proof()
        self.assertEqual(len(proof['modules']), 1)
        self.assertFalse(proof['all_lazy_imports_proven'])
        self.assertFalse(proof['native_helper_execution_proven'])

    def test_missing_manifest_code_or_inventory_is_not_legacy_pass(self):
        for key in ('code_files', 'runtime_files'):
            bad = copy.deepcopy(self.manifest)
            bad.pop(key)
            with self.assertRaisesRegex(ValueError, 'RESOURCE_IDENTITY_INCOMPLETE'):
                verify_inventory(self.root, bad)

    def test_partial_or_extra_code_whitelist_rejected(self):
        for action in ('missing', 'extra'):
            bad = copy.deepcopy(self.manifest)
            if action == 'missing':
                bad['code_files'].pop(SOURCE_FILES[0])
            else:
                bad['code_files']['scripts/ocr/paid_mvp/pipeline.py'] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'RESOURCE_CODE_WHITELIST'):
                verify_inventory(self.root, bad)

    def test_overlap_hash_conflict_cannot_be_hidden_by_dict_merge(self):
        self.manifest['runtime_files'][SOURCE_FILES[0]] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'RESOURCE_CODE_INVENTORY_CONFLICT'):
            self.verify()

    def test_undeclared_module_and_cached_bytecode_rejected(self):
        for name in ('scripts/ocr/app_mvp_bridge/extra.py', 'python/lib/__pycache__/extra.pyc'):
            file = self.root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text('extra')
            with self.assertRaisesRegex(ValueError, 'RESOURCE_INVENTORY_MISSING_OR_EXTRA'):
                self.verify()
            file.unlink()

    def test_declared_nonlocal_module_or_python_hook_rejected(self):
        for name, error in [('scripts/ocr/paid_mvp/selector.py', 'RESOURCE_NONLOCAL_CODE'),
                            ('python/lib/evil.pth', 'RESOURCE_IMPLICIT_RUNTIME_HOOK')]:
            file = self.root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text('extra')
            self.manifest['runtime_files'][name] = sha(file)
            with self.assertRaisesRegex(ValueError, error):
                self.verify()
            self.manifest['runtime_files'].pop(name)
            file.unlink()

    def test_tessdata_and_executable_must_be_inventory_bound(self):
        for name, error in [('tessdata/grc.traineddata', 'RESOURCE_TESSDATA_UNHASHED'),
                            ('python/bin/python', 'RESOURCE_CONTRACT_UNHASHED')]:
            bad = copy.deepcopy(self.manifest)
            bad['runtime_files'].pop(name)
            with self.assertRaisesRegex(ValueError, error):
                verify_inventory(self.root, bad)

    def test_hash_change_and_equal_bytes_symlink_rejected(self):
        file = self.root / 'bin/helper'
        file.write_text('changed')
        with self.assertRaisesRegex(ValueError, 'RESOURCE_HASH_MISMATCH'):
            self.verify()
        file.write_text('bin/helper')
        external = self.root.parent / (self.root.name + '-external')
        external.write_bytes(file.read_bytes())
        try:
            file.unlink()
            file.symlink_to(external)
            with self.assertRaisesRegex(ValueError, 'RESOURCE_SYMLINK'):
                self.verify()
        finally:
            external.unlink()

    def test_path_alias_escape_and_local_mode_rejected(self):
        for name in ('../escape', 'python//bin/python'):
            bad = copy.deepcopy(self.manifest)
            bad['runtime_files'][name] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'RESOURCE_PATH_ESCAPE'):
                verify_inventory(self.root, bad)
        self.manifest['modes'] = ['local', 'paid']
        with self.assertRaisesRegex(ValueError, 'RESOURCE_LOCAL_MODE_REQUIRED'):
            self.verify()

    def test_loaded_module_outside_or_modified_after_inventory_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'RESOURCE_LOADED_OUTSIDE'):
            self.proof(modules={'injected': types.SimpleNamespace(__file__=__file__)})
        (self.root / SOURCE_FILES[0]).write_text('changed during request')
        with self.assertRaisesRegex(ValueError, 'RESOURCE_LOADED_HASH_MISMATCH'):
            self.proof()

    def test_loaded_executable_and_manifest_changes_rejected(self):
        with self.assertRaisesRegex(ValueError, 'RESOURCE_EXECUTABLE_MISMATCH'):
            self.proof(executable=__file__)
        (self.root / 'runtime-manifest.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'RESOURCE_MANIFEST_CHANGED'):
            self.proof()

    def test_missing_namespace_initializer_rejected(self):
        self.manifest['runtime_files'].pop('scripts/__init__.py')
        with self.assertRaisesRegex(ValueError, 'RESOURCE_INITIALIZERS_MISSING'):
            self.verify()


if __name__ == '__main__':
    unittest.main()
