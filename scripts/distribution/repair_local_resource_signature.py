"""Repair one explicitly authorized OpenBLAS signature in a new resource copy.

This CLI records an existing user decision; the input record grants no permission
on its own. It never signs another object, overwrites an output, downloads, starts
OCR, installs an App, notarizes or publishes. Failed copies/receipts are retained.
"""
import argparse
import copy
import datetime
import json
import shutil
import subprocess
from pathlib import Path
from scripts.ocr.app_mvp_bridge.runtime_contract import resource_path, sha, verify_inventory
from .package_local_candidate import macho_audit, output_guard

LIBRARY = 'python/lib/python3.11/site-packages/numpy/.dylibs/libopenblas64_.0.dylib'


def check(ok, message):
    if not ok:
        raise ValueError(message)


def write_new(path, value):
    with path.open('x', encoding='utf-8') as out:
        json.dump(value, out, ensure_ascii=False, indent=2)


def inventory(root):
    return {p.relative_to(root).as_posix(): sha(p) for p in root.rglob('*') if p.is_file()}


def preflight(preview_path, authorization_path, evidence):
    preview_path, authorization_path = Path(preview_path), Path(authorization_path)
    preview = json.loads(preview_path.read_text())
    authorization = json.loads(authorization_path.read_text())
    check(preview.get('schema') == 'local-adhoc-signing-preview/1' and
          authorization.get('schema') == 'local-signature-authorization-record/1', 'REPAIR_INPUT_SCHEMA')
    check(authorization.get('preview_sha256') == sha(preview_path) and
          bool(authorization.get('user_source')) and authorization.get('extra_signing_allowed') is False,
          'REPAIR_AUTHORIZATION_RECORD_BINDING')
    source = Path(preview['input_resource_root'])
    check(source.is_absolute() and not any(p.is_symlink() for p in (source, *source.parents)), 'REPAIR_SOURCE_PATH')
    source = source.resolve(strict=True)
    target = output_guard(preview['target_new_copy'])
    check(Path(preview['target_new_copy']).is_absolute(), 'REPAIR_TARGET_ABSOLUTE_REQUIRED')
    check(source != target and source not in target.parents and target not in source.parents, 'REPAIR_SCOPE_OVERLAP')
    check(preview.get('relative_library') == LIBRARY, 'REPAIR_ONE_LIBRARY_ONLY')
    command = ['/usr/bin/codesign', '--force', '--sign', '-', str(target / LIBRARY)]
    check(preview.get('proposed_command') == command and preview.get('costs_usd') == 0, 'REPAIR_COMMAND_SCOPE')
    for name, key in [('runtime-manifest.json', 'input_runtime_manifest_sha256'),
                      ('resource-build.json', 'input_resource_build_sha256')]:
        check(sha(source / name) == preview[key], 'REPAIR_INPUT_CHANGED: ' + name)
    manifest = json.loads((source / 'runtime-manifest.json').read_text())
    build = json.loads((source / 'resource-build.json').read_text())
    verify_inventory(source, manifest)
    entries = build['files']
    check(len(entries) == preview['input_inventory_files'] and
          len({e['path'] for e in entries}) == len(entries) and
          {e['path'] for e in entries} == set(manifest['runtime_files']) | {'runtime-manifest.json'},
          'REPAIR_BUILD_INVENTORY')
    check(all(sha(resource_path(source, e['path'])) == e['sha256'] for e in entries), 'REPAIR_BUILD_HASH_CHANGED')
    check(sha(source / LIBRARY) == preview['library_current_sha256'] == manifest['runtime_files'][LIBRARY],
          'REPAIR_LIBRARY_CHANGED')
    evidence = Path(evidence)
    check(evidence.is_absolute() and not any(p.is_symlink() for p in (evidence, *evidence.parents)),
          'REPAIR_EVIDENCE_PATH')
    evidence = evidence.resolve()
    check(not evidence.exists(), 'REPAIR_EVIDENCE_NOT_NEW')
    check(all(evidence != p and p not in evidence.parents and evidence not in p.parents for p in (source, target)),
          'REPAIR_EVIDENCE_SCOPE_OVERLAP')
    return preview, source, target, evidence, manifest, build, command


