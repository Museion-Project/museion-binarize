"""Package a pinned development source review; never feed answers to an observer.

The original ledgers and saved page images are copied byte for byte. Proposed
parents are supplementary AI review material, not replacements for UNKNOWN or
UNVERIFIED references, independent confirmation, or scored/runtime input.
Only the Python standard library is used; no PDF, OCR, network or model calls.
"""
import argparse
from collections import Counter
import hashlib
import html
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SUPPORTED = 'AI_SOURCE_SUPPORTED_INTERPRETATION'
REVIEW_SCHEMA = 'directory-source-hierarchy-additional-review/1'
PACKAGE_SCHEMA = 'directory-source-review-package/1'
FALSE_FLAGS = ('human_gold', 'independent_confirmation', 'original_v3_changed',
               'global_hierarchy_levels_verified', 'strict_quality_scoring_ready',
               'natural_observer_run', 'quality_ready', 'app_admission',
               'distribution_ready', 'release_ready')
ROW_FALSE = ('human_checked', 'independent_reference', 'automatic_admission',
             'formal_scoring_ready', 'input_to_runtime',
             'global_hierarchy_level_verified')


class ReviewError(ValueError):
    pass


def require(ok, reason):
    if not ok:
        raise ReviewError(reason)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def canonical(path):
    path = Path(path).absolute()
    require('..' not in path.parts, 'PATH_ESCAPE')
    require(not any(p.is_symlink() for p in (path, *path.parents)), 'PATH_SYMLINK')
    return path


def relative(name):
    require(isinstance(name, str) and name, 'PATH_ESCAPE')
    p = Path(name)
    require(not p.is_absolute() and '..' not in p.parts and
            p.as_posix() == name and '\\' not in name, 'PATH_ESCAPE')
    return p


def checked_file(root, name, expected):
    path = canonical(root / relative(name))
    require(path.is_file(), 'SOURCE_FILE_MISSING: ' + name)
    require(digest(path) == expected, 'SOURCE_HASH_CHANGED: ' + name)
    return path


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def flags_false(value, names):
    require(all(value.get(n) is False for n in names), 'REVIEW_PROMOTION')


