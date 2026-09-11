import json,subprocess,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import macos_local_readiness as readiness
import package_macos_dmg
from sign_local_macos import bundle_digest

class MacOSLocalRelease(unittest.TestCase):
    def test_signing_alone_and_missing_records_cannot_pass_notarization(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);app=root/'Fixture.app';app.mkdir();(app/'file').write_text('signed fixture placeholder')
            dmg=root/'fixture.dmg';dmg.write_bytes(b'dmg fixture')
            evidence=dict(schema='mpdf-macos-local-evidence/1',target='aarch64-apple-darwin',source_commit='a'*40,app=dict(path=str(app),bundle_sha256=bundle_digest(app)),dmg=dict(path=str(dmg),sha256=readiness.digest(dmg)),records={})
            path=root/'evidence.json';path.write_text(json.dumps(evidence))
            result=readiness.evaluate(path);self.assertEqual(result['developer_id_notary'],'pending')
            (app/'file').write_text('changed')
            self.assertEqual(readiness.evaluate(path)['macos_release_artifacts'],'fail')
    def test_dmg_preserves_symlinks_and_refuses_existing_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);app=root/'Fixture.app';app.mkdir();(app/'payload').write_text('data');(app/'link').symlink_to('payload')
            def package(command,**kwargs):
                staged=Path(command[command.index('-srcfolder')+1])/'Fixture.app'
                self.assertTrue((staged/'link').is_symlink());self.assertEqual((staged/'link').readlink(),Path('payload'))
                Path(command[-1]).write_bytes(b'test dmg')
                return subprocess.CompletedProcess(command,0)
            with patch.object(package_macos_dmg.subprocess,'run',side_effect=package):
                output=package_macos_dmg.build_dmg(app,'0.1.0-rc.3','aarch64-apple-darwin',root/'out')
                with self.assertRaises(FileExistsError):package_macos_dmg.build_dmg(app,'0.1.0-rc.3','aarch64-apple-darwin',root/'out')
                self.assertEqual(output.read_bytes(),b'test dmg')

if __name__=='__main__':unittest.main()
