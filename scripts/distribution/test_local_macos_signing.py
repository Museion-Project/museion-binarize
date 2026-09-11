import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import sign_local_macos as signing


class SigningTests(unittest.TestCase):
    def test_real_runner_accepts_shared_signer_check_argument(self):
        import sys
        result=signing.run([sys.executable,'-c','print("runner-ok")'],check=True,capture_output=True,text=True)
        self.assertEqual(result.stdout.strip(),'runner-ok')

    def test_no_adhoc_or_wrong_certificate_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            app=Path(tmp)/'Fixture.app';app.mkdir()
            for identity in ['-', 'Apple Development: Test', '']:
                with self.assertRaises(ValueError): signing.sign(app,identity,Path(tmp)/'receipt.json')
    def test_rejected_notarization_never_staples_or_writes_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg=Path(tmp)/'fixture.dmg';dmg.write_bytes(b'fixture');receipt=Path(tmp)/'receipt.json';commands=[]
            def fake(command,**kwargs):
                commands.append(command);return subprocess.CompletedProcess(command,0,stdout=json.dumps({'status':'Invalid'}))
            with self.assertRaises(ValueError):signing.notarize(dmg,'profile',receipt,runner=fake)
            self.assertEqual(len(commands),1);self.assertFalse(receipt.exists())
    def test_disabled_gatekeeper_acceptance_is_not_policy_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg=Path(tmp)/'fixture.dmg';dmg.write_bytes(b'fixture')
            def fake(command,**kwargs):
                if command[1]=='notarytool':return subprocess.CompletedProcess(command,0,stdout=json.dumps({'status':'Accepted','id':'test'}))
                return subprocess.CompletedProcess(command,0,stdout='',stderr='accepted\nsource=no usable signature\noverride=security disabled')
            result=signing.notarize(dmg,'profile',Path(tmp)/'receipt.json',runner=fake)
            self.assertEqual(result['notarization_state'],'accepted_stapled')
            self.assertEqual(result['gatekeeper_assessment'],'unverified')
    def test_stapling_failure_never_claims_ready(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg=Path(tmp)/'fixture.dmg';dmg.write_bytes(b'fixture');receipt=Path(tmp)/'receipt.json'
            def fake(command,**kwargs):
                if command[1]=='notarytool':return subprocess.CompletedProcess(command,0,stdout=json.dumps({'status':'Accepted','id':'test'}))
                raise subprocess.CalledProcessError(1,command)
            with self.assertRaises(subprocess.CalledProcessError):signing.notarize(dmg,'profile',receipt,runner=fake)
            self.assertFalse(receipt.exists())


if __name__=='__main__': unittest.main()