def validate(corpus, freeze_sha256, review_path, review_sha256):
    """Validate identity and interpretation structure; not interpretation truth."""
    corpus, review_path = canonical(corpus), canonical(review_path)
    require(corpus.is_dir() and review_path.is_file(), 'INPUT_MISSING')
    freeze_path = corpus / 'freeze_manifest.json'
    require(not freeze_path.is_symlink() and digest(freeze_path) == freeze_sha256,
            'CORPUS_FREEZE_CHANGED')
    freeze = read(freeze_path)
    require(freeze.get('schema') == 'toc-dataset-freeze-v2/1', 'CORPUS_SCHEMA')
    for name, expected in freeze['files'].items():
        checked_file(corpus, name, expected)
    require(digest(review_path) == review_sha256, 'REVIEW_HASH_CHANGED')
    review = read(review_path)
    require(review.get('schema') == REVIEW_SCHEMA, 'REVIEW_SCHEMA')
    require(review['corpus_freeze_sha256'] == freeze_sha256 and
            canonical(review['source_corpus']) == corpus, 'REVIEW_CORPUS_BINDING')
    flags_false(review, FALSE_FLAGS)
    require(review.get('manual_AI_review_only') is True and
            type(review.get('predictions')) is int and review['predictions'] == 0 and
            type(review.get('quality_evaluations')) is int and
            review['quality_evaluations'] == 0, 'REVIEW_PROMOTION')
    books, entries, pending, pages = {}, {}, [], {}
    for name in sorted(freeze['files']):
        rel = relative(name)
        if len(rel.parts) != 3 or rel.parts[0] != 'development' or rel.name != 'reference-ledger.json':
            continue
        ledger = read(corpus / rel)
        bid = rel.parts[1]
        require(ledger.get('schema') == 'toc-reference-ledger-v2/1' and
                ledger['book_id'] == bid and ledger['split'] == 'development', 'LEDGER_SCOPE')
        flags_false(ledger, ('human_checked', 'automatic_admission', 'formal_confirmation_ready'))
        require(bid not in books, 'DUPLICATE_BOOK')
        books[bid] = (name, ledger)
        for ordinal, entry in enumerate(ledger['entries'], 1):
            key = (bid, entry['unit_id'])
            require(key not in entries and entry.get('human_checked') is False, 'ENTRY_IDENTITY')
            entries[key] = (entry, ordinal)
            uncertain = [u for u in entry['uncertainty'] if u['field'] == 'unnumbered_hierarchy']
            if uncertain:
                require(all(u['status'] == 'UNVERIFIED' for u in uncertain), 'REVIEW_PROMOTION')
                pending.append(key)
        for page in ledger['pages']:
            key = (bid, page['pdf_page_1based'])
            image = relative(page['image'])
            require(key not in pages and image.parts[:3] == ('development', bid, 'pages') and
                    page['source_sha256'] == ledger['source_sha256'] and
                    freeze['files'].get(page['image']) == page['image_sha256'], 'PAGE_BINDING')
            pages[key] = page
    require(books and entries, 'NO_DEVELOPMENT_ENTRIES')
    reviewed, preserved = review['reviewed_entries'], review['preserved_nonpending_entries']
    require(Counter((r['book_id'], r['unit_id']) for r in reviewed) == Counter(pending),
            'PENDING_COVERAGE')
    remainder = [key for key in entries if key not in set(pending)]
    require(Counter((r['book_id'], r['original_entry']['unit_id']) for r in preserved) ==
            Counter(remainder), 'NONPENDING_COVERAGE')

    def original(row):
        key = (row['book_id'], row['original_entry']['unit_id'])
        require(key in entries and row['original_entry'] == entries[key][0], 'ORIGINAL_ENTRY_CHANGED')
        return entries[key]

    def parent(bid, pid, title, before, ordinal=None):
        if pid is None:
            require(title is None and ordinal is None, 'PARENT_BINDING')
            return None
        value = entries.get((bid, pid))
        require(value is not None and value[1] < before and
                value[0]['title_literal'] == title and
                (ordinal is None or ordinal == value[1]), 'PARENT_BINDING')
        return value[0]

    def images(row, required_entries):
        bid = row['book_id']
        bound = row['source_images']
        require(len(bound) == len({p['pdf_page_1based'] for p in bound}), 'PAGE_BINDING')
        require(all(p == pages.get((bid, p['pdf_page_1based'])) for p in bound), 'PAGE_BINDING')
        required = {n for e in required_entries if e for n in e['pdf_pages_1based']}
        require(required <= {p['pdf_page_1based'] for p in bound} and
                isinstance(row.get('source_cue'), str) and row['source_cue'].strip(), 'SOURCE_EVIDENCE_MISSING')

    for row in preserved:
        original(row)
    graphs = {bid: {e['unit_id']: e['parent_unit_id'] for e in ledger['entries']}
              for bid, (_, ledger) in books.items()}
    for row in reviewed:
        entry, ordinal = original(row)
        bid = row['book_id']; name, ledger = books[bid]
        require(row['unit_id'] == entry['unit_id'] and row['entry_ordinal'] == ordinal and
                row['source_sha256'] == ledger['source_sha256'] and
                row['original_ledger_sha256'] == freeze['files'][name] and
                canonical(ROOT / row['original_ledger_path']) == corpus / name, 'LEDGER_BINDING')
        flags_false(row, ROW_FALSE)
        require(row.get('original_UNVERIFIED_retained') is True, 'REVIEW_PROMOTION')
        old = entries.get((bid, entry['parent_unit_id']))
        require(row['original_parent_title'] == (old[0]['title_literal'] if old else None), 'PARENT_BINDING')
        require(row['status'] in (SUPPORTED, 'UNKNOWN'), 'REVIEW_STATUS')
        if row['status'] == 'UNKNOWN':
            require(row['proposed_parent_unit_id'] is None and row['proposed_parent_title'] is None and
                    row['root_interpretation'] is False and row['local_outline_depth_hint'] is None and
                    row['parent_differs_from_frozen_reference'] is False, 'UNKNOWN_PROMOTION')
            choices = row['candidate_parents']
            require(len(choices) >= 2 and len({c['unit_id'] for c in choices}) == len(choices),
                    'UNKNOWN_CANDIDATES')
            possible = [parent(bid, c['unit_id'], c['title'], ordinal, c['ordinal']) for c in choices]
            require(len(set(row['candidate_local_depth_hints'])) >= 2 and
                    all(type(n) is int and n >= 0 for n in row['candidate_local_depth_hints']),
                    'UNKNOWN_CANDIDATES')
            images(row, [entry, *possible])
        else:
            pid = row['proposed_parent_unit_id']
            proposed = parent(bid, pid, row['proposed_parent_title'], ordinal)
            require(row['root_interpretation'] is (pid is None) and
                    type(row['local_outline_depth_hint']) is int and row['local_outline_depth_hint'] >= 0 and
                    row['parent_differs_from_frozen_reference'] is (pid != entry['parent_unit_id']),
                    'PARENT_BINDING')
            images(row, [entry, proposed])
            graphs[bid][entry['unit_id']] = pid
    # Context links are carried for review, never applied to either parent graph.
    contexts = review['additional_context_proposals']
    context_keys = []
    for row in contexts:
        entry, ordinal = original(row); bid = row['book_id']
        key = (bid, entry['unit_id']); context_keys.append(key)
        require(key in remainder and row['status'] == 'ADDITIONAL_AI_CONTEXT_PROPOSAL', 'CONTEXT_SCOPE')
        flags_false(row, ('human_checked', 'independent_reference', 'automatic_admission',
                          'applied_to_frozen_reference', 'applied_to_parent_validation_graph'))
        proposed = parent(bid, row['candidate_parent_unit_id'], row['candidate_parent_title'], ordinal)
        images(row, [entry, proposed])
    require(len(context_keys) == len(set(context_keys)), 'CONTEXT_COVERAGE')
    for graph in graphs.values():
        for node in graph:
            visited = set()
            while node is not None:
                require(node in graph and node not in visited, 'PARENT_GRAPH')
                visited.add(node); node = graph[node]
    page_reviews = review['source_page_reviews']
    require(Counter((p['book_id'], p['pdf_page_1based']) for p in page_reviews) == Counter(pages.keys()),
            'PAGE_REVIEW_COVERAGE')
    for p in page_reviews:
        source = pages[p['book_id'], p['pdf_page_1based']]
        require(all(p.get(k) == v for k, v in source.items()), 'PAGE_BINDING')
        flags_false(p, ('human_checked', 'machine_attestation_of_model_visual_reasoning'))
        require(p['review_status'] == 'COORDINATOR_AI_OPENED_COMPLETE_SAVED_PAGE', 'PAGE_REVIEW_STATUS')
    counts = Counter(r['status'] for r in reviewed)
    differences = sum(r['parent_differs_from_frozen_reference'] for r in reviewed)
    require(dict(counts) == review['status_counts'] and
            differences == review['parent_interpretations_differing_from_v3'] and
            len(pending) == review['original_v3_hierarchy_UNVERIFIED_count'], 'REVIEW_COUNTS')
    summary = dict(books=len(books), source_pages=len(pages), original_entries=len(entries),
                   original_UNVERIFIED=len(pending), preserved_nonpending=len(remainder),
                   AI_supported_parent_proposals=counts[SUPPORTED], additional_UNKNOWN=counts['UNKNOWN'],
                   parent_difference_proposals=differences, unapplied_context_proposals=len(contexts))
    return books, pages, review, summary


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write('\n')


