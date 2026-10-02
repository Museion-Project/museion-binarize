"""Local resource integrity and observed Python loading; no release attestation.

The manifest is an unsigned input. Matching it proves consistency, not trusted
provenance, licensing, signatures, every lazy import, or helper execution.
"""
import hashlib
import json
import re
import sys
from pathlib import Path

SOURCE_FILES = tuple('scripts/ocr/' + p for p in (
    'app_mvp_bridge/__init__.py', 'app_mvp_bridge/__main__.py',
    'app_mvp_bridge/sessions.py', 'app_mvp_bridge/save_recovery.py',
    'app_mvp_bridge/runtime_contract.py',
    'mvp/__init__.py', 'mvp/__main__.py', 'mvp/core.py', 'mvp/local.py',
    'mvp/store.py', 'free_local/pipeline.py'))
INITIALIZERS = {'scripts/__init__.py', 'scripts/ocr/__init__.py'}
BUILD_METADATA = {'runtime-manifest.json', 'resource-build.json', 'tauri.generated.overlay.json'}


def check(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def resource_path(root, name):
    check(type(name) is str and bool(name), 'RESOURCE_PATH_ESCAPE')
    relative = Path(name)
    check(not relative.is_absolute() and '..' not in relative.parts and
          name == relative.as_posix() and name != '.', 'RESOURCE_PATH_ESCAPE')
    path = root / relative
    check(not any(p.is_symlink() for p in (path, *path.parents) if p.is_relative_to(root)),
          'RESOURCE_SYMLINK: ' + name)
    check(path.resolve().is_relative_to(root), 'RESOURCE_PATH_ESCAPE')
    return path


def verify_inventory(root, manifest):
    root = Path(root).resolve()
    check(manifest.get('schema') == 'museion-local-runtime/1', 'RESOURCE_MANIFEST_SCHEMA')
    code, files = manifest.get('code_files'), manifest.get('runtime_files')
    check(type(code) is dict and type(files) is dict and code and files, 'RESOURCE_IDENTITY_INCOMPLETE')
    check(set(code) == set(SOURCE_FILES), 'RESOURCE_CODE_WHITELIST_INCOMPLETE_OR_EXTRA')
    check(set(code) <= set(files) and all(code[p] == files[p] for p in code), 'RESOURCE_CODE_INVENTORY_CONFLICT')
    check(INITIALIZERS <= set(files), 'RESOURCE_INITIALIZERS_MISSING')
    check(manifest.get('modes') == ['local'], 'RESOURCE_LOCAL_MODE_REQUIRED')
    for name, digest in files.items():
        path = resource_path(root, name)
        check(type(digest) is str and re.fullmatch('[0-9a-f]{64}', digest), 'RESOURCE_HASH_FORMAT')
        check(path.is_file() and sha(path) == digest, 'RESOURCE_HASH_MISMATCH: ' + name)
        check(path.parts[len(root.parts)] in ('scripts', 'python', 'native', 'bin', 'fonts', 'tessdata', 'notices', 'provenance'),
              'RESOURCE_UNDECLARED_NAMESPACE: ' + name)
        if name.startswith('scripts/'):
            check(name in set(SOURCE_FILES) | INITIALIZERS, 'RESOURCE_NONLOCAL_CODE: ' + name)
        check(path.suffix != '.pth' and not path.name.startswith(('sitecustomize', 'usercustomize')),
              'RESOURCE_IMPLICIT_RUNTIME_HOOK: ' + name)
    for key in ('python', 'apple_helper', 'tesseract', 'font'):
        check(manifest.get(key) in files, 'RESOURCE_CONTRACT_UNHASHED: ' + key)
    tessdata = manifest.get('tessdata')
    resource_path(root, tessdata)
    for language in ('eng', 'grc'):
        check(tessdata + '/' + language + '.traineddata' in files, 'RESOURCE_TESSDATA_UNHASHED: ' + language)
    actual = set()
    for path in root.rglob('*'):
        check(not path.is_symlink(), 'RESOURCE_SYMLINK: ' + str(path.relative_to(root)))
        if path.is_file():
            actual.add(path.relative_to(root).as_posix())
    check(actual - BUILD_METADATA == set(files), 'RESOURCE_INVENTORY_MISSING_OR_EXTRA')
    return dict(schema='local-inventory-proof/1', files=len(files), code_files=len(code),
                manifest_trusted=False, distribution_ready=False)


def loaded_proof(config, *, modules=None, executable=None):
    """Hash observed origins only. Called before dispatch and after each response."""
    root = Path(config['package_root']).resolve()
    manifest_path = root / 'runtime-manifest.json'
    check(sha(manifest_path) == config['resource_manifest_sha256'], 'RESOURCE_MANIFEST_CHANGED')
    manifest = json.loads(manifest_path.read_text())
    files = manifest['runtime_files']
    python = Path(executable or sys.executable).resolve()
    check(python == resource_path(root, manifest['python']).resolve() and sha(python) == files[manifest['python']],
          'RESOURCE_EXECUTABLE_MISMATCH')
    records = []
    for name, module in sorted((sys.modules if modules is None else modules).copy().items()):
        origin = getattr(module, '__file__', None)
        if not origin:
            continue  # built-in/frozen modules have no file claim
        path = Path(origin).resolve()
        check(path.is_relative_to(root), 'RESOURCE_LOADED_OUTSIDE: ' + name)
        relative = path.relative_to(root).as_posix()
        check(relative in files and path.is_file() and sha(path) == files[relative], 'RESOURCE_LOADED_HASH_MISMATCH: ' + name)
        records.append(dict(module=name, path=relative, sha256=files[relative]))
    return dict(schema='local-loaded-python-proof/1', python=manifest['python'],
                python_sha256=files[manifest['python']], modules=records,
                extension_modules=[r for r in records if r['path'].endswith(('.so', '.dylib'))],
                resource_manifest_sha256=config['resource_manifest_sha256'],
                all_lazy_imports_proven=False, native_helper_execution_proven=False,
                system_dynamic_libraries_proven=False, distribution_ready=False)
