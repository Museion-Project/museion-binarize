"""Explicit shared private state; never write a ledger into signed resources."""
import os
from pathlib import Path
from .pipeline import check, locked, write


def legacy_directory():
    return Path(__file__).resolve().parent / '.request-ledger'


def directory(root):
    check(root is not None, 'explicit shared persistent --state-root required')
    path = Path(root).expanduser()
    check(path.is_absolute(), 'state root must be absolute')
    check(not any(p.is_symlink() for p in (path, *path.parents)), 'state root symlink prohibited')
    path = path.resolve()
    repo = Path(__file__).resolve().parents[3]
    check(path != Path('/') and not path.is_relative_to(repo), 'state root cannot be a source/resource directory')
    check(not any(p.name.endswith('.app') for p in (path, *path.parents)), 'signed App resource state prohibited')
    old = legacy_directory()
    check(not old.exists() or (old.is_dir() and not any(old.iterdir())),
          'LEGACY_LEDGER_RELOCATION_REQUIRED: preserve and reconcile all rows/raw/UNKNOWN holds first')
    if path.exists():
        check(path.is_dir() and path.stat().st_uid == os.getuid() and path.stat().st_mode & 0o077 == 0,
              'state root must be a private owner-only directory')
    return path


def halt(root, call, receipt, reason):
    """Known HTTP/usage does not release a semantic identity failure for re-send."""
    db = locked(root)
    try:
        db.execute('BEGIN IMMEDIATE')
        db.execute('UPDATE calls SET state=? WHERE id=?', ('identity_failed', call['request_id']))
        db.commit()
        write(Path(root) / 'raw' / call['request_id'] / 'dispatch-halt.json',
              dict(reason=reason, request_id=call['request_id'],
                   response_sha256=receipt['response_sha256'], raw_preserved=True), True)
    finally:
        db.close()


def accept(root, call):
    """Release received state only after base field/identity validation."""
    db = locked(root)
    try:
        db.execute('BEGIN IMMEDIATE')
        row = db.execute('SELECT state,cost FROM calls WHERE id=?', (call['request_id'],)).fetchone()
        check(row is not None and row[0] in ('received', 'complete') and row[1] is not None,
              'semantic validation cannot clear an earlier failure/unknown hold')
        db.execute('UPDATE calls SET state=? WHERE id=?', ('complete', call['request_id']))
        db.commit()
    finally:
        db.close()
