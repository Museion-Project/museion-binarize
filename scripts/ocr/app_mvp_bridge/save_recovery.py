"""Inspect interrupted saves; explicit recovery creates a receipt, never a PDF."""
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def check(ok, reason):
    if not ok:
        raise ValueError(reason)


def inspect_one(path, folder, meta, current):
    result = dict(journal_id=path.name, journal_sha256=None, status='unresolved',
                  recoverable=False, output_pdf=None, reason=None)
    try:
        check(not path.is_symlink(), 'JOURNAL_SYMLINK')
        result['journal_sha256'] = sha(path)
        journal = json.loads(path.read_text())
        check(type(journal) is dict and type(journal.get('result')) is dict, 'JOURNAL_SCHEMA')
        saved = journal['result']
        result['output_pdf'] = saved.get('output_pdf')
        binding = saved.get('recovery_binding')
        check(journal.get('schema') == 'save-pending/2' and journal['state'] == 'PREPARED'
              and isinstance(binding, dict), 'LEGACY_RECOVERY_BINDING_MISSING')
        check(saved.get('source_pdf') == meta['source'] and saved.get('source_sha256') == meta['source_sha256']
              and saved.get('review_required') is True, 'RECOVERY_RECEIPT_SOURCE_MISMATCH')
        check(binding['session_id'] == folder.name and binding['source_pdf'] == meta['source']
              and binding['source_sha256'] == meta['source_sha256'] == sha(meta['source']),
              'RECOVERY_SOURCE_MISMATCH')
        check(current.get('runtime_compatible') is True
              and binding['runtime_binding'] == meta.get('runtime_binding'), 'RECOVERY_RUNTIME_MISMATCH')
        check(binding['revision'] == saved['revision'] == current['revision'], 'RECOVERY_STALE_REVISION')
        derived = Path(current['output_pdf'])
        check(str(derived) == binding['derived_pdf'] == journal['source_pdf']
              and sha(derived) == binding['derived_sha256'] == saved['sha256'], 'RECOVERY_DERIVED_CHANGED')
        check(type(binding.get('cancellation_state')) is dict
              and set(binding['cancellation_state']) == {'cancel', 'CANCEL'}, 'RECOVERY_CANCEL_BINDING')
        for name, value in binding['cancellation_state'].items():
            check(name in ('cancel', 'CANCEL'), 'RECOVERY_CANCEL_BINDING')
            cancel = folder / 'operation' / name
            check((sha(cancel) if cancel.is_file() else None) == value, 'RECOVERY_CANCEL_CHANGED')
        receipt = Path(journal['receipt_path'])
        check(receipt.parent == folder and re.fullmatch(r'save-[0-9a-f]{32}\.json', receipt.name)
              and not receipt.is_symlink(), 'RECOVERY_RECEIPT_SCOPE')
        dest = Path(saved['output_pdf'])
        check(dest.is_absolute() and not any(p.is_symlink() for p in (dest, *dest.parents))
              and dest != Path(meta['source']).resolve(), 'RECOVERY_OUTPUT_SCOPE')
        check(dest.is_file(), 'RECOVERY_OUTPUT_MISSING')
        stat = dest.stat()
        check(journal.get('target_identity') == dict(device=stat.st_dev, inode=stat.st_ino, size=stat.st_size)
              and sha(dest) == saved['sha256'], 'RECOVERY_OUTPUT_CHANGED_OR_REPLACED')
        if receipt.exists():
            check(json.loads(receipt.read_text()) == saved, 'RECOVERY_RECEIPT_CONFLICT')
        marker = folder / ('save-recovery-' + result['journal_sha256'] + '.json')
        if marker.exists():
            check(not marker.is_symlink() and json.loads(marker.read_text()) == dict(
                schema='save-recovery/1', journal_id=path.name, journal_sha256=result['journal_sha256'],
                receipt_path=str(receipt), receipt_sha256=sha(receipt), output_sha256=saved['sha256']),
                  'RECOVERY_RECORD_CONFLICT')
            result['status'] = 'recovered'
        else:
            result.update(status='receipt_present' if receipt.exists() else 'complete_copy_pending_receipt',
                          recoverable=True)
    except (ValueError, OSError, KeyError, TypeError) as error:
        result['reason'] = str(error)
    return result


def inspect(folder, meta, current):
    return [inspect_one(path, folder, meta, current) for path in sorted(folder.glob('save-pending-*.json'))]


def publish_json(path, value):
    """Complete owner-private receipt, linked exclusively on the same volume."""
    encoded = json.dumps(value, ensure_ascii=False, indent=2).encode()
    fd, name = tempfile.mkstemp(prefix='.museion-recovery-', dir=path.parent)
    staged = Path(name)
    try:
        with os.fdopen(fd, 'wb') as out:
            out.write(encoded)
            out.flush()
            os.fsync(out.fileno())
        try:
            os.link(staged, path)
        except FileExistsError:
            check(not path.is_symlink() and json.loads(path.read_text()) == value, 'RECOVERY_RECEIPT_CONFLICT')
        fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        staged.unlink(missing_ok=True)


def recover(folder, meta, current, request, validate):
    name = request.get('save_journal_id')
    check(type(name) is str and re.fullmatch(r'save-pending-[0-9a-f]{32}\.json', name), 'RECOVERY_JOURNAL_ID')
    path = folder / name
    before = inspect_one(path, folder, meta, current)
    check(before['journal_sha256'] == request.get('save_journal_sha256'), 'RECOVERY_JOURNAL_CHANGED')
    check(before['recoverable'] or before['status'] == 'recovered', before['reason'] or 'RECOVERY_UNRESOLVED')
    validate()
    # Repeat all guards after validation; no automatic recognition, PDF writing,
    # cleanup, or revision substitution can occur on this path.
    after = inspect_one(path, folder, meta, current)
    check(after == before, 'RECOVERY_CHANGED_DURING_VALIDATION')
    journal = json.loads(path.read_text())
    receipt = Path(journal['receipt_path'])
    publish_json(receipt, journal['result'])
    checked = inspect_one(path, folder, meta, current)
    check(checked['journal_sha256'] == before['journal_sha256']
          and (checked['recoverable'] or checked['status'] == 'recovered'),
          checked['reason'] or 'RECOVERY_CHANGED_BEFORE_RECORD')
    marker = folder / ('save-recovery-' + before['journal_sha256'] + '.json')
    publish_json(marker, dict(schema='save-recovery/1', journal_id=name,
                             journal_sha256=before['journal_sha256'], receipt_path=str(receipt),
                             receipt_sha256=sha(receipt), output_sha256=journal['result']['sha256']))
    # Preserve the immutable journal and any crash staging files as history.
    return journal['result']
