"""Private local sessions and source/runtime-bound resume; no migration or OCR."""
import hashlib
import json
import os
import sys
import fcntl
import stat
import time
import tempfile
import copy
import shutil
import uuid
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


def recovery_state(folder):
    """Observed immutable history, not a claim that a process has stopped."""
    folder = Path(folder)
    check(not folder.is_symlink(), 'RECOVERY_PATH_UNSAFE')
    files = {}
    for path in folder.rglob('*'):
        check(not path.is_symlink() and (path.is_file() or path.is_dir()), 'RECOVERY_PATH_UNSAFE')
        if path.is_file():
            files[str(path.relative_to(folder))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()


def recovery_successor(folder, meta, config):
    pointer = Path(folder) / 'PROCESSING_SUCCESSOR.json'
    if not pointer.exists() and not pointer.is_symlink():
        return None
    value = safe_record(pointer)
    check(type(value) is dict and value.get('schema') == 'local-processing-successor/1' and
          value.get('previous_session_id') == Path(folder).name and value.get('source_sha256') == meta['source_sha256'],
          'RECOVERY_SUCCESSOR_INVALID')
    target = folder_for(value.get('session_id'), config)
    child = safe_record(target / 'desktop-session.json')
    sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
    check(child.get('source_sha256') == meta['source_sha256'] and
          child.get('processing_recovery', {}).get('previous_session_id') == Path(folder).name and
          value.get('metadata_sha256') == sha(target / 'desktop-session.json') and
          value.get('job_sha256') == sha(target / 'operation/job.json'), 'RECOVERY_SUCCESSOR_CHANGED')
    return target.name


def processing_recovery_plan(folder, meta, config):
    """Admit known failed/cancelled pages; never infer safe cleanup from a PID."""
    from scripts.ocr.mvp import local
    from scripts.ocr.mvp.core import digest
    from scripts.ocr.mvp.store import load_snapshot
    folder = Path(folder);op = folder / 'operation'
    check(compatibility(meta, config) == 'verified', 'SESSION_RUNTIME_UNVERIFIED_OR_CHANGED')
    check(recovery_successor(folder, meta, config) is None, 'RECOVERY_SUCCESSOR_EXISTS')
    check(not op.is_symlink() and not (op / 'worker-cleanup-failure.json').exists(), 'RECOVERY_CLEANUP_UNVERIFIED')
    job = safe_record(op / 'job.json');task = job.get('task');normalized = local.runtime_config(config['local'])
    check(type(task) is dict and job.get('schema_version') == 1 and job.get('config') == normalized and
          job.get('identity') == digest(dict(task=task, config=normalized)), 'RECOVERY_JOB_CHANGED')
    reused, prior_review = local._reused_page_state(op, task, normalized, job)
    check(task.get('operation_id') == folder.name and task.get('mode') == 'local' and
          task.get('config_version') == local.CONFIG_VERSION and
          Path(task.get('output_directory', '')).resolve() == op.resolve() and
          task.get('input_sha256') == meta['source_sha256'] and Path(task.get('input_pdf', '')).resolve() == Path(meta['source']).resolve() and
          hashlib.sha256(Path(meta['source']).read_bytes()).hexdigest() == meta['source_sha256'], 'RECOVERY_SOURCE_MISMATCH')
    pages = task.get('page_numbers')
    check(type(pages) is list and pages == meta.get('document_binding', {}).get('physical_pages'), 'RECOVERY_PAGES_CHANGED')
    snapshot = None;revision = None
    if (op / 'CURRENT.json').exists() or (op / 'CURRENT.json').is_symlink():
        check(not (op / 'CURRENT.json').is_symlink(), 'RECOVERY_CURRENT_UNSAFE')
        snapshot, revision = load_snapshot(op)
        check(snapshot['input_sha256'] == meta['source_sha256'] and snapshot.get('operation_id') == folder.name and
              [p['page'] for p in snapshot['pages']] == pages, 'RECOVERY_DRAFT_CHANGED')
        local.load_task_completion(op, snapshot, revision)
    current = {p['page']: p for p in snapshot['pages']} if snapshot else {}
    complete = {};retry = [];failures = [];saved = {}
    for number in pages:
        p = local.saved_page_result(op, number, task, normalized)
        if p is not None:saved[number] = p
        if p is None:
            check(not any((op / 'raw' / f'worker-{number}{suffix}').exists() or (op / 'raw' / f'worker-{number}{suffix}').is_symlink()
                          for suffix in ('.json', '.stdout', '.stderr')), 'RECOVERY_PARTIAL_WORKER_UNVERIFIED')
            if snapshot:
                check(current[number].get('route') == 'failed' and not current[number].get('words'), 'RECOVERY_PAGE_RESULT_MISSING')
            retry.append(number)
        elif p['route'] == 'failed':
            failures.append(number);retry.append(number)
        else:
            check(not (op / 'raw' / f'page-{number:04d}' / 'parent-result.json').exists(), 'RECOVERY_PARENT_SUCCESS_UNEXPECTED')
            selected = copy.deepcopy(current.get(number, p))
            check(selected.get('raw_files') == p['raw_files'] and selected.get('image_path') == p['image_path'] and
                  selected.get('image_sha256') == p['image_sha256'] and selected.get('route') == p['route'], 'RECOVERY_REVIEW_SOURCE_CHANGED')
            if selected.get('status') == 'EXPORT_REVIEW':
                selected['status'] = selected.pop('pre_export_status', 'OCR_DRAFT')
            complete[number] = selected
    local._validate_saved_checkpoint(op, pages, saved, reused)
    cancelled = any((op / name).exists() or (op / name).is_symlink() for name in ('CANCEL', 'cancel'))
    check(bool(retry) and (cancelled or bool(failures) or bool(snapshot)), 'RECOVERY_NOT_NEEDED')
    inherited = dict(source_session_id=folder.name, source_revision=snapshot['revision'] if snapshot else None,
                     source_snapshot_sha256=hashlib.sha256((revision / 'snapshot.json').read_bytes()).hexdigest() if revision else None,
                     receipts=copy.deepcopy(snapshot.get('receipts', [])) if snapshot else [],
                     note='Original review history and edits retained; not new human approval or a successful old task')
    if prior_review is not None:
        inherited['previous'] = copy.deepcopy(prior_review)
    return dict(task=task, config=normalized, completed=complete, retry_pages=retry,
                inherited_review=inherited, original_job_sha256=hashlib.sha256((op / 'job.json').read_bytes()).hexdigest())


@contextmanager
def processing_recovery(folder, meta, request, config):
    """Hold the original producer until the explicit successor attempt returns."""
    from scripts.ocr.mvp import local
    from scripts.ocr.mvp.core import digest
    from .save_recovery import publish_json
    folder = Path(folder);old_op = folder / 'operation'
    with local.task_producer(old_op):
        with session_control(folder):
            check(safe_record(folder / 'desktop-session.json') == meta, 'SESSION_METADATA_CHANGED')
            plan = processing_recovery_plan(folder, meta, config)
            state = recovery_state(folder)
            check(state == request.get('continuation_state_sha256'), 'RECOVERY_STATE_CHANGED')
            check(request['client_operation_id'] not in (meta.get('client_operation_id'), current_attempt(folder, meta)), 'RECOVERY_TOKEN_REUSED')
            child = roots(config, create=True)[0] / uuid.uuid4().hex
            child.mkdir(mode=0o700)
            task = dict(plan['task'], operation_id=child.name, output_directory=str(child / 'operation'))
            updated = copy.deepcopy(meta)
            updated.update(client_operation_id=request['client_operation_id'], provenance='explicit-local-processing-recovery',
                           processing_recovery=dict(previous_session_id=folder.name, previous_state_sha256=state,
                                                    previous_job_sha256=plan['original_job_sha256'], retry_pages=plan['retry_pages']))
            updated['document_binding']['document_id'] = request.get('document_id')
            local.write(child / 'desktop-session.json', updated)
            local.write(child / 'task-request.json', request)
            op = child / 'operation';op.mkdir()
            results = {}
            for number, page in plan['completed'].items():
                old_directory = old_op / 'raw' / f'page-{number:04d}'
                new_directory = op / 'raw' / old_directory.name
                new_directory.mkdir(parents=True)
                for name, expected in page['raw_files'].items():
                    source = old_op / name;target = op / name
                    check(source.is_relative_to(old_directory) and not source.is_symlink(), 'RECOVERY_RAW_SCOPE')
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source, target)
                    check(hashlib.sha256(target.read_bytes()).hexdigest() == expected, 'RECOVERY_RAW_COPY_CHANGED')
                page = copy.deepcopy(page)
                page['image_path'] = str(op / Path(page['image_path']).relative_to(old_op))
                page['reused_result'] = dict(previous_session_id=folder.name, previous_state_sha256=state,
                                             original_page_result_sha256=hashlib.sha256((old_directory / 'page-result.json').read_bytes()).hexdigest(),
                                             note='Derivative reuse record; copied raw bytes unchanged; no new page worker')
                local.write(new_directory / 'page-result.json', page)
                results[str(number)] = digest(page)
            reused = dict(schema='local-completed-page-reuse/1', task_identity=digest(dict(task=task, config=plan['config'])),
                          source_sha256=meta['source_sha256'], page_results=results, inherited_review=plan['inherited_review'])
            local.write(op / 'reused-pages.json', reused)
            reuse_sha = hashlib.sha256((op / 'reused-pages.json').read_bytes()).hexdigest()
            local.write(op / 'job.json', dict(schema_version=1, task=task, config=plan['config'],
                        identity=reused['task_identity'], reuse_manifest_sha256=reuse_sha))
            local.write(op / 'progress.json', dict(schema_version=1, completed=0, total=len(task['page_numbers']), page_results=[]))
            local._reused_page_state(op, task, plan['config'], safe_record(op / 'job.json'))
            check(recovery_state(folder) == state and compatibility(meta, config) == 'verified', 'RECOVERY_STATE_CHANGED')
            publish_json(folder / 'PROCESSING_SUCCESSOR.json', dict(schema='local-processing-successor/1',
                         previous_session_id=folder.name, session_id=child.name, source_sha256=meta['source_sha256'],
                         metadata_sha256=hashlib.sha256((child / 'desktop-session.json').read_bytes()).hexdigest(),
                         job_sha256=hashlib.sha256((op / 'job.json').read_bytes()).hexdigest(),
                         observed_state_sha256=state, client_operation_id=request['client_operation_id']))
        yield child, updated, task