def render(books, review, summary):
    esc = html.escape
    chunks = ['<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>目录来源审阅资料</title>',
              '<style>body{font:16px/1.6 system-ui;margin:28px;color:#23303c}summary{cursor:pointer;font-size:20px}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccd5df;padding:8px;vertical-align:top}.unknown{background:#fdeaea}.changed{background:#fff4db}.sources{display:flex;flex-wrap:wrap;gap:12px}img{max-width:220px;height:auto}a{color:#245f91}</style>',
              '<h1>目录来源审阅资料</h1><p>原稿与原图完整保留，AI归组建议另列。全部原层级未认证状态保留，完整层级和独立确认仍待完成；这些资料尚不能用于质量评分或自动生成书签。</p>',
              '<p>'+esc(f"{summary['books']} 本书 / {summary['source_pages']} 张来源图 / {summary['original_entries']} 条原记录；{summary['AI_supported_parent_proposals']} 条AI归组建议，另有 {summary['additional_UNKNOWN']} 条仍有歧义。原 {summary['original_UNVERIFIED']} 条未认证状态保持。")+'</p>']
    for bid, (name, ledger) in books.items():
        chunks.append('<details><summary>'+esc(ledger.get('bibliographic_title', bid))+'</summary><p><a href="'+esc(name, quote=True)+'">原始完整记录</a> · <a href="development/'+esc(bid, quote=True)+'/hierarchy-proposals.json">附加建议与未决项</a></p><div class="sources">')
        for page in ledger['pages']:
            href = esc(page['image'], quote=True)
            chunks.append('<figure><a href="'+href+'"><img loading="lazy" src="'+href+'" alt="目录来源页"></a><figcaption>PDF物理页 '+str(page['pdf_page_1based'])+'</figcaption></figure>')
        chunks.append('</div><table><tr><th>条目</th><th>原归组</th><th>AI建议 / 未决</th><th>来源说明</th></tr>')
        for row in review['reviewed_entries']:
            if row['book_id'] != bid:
                continue
            unknown = row['status'] == 'UNKNOWN'
            css = 'unknown' if unknown else 'changed' if row['parent_differs_from_frozen_reference'] else ''
            proposed = ('未定：'+' / '.join(c['title'] or '顶层独立条目' for c in row['candidate_parents'])
                        if unknown else row['proposed_parent_title'] or '顶层独立条目')
            chunks.append('<tr class="'+css+'">'+''.join('<td>'+esc(value)+'</td>' for value in
                          (row['original_entry']['title_literal'], row['original_parent_title'] or '顶层独立条目',
                           proposed, row['source_cue']))+'</tr>')
        chunks.append('</table></details>')
    return '\n'.join(chunks) + '\n</html>\n'


