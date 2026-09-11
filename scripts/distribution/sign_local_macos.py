#!/usr/bin/env python3
"""Sign with an existing Developer ID identity; notarize via a Keychain profile.

No private keys, passwords or API keys are exported or accepted as arguments.
Signing and notarization are separate; a successful signature is not notarization.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from sign_macos_release import sign_and_verify


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def bundle_digest(app):
    records = []
    for path in sorted(app.rglob('*')):
        if path.is_symlink():
            records.append([str(path.relative_to(app)), 'link', str(path.readlink())])
        elif path.is_file():
            records.append([str(path.relative_to(app)), 'file', digest(path)])
    return hashlib.sha256(json.dumps(records, separators=(',', ':')).encode()).hexdigest()


def run(command, **kwargs):
    kwargs["check"] = True
    return subprocess.run(command, **kwargs)


def sign(app, identity, receipt, runner=run):
    if not app.is_dir() or app.suffix != '.app':
        raise ValueError('An existing .app is required')
    if not identity.startswith('Developer ID Application: '):
        raise ValueError('A Developer ID Application identity is required; no ad-hoc fallback')
    if receipt.exists():
        raise FileExistsError(receipt)
    sign_and_verify(app, identity, developer_id=True, runner=runner)
    details = runner(['codesign', '-dv', '--verbose=4', str(app)], capture_output=True, text=True)
    text = details.stderr
    if 'Authority=Developer ID Application:' not in text or 'TeamIdentifier=not set' in text or 'TeamIdentifier=' not in text:
        raise ValueError('Signed bundle has no verified Developer ID authority/team')
    if 'runtime' not in text or 'Sealed Resources=none' in text:
        raise ValueError('Hardened runtime and sealed resources are required')
    value = dict(schema='mpdf-local-signing/1', app_name=app.name,
                 signing_state='developer_id', notarization_state='not_submitted',
                 bundle_sha256=bundle_digest(app), verification=text)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text(json.dumps(value, indent=2)+'\n')
    return value


def notarize(dmg, profile, receipt, runner=run):
    if not dmg.is_file() or dmg.suffix != '.dmg' or not profile.strip():
        raise ValueError('An existing DMG and a Keychain profile name are required')
    if receipt.exists():
        raise FileExistsError(receipt)
    before = digest(dmg)
    response = runner(['xcrun', 'notarytool', 'submit', str(dmg), '--keychain-profile', profile,
                       '--wait', '--output-format', 'json'], capture_output=True, text=True)
    result = json.loads(response.stdout)
    if result.get('status') != 'Accepted':
        raise ValueError('Notarization not accepted: '+str(result.get('status')))
    runner(['xcrun', 'stapler', 'staple', str(dmg)])
    runner(['xcrun', 'stapler', 'validate', str(dmg)])
    assessment = runner(['spctl', '-a', '-vv', '--type', 'open', '--context', 'context:primary-signature', str(dmg)], capture_output=True, text=True)
    assessment_text = (assessment.stdout or '') + (assessment.stderr or '')
    gatekeeper = 'accepted' if 'source=Notarized Developer ID' in assessment_text and 'override=' not in assessment_text else 'unverified'
    value = dict(schema='mpdf-local-notarization/1', notarization_state='accepted_stapled',
                 submission_id=result.get('id'), submitted_sha256=before, dmg_sha256=digest(dmg),
                 gatekeeper_assessment=gatekeeper, assessment_details=assessment_text)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text(json.dumps(value, indent=2)+'\n')
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='phase', required=True)
    p = sub.add_parser('sign');p.add_argument('--app', type=Path, required=True);p.add_argument('--identity', required=True);p.add_argument('--receipt', type=Path, required=True)
    p = sub.add_parser('notarize');p.add_argument('--dmg', type=Path, required=True);p.add_argument('--profile', required=True);p.add_argument('--receipt', type=Path, required=True)
    args = parser.parse_args()
    value = sign(args.app,args.identity,args.receipt) if args.phase == 'sign' else notarize(args.dmg,args.profile,args.receipt)
    print(json.dumps(value))


if __name__ == '__main__':
    main()
