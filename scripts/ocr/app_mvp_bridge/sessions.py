"""Private local sessions and source/runtime-bound resume; no migration or OCR."""
import hashlib
import json
import os
import sys
import fcntl
import stat
import time
import tempfile
from contextlib import contextmanager
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


@contextmanager
def session_control(folder):
    """Short control lease, independent of the whole-task producer lease.

    The immutable original metadata inode is shared by attempt publication and
    cancellation, so an old cancellation cannot race past a new client token.
    No PID marker is inferred or removed; the descriptor is not inherited.
    """
    path = Path(folder) / 'desktop-session.json'
    check(not path.is_symlink(), 'SESSION_METADATA_SYMLINK')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        check(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.getuid(),
              'SESSION_CONTROL_FILE_UNSAFE')
        deadline = time.monotonic() + 2
        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                check(time.monotonic() < deadline, 'SESSION_CONTROL_BUSY')
                time.sleep(.02)
        current = path.lstat()
        check((current.st_dev, current.st_ino) == (info.st_dev, info.st_ino) and current.st_nlink == 1,
              'SESSION_CONTROL_FILE_CHANGED')
        yield
    finally:
        os.close(fd)


def safe_record(path):
    path = Path(path)
    check(not path.is_symlink(), 'PROCESSING_RECORD_UNSAFE')
    info = path.stat()
    check(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.getuid(),
          'PROCESSING_RECORD_UNSAFE')
    return json.loads(path.read_text())


def current_attempt(folder, meta):
    folder = Path(folder)
    pointer = folder / 'active-processing.json'
    if not pointer.exists() and not pointer.is_symlink():
        return meta.get('client_operation_id')
    active = safe_record(pointer)
    token = active.get('client_operation_id') if type(active) is dict else None
    check(type(token) is str and len(token) == 32 and all(c in '0123456789abcdef' for c in token),
          'PROCESSING_RECORD_INVALID')
    name = 'processing-attempt-' + token + '.json'
    check(active.get('schema') == 'desktop-active-processing/1' and active.get('record') == name,
          'PROCESSING_RECORD_INVALID')
    record = safe_record(folder / name)
    sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
    check(type(record) is dict and record.get('schema') == 'desktop-processing-attempt/1' and
          record.get('session_id') == folder.name and record.get('client_operation_id') == token and
          active.get('record_sha256') == sha(folder / name) and
          record.get('original_metadata_sha256') == sha(folder / 'desktop-session.json') and
          record.get('job_sha256') == sha(folder / 'operation/job.json'), 'PROCESSING_RECORD_CHANGED')
    return token


def continuation_state(folder):
    """Bind the displayed disk state; never treat it as proof of worker liveness."""
    folder = Path(folder)
    values = {}
    paths = [folder / name for name in ('desktop-session.json', 'operation/job.json',
                                       'operation/progress.json', 'active-processing.json')]
    raw = folder / 'operation/raw'
    check(not raw.is_symlink(), 'CONTINUATION_PATH_UNSAFE')
    if raw.exists():
        for path in raw.rglob('*'):
            check(not path.is_symlink(), 'CONTINUATION_PATH_UNSAFE')
            check(path.is_file() or path.is_dir(), 'CONTINUATION_PATH_UNSAFE')
            if path.is_file():
                paths.append(path)
    for path in paths:
        check(not path.is_symlink(), 'CONTINUATION_PATH_UNSAFE')
        values[str(path.relative_to(folder))] = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
    return hashlib.sha256(json.dumps(values, sort_keys=True).encode()).hexdigest()


def publish_attempt(folder, meta, task, request, cached):
    """Called inside task_producer. Preserve originals and every previous token."""
    from scripts.ocr.mvp.core import digest
    folder = Path(folder)
    token = request['client_operation_id']
    with session_control(folder):
        check(safe_record(folder / 'desktop-session.json') == meta, 'SESSION_METADATA_CHANGED')
        check(not any((folder / 'operation' / name).exists() or (folder / 'operation' / name).is_symlink()
                      for name in ('CANCEL', 'cancel')), 'CONTINUATION_CANCELLED')
        check(continuation_state(folder) == request.get('continuation_state_sha256'),
              'CONTINUATION_STATE_CHANGED')
        previous = current_attempt(folder, meta)
        check(token != previous and token != meta.get('client_operation_id'), 'CONTINUATION_TOKEN_REUSED')
        pointer = folder / 'active-processing.json'
        prior = safe_record(pointer) if pointer.exists() else None
        record = dict(schema='desktop-processing-attempt/1', session_id=folder.name,
                      client_operation_id=token, previous_client_operation_id=previous,
                      previous_active_record=prior, original_metadata_sha256=hashlib.sha256((folder / 'desktop-session.json').read_bytes()).hexdigest(),
                      previous_checkpoint=safe_record(folder / 'operation/progress.json') if (folder / 'operation/progress.json').is_file() else None,
                      job_sha256=hashlib.sha256((folder / 'operation/job.json').read_bytes()).hexdigest(),
                      source_sha256=meta['source_sha256'], runtime_binding_sha256=meta['runtime_binding']['sha256'],
                      physical_pages=task['page_numbers'], retained_pages={str(n): digest(p) for n, p in cached.items()},
                      remaining_pages=[n for n in task['page_numbers'] if n not in cached],
                      document_id=request.get('document_id'), document_page_count=request.get('document_page_count'),
                      observed_state_sha256=request['continuation_state_sha256'], note='Explicit processing attempt; not completion or worker-liveness evidence')
        name = 'processing-attempt-' + token + '.json'
        with (folder / name).open('x', encoding='utf-8') as out:
            json.dump(record, out, ensure_ascii=False, indent=2);out.flush();os.fsync(out.fileno())
        active = dict(schema='desktop-active-processing/1', client_operation_id=token, record=name,
                      record_sha256=hashlib.sha256((folder / name).read_bytes()).hexdigest())
        fd, temporary = tempfile.mkstemp(prefix='.active-processing-', dir=folder)
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as out:
                json.dump(active, out, ensure_ascii=False, indent=2);out.flush();os.fsync(out.fileno())
            os.replace(temporary, pointer)
            fd = os.open(folder, os.O_RDONLY)
            try:os.fsync(fd)
            finally:os.close(fd)
        finally:
            if Path(temporary).exists():Path(temporary).unlink()
