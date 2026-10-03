import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from . import package_local_candidate as pack


class LocalCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name).resolve();self.repo=self.root/'repo'
        for name in pack.SOURCE_FILES:
            file=self.repo/name;file.parent.mkdir(parents=True,exist_ok=True);file.write_text('')
        (self.repo/'scripts/ocr/mvp/core.py').write_text("CONFIG_VERSION='test-v2'\n")
        (self.repo/'scripts/ocr/mvp/store.py').write_text("EXPORTER_VERSION='export-v1'\n")
        self.app_config=self.repo/'apps/desktop/src-tauri/tauri.conf.json'
        self.app_config.parent.mkdir(parents=True)
        self.app_config.write_text(json.dumps(dict(bundle=dict(resources={'old-bookmarks.py':'bookmarks','old-model.py':'hierarchy_models'}))))
        self.freeze=dict(schema='local-code-freeze/1',config_version='test-v2',exporter_version='export-v1',
                         files={p:pack.sha(self.repo/p) for p in pack.SOURCE_FILES})
        deps=self.root/'deps';deps.mkdir();source=deps/'helper.swift';source.write_text('// test fixture')
        paths=['python/bin/python','bin/apple-accurate','bin/tesseract','fonts/test.ttf','tessdata/eng.traineddata','tessdata/grc.traineddata']
        files=[]
        for i,path in enumerate(paths):
            file=deps/str(i);file.write_bytes(str(i).encode())
            files.append(dict(source=str(file),path=path,sha256=pack.sha(file),component='python'))
        self.runtime=dict(schema='local-dependency-inputs/1',files=files,
                          runtime=dict(python=paths[0],apple_helper=paths[1],tesseract=paths[2],font=paths[3],tessdata='tessdata'),
                          helper_build=dict(schema='apple-helper-build/1',binary_sha256=files[1]['sha256'],
                                            source_path=str(source),source_sha256=pack.sha(source),
                                            compiler_version='test fixture',sdk='test fixture',command=['test fixture']))
        self.freeze_path=self.root/'freeze.json';self.runtime_path=self.root/'runtime.json'
        self.freeze_path.write_text(json.dumps(self.freeze));self.runtime_path.write_text(json.dumps(self.runtime))
    def tearDown(self):self.temp.cleanup()
    def preflight(self,**changes):
        return pack.preflight(self.repo,changes.get('freeze',self.freeze),changes.get('runtime',self.runtime),self.root/'stage',self.root/'output')
    def test_valid_preflight_has_no_mutation_and_excludes_evaluation(self):
        files,version,_,_=self.preflight();self.assertEqual(version,'test-v2')
        self.assertFalse((self.root/'stage').exists());self.assertFalse((self.root/'output').exists())
        self.assertTrue(all('development_panel' not in f['path'] and 'paid_mvp' not in f['path'] for f in files))
    def test_scope_hash_and_versions_rejected(self):
        bad=copy.deepcopy(self.freeze);bad['files']['scripts/ocr/paid_mvp/selector.py']='x'
        with self.assertRaisesRegex(ValueError,'CODE_FREEZE'):self.preflight(freeze=bad)
        bad=copy.deepcopy(self.freeze);bad['config_version']='stale'
        with self.assertRaisesRegex(ValueError,'CONFIG_VERSION'):self.preflight(freeze=bad)
        (self.repo/pack.SOURCE_FILES[0]).write_text('changed')
        with self.assertRaisesRegex(ValueError,'CODE_SOURCE_HASH'):self.preflight()
    def test_escape_duplicate_and_nonlocal_inputs_rejected(self):
        for key,value,error in [('path','../evil','PATH_ESCAPE'),('component','cllg','NON_LOCAL')]:
            bad=copy.deepcopy(self.runtime);bad['files'][0][key]=value
            with self.assertRaisesRegex(ValueError,error):self.preflight(runtime=bad)
        bad=copy.deepcopy(self.runtime);bad['files'].append(bad['files'][0])
        with self.assertRaisesRegex(ValueError,'DUPLICATE_PACKAGE_PATH'):self.preflight(runtime=bad)
    def test_nonempty_stage_old_output_and_evidence_rejected(self):
        stage=self.root/'stage';stage.mkdir();(stage/'old').touch()
        with self.assertRaisesRegex(ValueError,'OUTPUT_NOT_NEW_OR_EMPTY'):self.preflight()
        with self.assertRaisesRegex(ValueError,'PROTECTED_OUTPUT'):pack.output_guard(self.root/'docs/evidence/new')
        with self.assertRaisesRegex(ValueError,'OUTPUT_NOT_NEW_OR_EMPTY'):pack.output_guard(stage)
    def test_source_hash_mismatch_or_missing_helper_provenance_rejected(self):
        bad=copy.deepcopy(self.runtime);bad['files'][0]['sha256']='x'
        with self.assertRaisesRegex(ValueError,'DEPENDENCY_SOURCE_HASH'):self.preflight(runtime=bad)
        bad=copy.deepcopy(self.runtime);bad.pop('helper_build')
        with self.assertRaisesRegex(ValueError,'HELPER_BUILD_PROVENANCE'):self.preflight(runtime=bad)
    def test_macho_failure_does_not_publish_output(self):
        with patch.object(pack,'macho_audit',side_effect=ValueError('external dependency')):
            with self.assertRaisesRegex(ValueError,'external dependency'):
                pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        self.assertFalse((self.root/'output').exists());self.assertEqual(list((self.root/'stage').iterdir()),[])
    def test_published_resource_does_not_overwrite_or_claim_release(self):
        result=pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        self.assertFalse(result['distribution_ready'])
        original=(self.root/'output/runtime-manifest.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'OUTPUT_NOT_NEW_OR_EMPTY'):
            pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        self.assertEqual((self.root/'output/runtime-manifest.json').read_bytes(),original)
    def test_symlink_and_implicit_python_hook_rejected(self):
        link=self.root/'link';link.symlink_to(self.repo,target_is_directory=True)
        with self.assertRaisesRegex(ValueError,'OUTPUT_SYMLINK'):pack.output_guard(link/'new')
        with self.assertRaisesRegex(ValueError,'IMPLICIT_RUNTIME_HOOK'):pack.relative('python/lib/sitecustomize.py')
        with self.assertRaisesRegex(ValueError,'PACKAGE_PATH_ESCAPE'):pack.relative('python//bin/python')

    def test_loader_path_rpath_is_local_and_external_rpath_rejected(self):
        bundle=self.root/'bundle';bundle.mkdir();binary=bundle/'binary';binary.write_bytes(b'\xcf\xfa\xed\xfe')
        def otool(args,**kwargs):
            if args[1]=='-L':return str(binary)+':\n\t/usr/lib/libSystem.B.dylib (compatibility version 1)\n'
            if args[1]=='-D':return str(binary)+':\n'
            return 'cmd LC_RPATH\ncmdsize 32\npath @loader_path (offset 12)\n'
        with patch.object(pack.subprocess,'check_output',side_effect=otool):
            self.assertEqual(len(pack.macho_audit(bundle)),1)
        with patch.object(pack.subprocess,'check_output',side_effect=lambda args,**kw:otool(args,**kw).replace('@loader_path','/opt/homebrew/lib')):
            with self.assertRaisesRegex(ValueError,'EXTERNAL_MACHO_PATH'):pack.macho_audit(bundle)

    def test_relocation_requires_frozen_source_and_absolute_rpath(self):
        bad=copy.deepcopy(self.runtime);file=bad['files'][0]
        bad['remove_rpaths']=[dict(path=file['path'],source_sha256='wrong',rpaths=['/opt/old'])]
        with self.assertRaisesRegex(ValueError,'RELOCATION_SOURCE_IDENTITY'):self.preflight(runtime=bad)
        bad['remove_rpaths'][0]['source_sha256']=file['sha256'];bad['remove_rpaths'][0]['rpaths']=['@loader_path']
        with self.assertRaisesRegex(ValueError,'RELOCATION_SCOPE'):self.preflight(runtime=bad)

    def test_output_race_preserves_competing_directory(self):
        def race(root):
            output=self.root/'output';output.mkdir();(output/'user-file').write_text('preserve');return []
        with patch.object(pack,'macho_audit',side_effect=race):
            with self.assertRaises(FileExistsError):
                pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        self.assertEqual((self.root/'output/user-file').read_text(),'preserve')

    def test_local_overlay_removes_default_and_platform_resource_mappings(self):
        platform=self.app_config.parent/'tauri.macos.conf.json'
        platform.write_text(json.dumps(dict(bundle=dict(resources={'platform-model.py':'model'}))))
        result=pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        overlay=json.loads((self.root/'output/tauri.generated.overlay.json').read_text())
        self.assertEqual(overlay['bundle']['resources'],{'old-bookmarks.py':None,'old-model.py':None,
            'platform-model.py':None,str(self.root/'output'):'local-ocr'})
        receipt=json.loads((self.root/'output/resource-build.json').read_text())
        self.assertEqual(len(receipt['app_config_inputs']),2)
        self.assertFalse(result['distribution_ready'])

    def test_changed_app_config_cannot_publish_a_stale_resource_recipe(self):
        original=self.app_config.read_bytes()
        def changed(root):
            self.app_config.write_text(json.dumps(dict(bundle=dict(resources={'new-model.py':'model'}))))
            return []
        with patch.object(pack,'macho_audit',side_effect=changed):
            with self.assertRaisesRegex(ValueError,'APP_CONFIG_CHANGED_DURING_BUILD'):
                pack.package(self.repo,self.freeze_path,self.runtime_path,self.root/'stage',self.root/'output')
        self.assertFalse((self.root/'output').exists())
        self.assertEqual(list((self.root/'stage').iterdir()),[])
        self.assertNotEqual(self.app_config.read_bytes(),original)

    def test_unknown_config_format_missing_base_or_symlink_rejected(self):
        alternate=self.app_config.parent/'tauri.macos.conf.json5';alternate.write_text('{}')
        with self.assertRaisesRegex(ValueError,'APP_CONFIG_FORMAT_UNSUPPORTED'):
            pack.local_app_overlay(self.repo,self.root/'output')
        alternate.unlink();saved=self.root/'saved-config.json';self.app_config.rename(saved)
        with self.assertRaisesRegex(ValueError,'APP_BASE_CONFIG_REQUIRED'):
            pack.local_app_overlay(self.repo,self.root/'output')
        self.app_config.symlink_to(saved)
        with self.assertRaisesRegex(ValueError,'APP_CONFIG_SYMLINK'):
            pack.local_app_overlay(self.repo,self.root/'output')


if __name__=='__main__':unittest.main()