def create(corpus, freeze_sha256, review_path, review_sha256, output):
    corpus, review_path, output = canonical(corpus), canonical(review_path), canonical(output)
    require(not output.exists(), 'OUTPUT_EXISTS')
    require(not any(output == p or output in p.parents or p in output.parents
                    for p in (corpus, review_path)), 'OUTPUT_OVERLAP')
    books, pages, review, summary = validate(corpus, freeze_sha256, review_path, review_sha256)
    # All inputs are validated before creating an exclusively owned new output.
    output.mkdir(parents=True, exist_ok=False)
    copies = {name for name, _ in books.values()} | {p['image'] for p in pages.values()}
    for name in sorted(copies):
        target = output / name; target.parent.mkdir(parents=True, exist_ok=True)
        with target.open('xb') as stream:
            stream.write((corpus / name).read_bytes())
    target = output / 'inputs/hierarchy-source-review.json'; target.parent.mkdir()
    with target.open('xb') as stream:
        stream.write(review_path.read_bytes())
    for bid, (name, _) in books.items():
        write_json(output / 'development' / bid / 'hierarchy-proposals.json', dict(
            schema='directory-book-hierarchy-proposals/1', book_id=bid,
            original_ledger=name, original_ledger_sha256=digest(corpus / name),
            reviewed_entries=[r for r in review['reviewed_entries'] if r['book_id'] == bid],
            unapplied_context_proposals=[r for r in review['additional_context_proposals'] if r['book_id'] == bid],
            original_reference_changed=False, human_checked=False, strict_quality_scoring_ready=False,
            input_to_runtime=False, global_hierarchy_levels_verified=False))
    with (output / 'review.html').open('x', encoding='utf-8') as stream:
        stream.write(render(books, review, summary))
    files = {p.relative_to(output).as_posix(): digest(p) for p in sorted(output.rglob('*')) if p.is_file()}
    manifest = dict(schema=PACKAGE_SCHEMA, status='AI_SOURCE_ASSISTED_REVIEW_DRAFT',
                    original_corpus_freeze_sha256=freeze_sha256, original_review_sha256=review_sha256,
                    counts=summary, files=files, original_reference_changed=False,
                    input_to_runtime=False, human_gold=False, independent_confirmation=False,
                    strict_quality_scoring_ready=False, global_hierarchy_levels_verified=False,
                    quality_ready=False, app_admission=False, distribution_ready=False, release_ready=False,
                    predictions=0, quality_evaluations=0, provider_calls=0,
                    evidence_scope='Pinned source identity and mechanical parent graph only; AI interpretations remain unconfirmed')
    write_json(output / 'package-manifest.json', manifest)
    # Catch races/partial copies before reporting successful packaging.
    validate(corpus, freeze_sha256, review_path, review_sha256)
    for name in copies:
        require(digest(output / name) == digest(corpus / name), 'COPY_CHANGED')
    require(digest(target) == review_sha256, 'COPY_CHANGED')
    return verify(output, digest(output / 'package-manifest.json'))


