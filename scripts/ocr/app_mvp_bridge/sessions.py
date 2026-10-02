"""Private local sessions and source/runtime-bound resume; no migration or OCR."""
import hashlib
import json
import os
import sys
from pathlib import Path


def check(ok, message):
    if not ok:
        raise ValueError(message)


def roots(config, *, create=False):
    primary = Path(config['session_root']).expanduser()
    check(primary.is_absolute(), 'SESSION_ROOT_ABSOLUTE_REQUIRED')
    values = [primary, *(Path(p) for p in config.get('legacy_session_roots', []))]
    result = []
    for path in values:
        check(path.is_absolute() and not any(p.is_symlink() and not (str(p) in ('/var','/tmp') and p.resolve()==Path('/private'+str(p))) for p in (path, *path.parents)),
              'SESSION_ROOT_SYMLINK_OR_RELATIVE')
        path = path.resolve()
        check(path != Path('/') and not any(p.name.endswith('.app') for p in (path, *path.parents)),
              'SESSION_RESOURCE_ROOT_PROHIBITED')
        if path.exists():
            check(path.is_dir() and path.stat().st_uid == os.getuid(), 'SESSION_ROOT_OWNER_REQUIRED')
            if path == primary.resolve() and config.get('persistent_sessions'):
                check(path.stat().st_mode & 0o077 == 0, 'SESSION_ROOT_PRIVATE_REQUIRED')
        if path not in result:
            result.append(path)
    if create:
        result[0].mkdir(mode=0o700, parents=True, exist_ok=True)
    return result


def folder_for(sid, config):
    check(type(sid) is str and len(sid) == 32 and all(c in '0123456789abcdef' for c in sid),
          'INVALID_SESSION')
    matches = []
    for root in roots(config):
        folder = root / sid
        check(not folder.is_symlink(), 'SESSION_PATH_ESCAPE')
        if (folder / 'desktop-session.json').is_file():
            matches.append(folder)
    check(len(matches) == 1, 'SESSION_NOT_FOUND_OR_AMBIGUOUS')
    return matches[0]


def runtime_binding(config):
    from scripts.ocr.mvp.local import runtime_config
    from scripts.ocr.mvp.core import CONFIG_VERSION, CONSUMER_POLICY
    from scripts.ocr.mvp.store import EXPORTER_VERSION
    repo = Path(__file__).resolve().parents[3]
    names = ['app_mvp_bridge/' + name for name in
             ('__init__.py', '__main__.py', 'sessions.py', 'save_recovery.py', 'runtime_contract.py')]
    names += ['mvp/' + name for name in ('__init__.py', 'core.py', 'local.py', 'store.py')]
    names += ['free_local/pipeline.py']
    sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
    local = runtime_config(config.get('local'))
    dependencies = {}
    paths = [local[k] for k in ('apple_helper', 'tesseract', 'font')]
    paths += local.get('fallback_font_paths', [])
    tessdata = config.get('tessdata') or os.environ.get('TESSDATA_PREFIX')
    if tessdata:
        paths += [str(Path(tessdata) / (lang + '.traineddata')) for lang in ('eng', 'grc')]
    manifest = Path(config['package_root']) / 'runtime-manifest.json' if config.get('package_root') else None
    for name in paths:
        path = Path(name).resolve()
        dependencies[str(path)] = sha(path) if path.is_file() else None
    binding = dict(schema='local-session-runtime/1', config_version=CONFIG_VERSION,
                   consumer_policy=CONSUMER_POLICY, exporter_version=EXPORTER_VERSION,
                   code={name: sha(repo / 'scripts/ocr' / name) for name in names},
                   python=str(Path(sys.executable).resolve()), python_sha256=sha(sys.executable),
                   local_config=local, dependencies=dependencies, tessdata_root=tessdata,
                   runtime_manifest_sha256=sha(manifest) if manifest and manifest.is_file() else None)
    binding['sha256'] = hashlib.sha256(json.dumps(binding, sort_keys=True,
                                                ensure_ascii=False).encode()).hexdigest()
    return binding


def compatibility(meta, config):
    if not meta.get('runtime_binding'):
        return 'unverified'
    return 'verified' if meta['runtime_binding'] == runtime_binding(config) else 'mismatch'


def mutation_guard(meta, config):
    # Historical external-development fixtures keep their earlier source-only
    # component contract. The packaged App never enables that compatibility.
    if meta.get('runtime_binding') or config.get('require_runtime_binding'):
        check(compatibility(meta, config) == 'verified', 'SESSION_RUNTIME_UNVERIFIED_OR_CHANGED')
