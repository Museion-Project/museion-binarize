"""Explicit local resource relocation/loading probe. Never starts an OCR task.

Readiness calls Tesseract --list-langs. Faults mutate only this probe's new copy;
source resources and user sessions are untouched. This is not an App/GUI,
independent-Mac, recognition, license, signature or release acceptance.
"""
import argparse
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from scripts.ocr.app_mvp_bridge.runtime_contract import sha, verify_inventory


def probe(resource_root, output):
    root = Path(resource_root).resolve(strict=True)
    output = Path(output)
    if not output.is_absolute() or output.is_symlink() or any(p.is_symlink() for p in output.parents):
        raise ValueError('PROBE_OUTPUT_ABSOLUTE_NO_SYMLINK_REQUIRED')
    output = output.resolve()
    if output == root or root in output.parents or output in root.parents:
        raise ValueError('PROBE_SOURCE_OUTPUT_OVERLAP')
    manifest = json.loads((root / 'runtime-manifest.json').read_text())
    verify_inventory(root, manifest)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    copy = output / 'relocated resources with spaces'
    shutil.copytree(root, copy)
    verify_inventory(copy, manifest)
    before = {p.relative_to(copy).as_posix(): sha(p) for p in copy.rglob('*') if p.is_file()}
    calls = []

    def invoke(name, package, expected_error=None, *, inject=False, contract_only=False):
        env = {k: v for k, v in os.environ.items() if k not in
               ('PYTHONHOME', 'PYTHONPATH', 'DYLD_LIBRARY_PATH', 'DYLD_FRAMEWORK_PATH',
                'MUSEION_MVP_RUNTIME_CONFIG', 'MUSEION_LOCAL_LEGACY_SESSION_ROOT')}
        env.update(PYTHONPATH=str(package), PYTHONNOUSERSITE='1', PYTHONDONTWRITEBYTECODE='1',
                   PATH='/usr/bin:/bin:/usr/sbin:/sbin', MUSEION_LOCAL_RESOURCE_ROOT=str(package),
                   MUSEION_LOCAL_SESSION_ROOT=str(output / 'unused private sessions'))
        command = [str(package / manifest['python']), '-s', '-B']
        if contract_only:
            # Separate component API, not the bridge readiness/App path.
            script = "import os,json;from pathlib import Path;from scripts.ocr.app_mvp_bridge.runtime_contract import verify_inventory,loaded_proof,sha;root=Path(os.environ['MUSEION_LOCAL_RESOURCE_ROOT']);manifest=json.loads((root/'runtime-manifest.json').read_text());inventory=verify_inventory(root,manifest);config=dict(package_root=str(root),resource_manifest_sha256=sha(root/'runtime-manifest.json'));proof=loaded_proof(config);print(json.dumps(dict(contract_inventory_verified=True,loaded_runtime_proof=dict(before=proof,after=loaded_proof(config)),runtime_proof=dict(resource_root=str(root)))))"
            if inject:
                script = "import sys,types;sys.modules['probe_external_origin']=types.SimpleNamespace(__file__=sys.argv[1]);" + script
            script = "try:\n " + script + "\nexcept Exception as error:\n print(json.dumps(dict(status='failed',message=str(error))));raise SystemExit(1)"
            # json is needed even if the very first contract import fails.
            command += ['-c', 'import json\n' + script]
            if inject:
                command.append(str(output / 'external-origin.py'))
        else:
            command += ['-m', 'scripts.ocr.app_mvp_bridge']
        request = dict(component_api='runtime_contract') if contract_only else dict(mode='local', action='readiness')
        started = time.monotonic()
        result = subprocess.run(command, input=json.dumps(request), capture_output=True, text=True,
                                cwd=package, env=env, timeout=60)
        (output / (name + '.stdout.json')).write_text(result.stdout)
        (output / (name + '.stderr.log')).write_text(result.stderr)
        try:
            response = json.loads(result.stdout)
            if not isinstance(response, dict):
                raise ValueError('not a JSON object')
        except ValueError as error:
            response = dict(status='PROTOCOL_FAILED', message=str(error))
        passed = (result.returncode != 0 and expected_error in response.get('message', '')) if expected_error else (
            result.returncode == 0 and response.get('contract_inventory_verified' if contract_only else 'local_runtime_ready') is True and
            bool(response.get('loaded_runtime_proof', {}).get('before', {}).get('modules')) and
            bool(response.get('loaded_runtime_proof', {}).get('after', {}).get('modules')))
        calls.append(dict(name=name, command=command, request=request, returncode=result.returncode,
                          wall_seconds=time.monotonic() - started, expected_error=expected_error,
                          response_status=response.get('status'), response_message=response.get('message'),
                          termination_signal=-result.returncode if result.returncode < 0 else None,
                          matched_expected=passed, deliberately_injected_origin=inject, standalone_component_contract=contract_only,
                          recognizer_calls=0, readiness_language_list_checks=1 if not expected_error and passed and not contract_only else 0))
        (output / 'calls.json').write_text(json.dumps(calls, indent=2))
        return response

    original = invoke('original-readiness', root)
    relocated = invoke('relocated-readiness', copy)
    original_contract=invoke('original-contract', root, contract_only=True)
    relocated_contract=invoke('relocated-contract', copy, contract_only=True)
    path = copy / 'runtime-manifest.json'
    raw = path.read_bytes()
    for name, error, mutate in (
        ('partial-code', 'RESOURCE_CODE_WHITELIST', lambda m: m['code_files'].pop(next(iter(m['code_files'])))),
        ('conflicting-hash', 'RESOURCE_CODE_INVENTORY_CONFLICT', lambda m: m['runtime_files'].update({next(iter(m['code_files'])): '0' * 64})),
        ('unhashed-grc', 'RESOURCE_TESSDATA_UNHASHED', lambda m: m['runtime_files'].pop(m['tessdata'] + '/grc.traineddata')),
    ):
        bad = json.loads(raw)
        mutate(bad)
        try:
            path.write_text(json.dumps(bad))
            invoke(name, copy, error)
        finally:
            path.write_bytes(raw)
    code = copy / 'scripts/ocr/app_mvp_bridge/sessions.py'
    raw_code = code.read_bytes()
    try:
        code.write_bytes(raw_code + b'\n# probe-owned copy changed\n')
        invoke('changed-session-code', copy, 'RESOURCE_HASH_MISMATCH')
    finally:
        code.write_bytes(raw_code)
    extra = copy / 'python' / 'probe_undeclared.py'
    try:
        extra.write_text('# not in inventory\n')
        invoke('undeclared-module', copy, 'RESOURCE_INVENTORY_MISSING_OR_EXTRA')
    finally:
        extra.unlink()
    (output / 'external-origin.py').write_text('# identity-only fault, never executed\n')
    invoke('external-loaded-origin', copy, 'RESOURCE_LOADED_OUTSIDE', inject=True, contract_only=True)
    verify_inventory(copy, manifest)
    after = {p.relative_to(copy).as_posix(): sha(p) for p in copy.rglob('*') if p.is_file()}
    if before != after:
        raise ValueError('PROBE_RESOURCE_COPY_NOT_RESTORED')
    verify_inventory(root, manifest)
    observations = {}
    for name, response in [('original_bridge', original), ('relocated_bridge', relocated), ('original_contract', original_contract), ('relocated_contract', relocated_contract)]:
        proof = response.get('loaded_runtime_proof')
        if not proof:
            observations[name] = dict(loaded_runtime_proof_available=False, status=response.get('status'), message=response.get('message'))
            continue
        observations[name] = dict(before_modules=len(proof['before']['modules']),
                                  after_modules=len(proof['after']['modules']),
                                  extension_modules=proof['after']['extension_modules'],
                                  manifest_sha256=proof['after']['resource_manifest_sha256'],
                                  runtime_root=response['runtime_proof']['resource_root'])
    report = dict(schema='local-relocated-runtime-probe/1', state='COMPONENT_PASS' if all(c['matched_expected'] for c in calls) else 'COMPONENT_FAILED', calls=calls,
                  observations=observations, resource_copy_restored=True, source_inventory_preserved=True,
                  native_start_calls=0, recognizer_calls=0, network_requests=0, gui_sessions=0,
                  distribution_ready=False, quality_ready=False, release_ready=False,
                  probe_sha256=sha(__file__), limitations=[
                      'same Mac and unsigned manifest; no trusted source or independent-machine attestation',
                      'observed Python modules only; lazy imports, all dynamic libraries and helper execution pending',
                      'readiness only, no ordinary App or recognition/review/save workflow',
                      'license, model blob provenance and signing unresolved'])
    (output / 'summary.json').write_text(json.dumps(report, indent=2))
    return {k: report[k] for k in ('state', 'native_start_calls', 'recognizer_calls', 'distribution_ready')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resource-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result=probe(args.resource_root, args.output)
    print(json.dumps(result))
    if result['state'] != 'COMPONENT_PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
