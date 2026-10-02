"""Bounded readiness probe through the App's production Rust launcher API.

Two readiness requests only; negative fixtures must be rejected before Python.
Mutates a new private copy only. No OCR, GUI, signature, network or source PDF.
"""
import argparse
import json
import os
import shlex
import shutil
import signal
import subprocess
import time
from pathlib import Path

from scripts.ocr.app_mvp_bridge.runtime_contract import sha, verify_inventory


def inventory(root):
    return {p.relative_to(root).as_posix(): sha(p) for p in root.rglob('*') if p.is_file()}


def probe(launcher, resource_root, output):
    launcher = Path(launcher).resolve(strict=True)
    root = Path(resource_root).resolve(strict=True)
    output = Path(output)
    if not output.is_absolute() or any(p.is_symlink() for p in (output, *output.parents)):
        raise ValueError('PROBE_OUTPUT_ABSOLUTE_NO_SYMLINK_REQUIRED')
    output = output.resolve()
    if output == root or root in output.parents or output in root.parents:
        raise ValueError('PROBE_SOURCE_OUTPUT_OVERLAP')
    manifest_path = root / 'runtime-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    verify_inventory(root, manifest)
    before = inventory(root)
    launcher_sha = sha(launcher)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    copy = output / 'relocated resources with spaces'
    shutil.copytree(root, copy)
    marker = output / 'must never execute marker'
    startup = output / 'poison startup.py'
    startup.write_text("from pathlib import Path\nPath(" + repr(str(marker)) + ").write_text('unexpected')\n")
    calls = []

    def invoke(name, package, error=None):
        # Poison harmless parent Python settings. DYLD injection is verified by
        # pure Rust command tests, not by injecting into this Rust process itself.
        env = dict(os.environ)
        for key in ('DYLD_INSERT_LIBRARIES', 'DYLD_LIBRARY_PATH', 'DYLD_FRAMEWORK_PATH'):
            env.pop(key, None)
        env.update(PYTHONHOME='/does-not-exist/poison', PYTHONPATH=str(output),
                   PYTHONSTARTUP=str(startup), PYTHONINSPECT='1',
                   MUSEION_MVP_RUNTIME_CONFIG='/does-not-exist/poison',
                   MUSEION_LOCAL_RESOURCE_ROOT='/does-not-exist/poison',
                   MUSEION_LOCAL_SESSION_ROOT='/does-not-exist/poison',
                   MUSEION_LOCAL_LEGACY_SESSION_ROOT='/does-not-exist/poison')
        command = [str(launcher), str(package), str(output / 'unused private sessions')]
        started = time.monotonic()
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, env=env, start_new_session=True)
        timeout = False
        try:
            stdout, stderr = process.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            timeout = True
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
        (output / (name + '.stdout.json')).write_text(stdout)
        (output / (name + '.stderr.log')).write_text(stderr)
        try:
            response = json.loads(stdout)
            if not isinstance(response, dict):
                raise ValueError('not an object')
        except ValueError as failure:
            response = dict(status='PROTOCOL_FAILED', message=str(failure))
        proof = response.get('launcher_runtime_proof', {})
        bridge = response.get('bridge', {})
        loaded = bridge.get('loaded_runtime_proof', {})
        if error:
            matched = (process.returncode == 2 and response.get('launched') is False and
                       response.get('phase') in ('prepare', 'pre-spawn') and
                       error in response.get('message', '') and not marker.exists())
        else:
            matched = (process.returncode == 0 and response.get('launched') is True and
                       response.get('python_pid', 0) > 0 and bridge.get('local_runtime_ready') is True and
                       proof.get('inventory_verified_before_spawn') is True and
                       proof.get('preparation_identity_rechecked') is True and
                       proof.get('resource_root') == str(package) and
                       proof.get('manifest_sha256') == before['runtime-manifest.json'] and
                       proof.get('inventory_files') == len(manifest['runtime_files']) and
                       proof.get('code_files') == 11 and proof.get('manifest_trusted') is False and
                       proof.get('atomic_fd_exec_proven') is False and
                       all(loaded.get(stage, {}).get('modules') and
                           loaded[stage].get('resource_manifest_sha256') == proof['manifest_sha256']
                           for stage in ('before', 'after')) and not marker.exists())
        call = dict(name=name, command=command, diagnostic_pid=process.pid, returncode=process.returncode,
                    wall_seconds=time.monotonic()-started, timeout=timeout, expected_error=error,
                    launched=response.get('launched'), python_pid=response.get('python_pid'),
                    phase=response.get('phase'), response_message=response.get('message'),
                    matched_expected=matched and not timeout, poisoned_parent_python_environment=True,
                    native_start_calls=0, recognizer_calls=0,
                    readiness_language_list_checks=1 if not error and matched else 0)
        calls.append(call)
        (output / 'calls.json').write_text(json.dumps(calls, indent=2))
        return response

    original = invoke('original-readiness', root)
    relocated = invoke('relocated-readiness', copy)
    path = copy / 'runtime-manifest.json'
    raw = path.read_bytes()
    for name, error, mutate in (
        ('partial-code', 'RESOURCE_CODE_WHITELIST', lambda m: m['code_files'].pop(next(iter(m['code_files'])))),
        ('conflicting-hash', 'RESOURCE_CODE_INVENTORY_CONFLICT', lambda m: m['runtime_files'].update({next(iter(m['code_files'])): '0'*64})),
        ('unhashed-grc', 'RESOURCE_TESSDATA_UNHASHED', lambda m: m['runtime_files'].pop(m['tessdata']+'/grc.traineddata')),
        ('path-alias', 'RESOURCE_PATH_ESCAPE', lambda m: m['runtime_files'].update({'python//bad': '0'*64})),
        ('startup-hook', 'RESOURCE_IMPLICIT_RUNTIME_HOOK', lambda m: m['runtime_files'].update({'python/evil.pth': '0'*64})),
        ('nonlocal-code', 'RESOURCE_NONLOCAL_CODE', lambda m: m['runtime_files'].update({'scripts/ocr/paid_mvp/evil.py': '0'*64})),
        ('nonlocal-mode', 'RESOURCE_LOCAL_MODE_REQUIRED', lambda m: m.update(modes=['local', 'paid'])),
    ):
        bad = json.loads(raw)
        mutate(bad)
        try:
            path.write_text(json.dumps(bad))
            invoke(name, copy, error)
        finally:
            path.write_bytes(raw)
    for name, relative, error in (
        ('replaced-interpreter', manifest['python'], 'RESOURCE_HASH_MISMATCH'),
        ('changed-session-code', 'scripts/ocr/app_mvp_bridge/sessions.py', 'RESOURCE_HASH_MISMATCH'),
        ('changed-stdlib', 'python/lib/python3.11/json/__init__.py', 'RESOURCE_HASH_MISMATCH'),
    ):
        target = copy / relative
        data = target.read_bytes()
        mode = target.stat().st_mode
        try:
            if name == 'replaced-interpreter':
                target.write_text('#!/bin/sh\nprintf unexpected > ' + shlex.quote(str(marker)) + '\n')
            else:
                target.write_bytes(data + b'\n# owned fault copy\n')
            invoke(name, copy, error)
        finally:
            target.write_bytes(data)
            target.chmod(mode)
    extra = copy / 'python' / 'undeclared.py'
    try:
        extra.write_text('# owned fixture\n')
        invoke('undeclared-file', copy, 'RESOURCE_INVENTORY_MISSING_OR_EXTRA')
    finally:
        extra.unlink()
    target = copy / 'scripts/ocr/app_mvp_bridge/sessions.py'
    data, mode = target.read_bytes(), target.stat().st_mode
    try:
        target.unlink()
        target.symlink_to(root / 'scripts/ocr/app_mvp_bridge/sessions.py')
        invoke('symlink-code', copy, 'RESOURCE_SYMLINK')
    finally:
        target.unlink()
        target.write_bytes(data)
        target.chmod(mode)
    verify_inventory(root, manifest)
    verify_inventory(copy, manifest)
    preserved = inventory(root) == before and inventory(copy) == before
    observations = {}
    for name, response in [('original', original), ('relocated', relocated)]:
        loaded = response.get('bridge', {}).get('loaded_runtime_proof', {})
        observations[name] = dict(launcher=response.get('launcher_runtime_proof'),
            before_modules=len(loaded.get('before', {}).get('modules', [])),
            after_modules=len(loaded.get('after', {}).get('modules', [])),
            extension_modules=loaded.get('after', {}).get('extension_modules'))
    report = dict(schema='local-launcher-probe/1',
        state='COMPONENT_PASS' if preserved and all(c['matched_expected'] for c in calls) else 'COMPONENT_FAILED',
        launcher=str(launcher), launcher_sha256=launcher_sha, launcher_unchanged=sha(launcher)==launcher_sha,
        probe_sha256=sha(__file__), source_inventory=before, source_inventory_preserved=preserved,
        resource_copy_restored=preserved, calls=calls, observations=observations,
        diagnostic_invocations=len(calls), observed_python_children=sum(c['launched'] is True for c in calls),
        child_launch_accounting_complete=all(c['launched'] in (False, True) for c in calls),
        negative_pre_exec_rejections=sum(c['expected_error'] is not None and c['matched_expected'] for c in calls),
        unexpected_interpreter_marker_created=marker.exists(), native_start_calls=0,
        recognizer_calls=0, network_requests=0, gui_sessions=0, codesign_calls=0,
        readiness_language_list_checks=sum(c['readiness_language_list_checks'] for c in calls),
        distribution_ready=False, quality_ready=False, release_ready=False,
        limitations=['Shared production Rust API; diagnostic is not a Tauri GUI or ordinary App workflow',
            'Unsigned manifest provides consistency only, not authenticated supplier identity',
            'Path verification does not atomically bind the executed file descriptor',
            'Same Mac and observed loaded modules only; independent installation and all lazy/dynamic paths unverified'])
    (output / 'summary.json').write_text(json.dumps(report, indent=2))
    return {key: report[key] for key in ('state', 'diagnostic_invocations', 'observed_python_children',
                                       'negative_pre_exec_rejections', 'native_start_calls', 'distribution_ready')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--launcher', type=Path, required=True)
    parser.add_argument('--resource-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = probe(args.launcher, args.resource_root, args.output)
    print(json.dumps(result))
    if result['state'] != 'COMPONENT_PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