def verify(package, manifest_sha256):
    package = canonical(package); manifest_path = package / 'package-manifest.json'
    require(not manifest_path.is_symlink() and digest(manifest_path) == manifest_sha256, 'PACKAGE_MANIFEST_CHANGED')
    manifest = read(manifest_path)
    require(manifest.get('schema') == PACKAGE_SCHEMA and
            manifest.get('status') == 'AI_SOURCE_ASSISTED_REVIEW_DRAFT', 'PACKAGE_SCHEMA')
    flags_false(manifest, ('original_reference_changed', 'input_to_runtime', 'human_gold',
                          'independent_confirmation', 'strict_quality_scoring_ready',
                          'global_hierarchy_levels_verified', 'quality_ready', 'app_admission',
                          'distribution_ready', 'release_ready'))
    actual = []
    for path in package.rglob('*'):
        require(not path.is_symlink(), 'PATH_SYMLINK')
        if path.is_file():
            actual.append(path.relative_to(package).as_posix())
    require(set(actual) == set(manifest['files']) | {'package-manifest.json'}, 'PACKAGE_FILE_CLOSURE')
    for name, expected in manifest['files'].items():
        checked_file(package, name, expected)
    return dict(state='SOURCE_REVIEW_PACKAGE_INTEGRITY_PASS', package=str(package),
                manifest_sha256=manifest_sha256, files=len(manifest['files']), counts=manifest['counts'],
                evidence_scope=manifest['evidence_scope'], human_gold=False,
                strict_quality_scoring_ready=False, input_to_runtime=False,
                quality_ready=False, app_admission=False, distribution_ready=False, release_ready=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest='command', required=True)
    add = subs.add_parser('create')
    for name in ('corpus', 'freeze-sha256', 'review', 'review-sha256', 'output'):
        add.add_argument('--' + name, required=True)
    check = subs.add_parser('verify')
    check.add_argument('--package', required=True)
    check.add_argument('--manifest-sha256', required=True)
    args = parser.parse_args()
    try:
        receipt = (create(args.corpus, args.freeze_sha256, args.review, args.review_sha256, args.output)
                   if args.command == 'create' else verify(args.package, args.manifest_sha256))
        print(json.dumps(receipt, ensure_ascii=False))
        return 0
    except (ReviewError, KeyError, TypeError, OSError, json.JSONDecodeError) as exc:
        print(json.dumps(dict(state='SOURCE_REVIEW_PACKAGE_REJECTED', reason=str(exc)), ensure_ascii=False))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