def repair(preview_path, authorization_path, evidence):
    preview, source, target, evidence, manifest, build, command = preflight(preview_path, authorization_path, evidence)
    source_before = inventory(source)
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    receipt = dict(schema='local-signature-repair/1', state='PREPARED', source=str(source), target=str(target),
                   preview_sha256=sha(preview_path), authorization_record_sha256=sha(authorization_path),
                   repair_tool_sha256=sha(__file__), time_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   relative_library=LIBRARY, library_before_sha256=preview['library_current_sha256'],
                   signing_scope='one copied dylib, ad-hoc identity', command=command,
                   native_start_calls=0, recognizer_calls=0, cloud_calls=0, gui_sessions=0,
                   distribution_ready=False, release_ready=False)
    write_new(evidence / 'prepared.json', receipt)
    try:
        # Exclusive copy; never use a historical bundle or hard-linked source.
        shutil.copytree(source, target)
        check(inventory(target) == source_before, 'REPAIR_COPY_CHANGED')
        check(not (target / LIBRARY).samefile(source / LIBRARY), 'REPAIR_COPY_NOT_INDEPENDENT')
        write_new(evidence / 'sign-intent.json', dict(command=command, target_sha256=sha(target / LIBRARY),
                                                    note='one invocation only; this intent is not a successful signature receipt'))

        def run(args, name):
            proc = subprocess.run(args, capture_output=True, text=True, timeout=30)
            record = dict(command=args, returncode=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)
            write_new(evidence / (name + '.json'), record)
            check(proc.returncode == 0, 'REPAIR_' + name.upper().replace('-', '_') + '_FAILED')
            return record

        receipt['sign'] = run(command, 'sign')
        receipt['signature_verify'] = run(['/usr/bin/codesign', '--verify', '--verbose=2', str(target / LIBRARY)], 'verify')
        receipt['signature_display'] = run(['/usr/bin/codesign', '--display', '--verbose=4', str(target / LIBRARY)], 'display')
        check('Signature=adhoc' in receipt['signature_display']['stderr'], 'REPAIR_NOT_ADHOC')
        after_sign = inventory(target)
        changed = sorted(p for p, h in source_before.items() if after_sign.get(p) != h)
        check(changed == [LIBRARY] and set(after_sign) == set(source_before), 'REPAIR_UNEXPECTED_SIGN_MUTATION')
        receipt['library_after_sha256'] = sha(target / LIBRARY)
        closure = macho_audit(target)
        write_new(evidence / 'macho.json', dict(objects=closure, total=len(closure)))
        # New derived metadata retains the parent receipts unchanged at source.
        new_manifest = copy.deepcopy(manifest)
        new_manifest['runtime_files'][LIBRARY] = receipt['library_after_sha256']
        new_manifest['derived_signature_repair'] = dict(parent_manifest_sha256=preview['input_runtime_manifest_sha256'],
                                                       authorization_record_sha256=sha(authorization_path),
                                                       library=LIBRARY, before=receipt['library_before_sha256'],
                                                       after=receipt['library_after_sha256'], identity='ad-hoc')
        (target / 'runtime-manifest.json').write_text(json.dumps(new_manifest, indent=2))
        verify_inventory(target, new_manifest)
        signature_step = copy.deepcopy(receipt)
        signature_step['state'] = 'SIGNATURE_VERIFIED_STATIC_CLOSURE_PASS'
        new_build = copy.deepcopy(build)
        new_build.update(state='RESOURCE_ADHOC_REPAIR_CANDIDATE',
                         parent_resource_build_sha256=preview['input_resource_build_sha256'],
                         signing_performed=True, signing_scope=[LIBRARY],
                         signature_repair=signature_step, macho=closure,
                         signatures_unverified=True, distribution_ready=False, release_ready=False)
        # native_mutations are the retained rpath-stage receipts. Their signature
        # status belongs to that stage, not the later signature_repair stage.
        new_build['files'] = [dict(path=p.relative_to(target).as_posix(), sha256=sha(p), bytes=p.stat().st_size)
                              for p in sorted(target.rglob('*')) if p.is_file() and
                              p.name not in ('resource-build.json', 'tauri.generated.overlay.json')]
        (target / 'resource-build.json').write_text(json.dumps(new_build, indent=2))
        overlay = json.loads((target / 'tauri.generated.overlay.json').read_text())
        overlay['bundle']['resources'] = {str(target): 'local-ocr'}
        (target / 'tauri.generated.overlay.json').write_text(json.dumps(overlay, indent=2))
        final = inventory(target)
        expected = {LIBRARY, 'runtime-manifest.json', 'resource-build.json', 'tauri.generated.overlay.json'}
        check(set(final) == set(source_before) and {p for p in final if final[p] != source_before[p]} == expected,
              'REPAIR_UNEXPECTED_ARTIFACT_MUTATION')
        check(inventory(source) == source_before, 'REPAIR_SOURCE_CHANGED')
        receipt.update(state='ADHOC_REPAIR_COMPONENT_PASS', source_preserved=True, changed_files=sorted(expected),
                       runtime_manifest_sha256=sha(target / 'runtime-manifest.json'),
                       resource_build_sha256=sha(target / 'resource-build.json'),
                       static_macho_objects=len(closure), whole_bundle_signatures_verified=False,
                       runtime_probe_pending=True)
        write_new(evidence / 'result.json', receipt)
        return {k: receipt[k] for k in ('state', 'target', 'static_macho_objects', 'distribution_ready')}
    except BaseException as error:
        write_new(evidence / 'failure.json', dict(state='FAILED', error_type=type(error).__name__, message=str(error),
                                                 target=str(target), artifacts_retained=True, retry_performed=False))
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('preview', 'authorization-record', 'evidence'):
        parser.add_argument('--' + arg, type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(repair(args.preview, args.authorization_record, args.evidence)))


if __name__ == '__main__':
    main()
