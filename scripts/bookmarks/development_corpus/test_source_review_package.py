"""Synthetic guards for source packaging; no research images, readers or OCR."""
import copy
from html.parser import HTMLParser
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from scripts.bookmarks.development_corpus.source_review_package import (
    create, digest, ReviewError, SUPPORTED, validate, verify,
)


class Links(HTMLParser):
    def __init__(self):
        super().__init__(); self.paths = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key in ('src', 'href'):
                self.paths.append(value)


class SourceReviewPackageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.corpus = self.root / 'source'
        self.book = 'synthetic-book'
        self.ledger_name = 'development/' + self.book + '/reference-ledger.json'
        self.ledger_path = self.corpus / self.ledger_name
        self.ledger_path.parent.mkdir(parents=True)
        self.source_sha = 'a' * 64
        self.pages = []
        for n in (1, 2):
            name = f'development/{self.book}/pages/pdfpage-{n:04d}.png'
            path = self.corpus / name; path.parent.mkdir(exist_ok=True)
            path.write_bytes(f'synthetic saved image {n}'.encode())
            self.pages.append(dict(source_page_id=f'{self.book}:p{n}', pdf_page_1based=n,
                                   image=name, source_sha256=self.source_sha, image_sha256=digest(path)))
        self.entries = []
        for n in range(1, 5):
            self.entries.append(dict(unit_id=f'e{n}', title_literal=f'Title {n} <&>',
                printed_folio_literal='I' if n == 1 else str(n), parent_unit_id=None,
                human_checked=False, member_word_ids=[f'w{n}'], member_roles={f'w{n}': 'title'},
                bbox_by_page_pdf_points={'2': [1, 2, 3, 4]}, pdf_pages_1based=[1 if n == 1 else 2],
                uncertainty=[dict(field='unnumbered_hierarchy', status='UNVERIFIED')]
                    if n in (2, 3) else []))
        self.ledger = dict(schema='toc-reference-ledger-v2/1', book_id=self.book,
            bibliographic_title='Synthetic <&> Book', split='development', source_sha256=self.source_sha,
            human_checked=False, automatic_admission=False, formal_confirmation_ready=False,
            pages=self.pages, entries=self.entries)
        self.save_ledger()
        self.review_path = self.root / 'additional-review.json'
        rows = []
        for n in (2, 3):
            rows.append(dict(book_id=self.book, unit_id=f'e{n}', entry_ordinal=n,
                source_sha256=self.source_sha, original_ledger_path=str(self.ledger_path),
                original_ledger_sha256=digest(self.ledger_path), original_entry=copy.deepcopy(self.entries[n-1]),
                status=SUPPORTED if n == 2 else 'UNKNOWN', proposed_parent_unit_id='e1' if n == 2 else None,
                proposed_parent_title=self.entries[0]['title_literal'] if n == 2 else None,
                original_parent_title=None, root_interpretation=False,
                local_outline_depth_hint=1 if n == 2 else None,
                global_hierarchy_level_verified=False,
                candidate_parents=[] if n == 2 else [dict(ordinal=i, unit_id=f'e{i}',
                    title=self.entries[i-1]['title_literal']) for i in (1, 2)],
                candidate_local_depth_hints=[] if n == 2 else [1, 2], source_images=self.pages,
                source_cue='Synthetic relationship; no source quality claim',
                parent_differs_from_frozen_reference=n == 2, human_checked=False,
                independent_reference=False, original_UNVERIFIED_retained=True,
                automatic_admission=False, formal_scoring_ready=False, input_to_runtime=False))
        context = dict(book_id=self.book, original_entry=copy.deepcopy(self.entries[3]),
            candidate_parent_unit_id='e1', candidate_parent_title=self.entries[0]['title_literal'],
            status='ADDITIONAL_AI_CONTEXT_PROPOSAL', source_cue='Unapplied synthetic context',
            source_images=self.pages, applied_to_frozen_reference=False,
            applied_to_parent_validation_graph=False, human_checked=False,
            independent_reference=False, automatic_admission=False)
        self.review = dict(schema='directory-source-hierarchy-additional-review/1',
            source_corpus=str(self.corpus), corpus_freeze_sha256=self.freeze_sha,
            reviewed_entries=rows, preserved_nonpending_entries=[dict(book_id=self.book,
                original_entry=copy.deepcopy(self.entries[i])) for i in (0, 3)],
            additional_context_proposals=[context], source_page_reviews=[dict(book_id=self.book,
                **p, review_status='COORDINATOR_AI_OPENED_COMPLETE_SAVED_PAGE', human_checked=False,
                machine_attestation_of_model_visual_reasoning=False) for p in self.pages],
            status_counts={SUPPORTED: 1, 'UNKNOWN': 1}, parent_interpretations_differing_from_v3=1,
            original_v3_hierarchy_UNVERIFIED_count=2, manual_AI_review_only=True,
            human_gold=False, independent_confirmation=False, original_v3_changed=False,
            global_hierarchy_levels_verified=False, strict_quality_scoring_ready=False,
            natural_observer_run=False, predictions=0, quality_evaluations=0,
            quality_ready=False, app_admission=False, distribution_ready=False, release_ready=False)
        self.save_review()

    def save_ledger(self):
        self.ledger_path.write_text(json.dumps(self.ledger))
        files = {p.relative_to(self.corpus).as_posix(): digest(p)
                 for p in self.corpus.rglob('*') if p.is_file() and p.name != 'freeze_manifest.json'}
        self.freeze_path = self.corpus / 'freeze_manifest.json'
        self.freeze_path.write_text(json.dumps(dict(schema='toc-dataset-freeze-v2/1', files=files)))
        self.freeze_sha = digest(self.freeze_path)

    def save_review(self):
        self.review_path.write_text(json.dumps(self.review))
        self.review_sha = digest(self.review_path)

    def validate(self):
        return validate(self.corpus, self.freeze_sha, self.review_path, self.review_sha)

    def make(self, output=None):
        return create(self.corpus, self.freeze_sha, self.review_path, self.review_sha,
                      output or self.root / 'package')

    def test_portable_package_preserves_full_originals_and_ambiguity(self):
        before = self.ledger_path.read_bytes()
        receipt = self.make()
        self.assertEqual(receipt['counts']['original_entries'], 4)
        self.assertEqual(receipt['counts']['original_UNVERIFIED'], 2)
        self.assertEqual(receipt['counts']['additional_UNKNOWN'], 1)
        package = self.root / 'package'
        self.assertEqual((package / self.ledger_name).read_bytes(), before)
        self.assertEqual(self.ledger_path.read_bytes(), before)
        self.assertEqual((package / 'inputs/hierarchy-source-review.json').read_bytes(), self.review_path.read_bytes())
        proposals = json.loads((package / f'development/{self.book}/hierarchy-proposals.json').read_text())
        self.assertFalse(proposals['input_to_runtime'])
        self.assertFalse(proposals['unapplied_context_proposals'][0]['applied_to_parent_validation_graph'])
        self.assertIsNone(proposals['reviewed_entries'][1]['proposed_parent_unit_id'])
        self.assertFalse(proposals['reviewed_entries'][1]['root_interpretation'])
        text = (package / 'review.html').read_text()
        self.assertIn('Title 2 &lt;&amp;&gt;', text)
        self.assertIn('未定：', text)
        links = Links(); links.feed(text)
        self.assertTrue(links.paths)
        self.assertTrue(all(not Path(p).is_absolute() and (package / p).is_file() for p in links.paths))
        moved = self.root / 'moved'; shutil.move(str(package), moved)
        shutil.move(str(self.corpus), self.root / 'moved-original')
        result = verify(moved, receipt['manifest_sha256'])
        self.assertEqual(result['state'], 'SOURCE_REVIEW_PACKAGE_INTEGRITY_PASS')
        self.assertFalse(result['strict_quality_scoring_ready'])

    def test_stale_freeze_or_review_is_rejected_before_output(self):
        for kind in ('freeze', 'review'):
            with self.subTest(kind=kind):
                kwargs = dict(corpus=self.corpus, freeze_sha256=self.freeze_sha,
                              review_path=self.review_path, review_sha256=self.review_sha,
                              output=self.root / 'bad-output')
                kwargs['freeze_sha256' if kind == 'freeze' else 'review_sha256'] = '0' * 64
                with self.assertRaisesRegex(ReviewError, 'CHANGED'):
                    create(**kwargs)
                self.assertFalse(kwargs['output'].exists())

    def test_source_image_changes_rejected(self):
        image = self.corpus / self.pages[0]['image']; image.write_bytes(b'wrong page')
        with self.assertRaisesRegex(ReviewError, 'SOURCE_HASH_CHANGED'):
            self.make()
        self.assertFalse((self.root / 'package').exists())

    def test_original_literals_members_roles_boxes_not_replaceable(self):
        base = copy.deepcopy(self.review)
        for field, value in [('title_literal', 'new title'), ('member_word_ids', ['other']),
                             ('member_roles', {'w2': 'folio'}), ('bbox_by_page_pdf_points', {}),
                             ('printed_folio_literal', 'I')]:
            with self.subTest(field=field):
                self.review = copy.deepcopy(base)
                self.review['reviewed_entries'][0]['original_entry'][field] = value
                self.save_review()
                with self.assertRaisesRegex(ReviewError, 'ORIGINAL_ENTRY_CHANGED'):
                    self.validate()

    def test_missing_duplicate_pending_and_nonpending_entries_rejected(self):
        base = copy.deepcopy(self.review)
        for field in ('reviewed_entries', 'preserved_nonpending_entries'):
            for operation in ('missing', 'duplicate'):
                with self.subTest(field=field, operation=operation):
                    self.review = copy.deepcopy(base)
                    if operation == 'missing': self.review[field].pop()
                    else: self.review[field].append(copy.deepcopy(self.review[field][0]))
                    self.save_review()
                    with self.assertRaisesRegex(ReviewError, 'COVERAGE'):
                        self.validate()

    def test_promotion_to_human_scoring_runtime_or_applied_context_rejected(self):
        base = copy.deepcopy(self.review)
        targets = [(None, 'human_gold'), (None, 'strict_quality_scoring_ready'),
                   ('reviewed_entries', 'human_checked'), ('reviewed_entries', 'input_to_runtime'),
                   ('reviewed_entries', 'global_hierarchy_level_verified'),
                   ('additional_context_proposals', 'applied_to_parent_validation_graph')]
        for field, flag in targets:
            with self.subTest(flag=flag):
                self.review = copy.deepcopy(base)
                target = self.review if field is None else self.review[field][0]
                target[flag] = True; self.save_review()
                with self.assertRaisesRegex(ReviewError, 'REVIEW_PROMOTION'):
                    self.validate()

    def test_unknown_not_root_or_single_forced_candidate(self):
        base = copy.deepcopy(self.review)
        for field, value in [('root_interpretation', True), ('local_outline_depth_hint', 0),
                             ('candidate_parents', base['reviewed_entries'][1]['candidate_parents'][:1])]:
            with self.subTest(field=field):
                self.review = copy.deepcopy(base)
                self.review['reviewed_entries'][1][field] = value; self.save_review()
                with self.assertRaisesRegex(ReviewError, 'UNKNOWN_'):
                    self.validate()

    def test_parent_source_binding_and_graph_cycles_rejected(self):
        base = copy.deepcopy(self.review)
        for field, value in [('proposed_parent_unit_id', 'other-book-parent'),
                             ('proposed_parent_unit_id', 'e3'), ('proposed_parent_title', 'wrong'),
                             ('source_images', [self.pages[1]])]:
            with self.subTest(field=field, value=value):
                self.review = copy.deepcopy(base)
                self.review['reviewed_entries'][0][field] = value; self.save_review()
                with self.assertRaises(ReviewError): self.validate()
        self.review = copy.deepcopy(base)
        self.ledger['entries'][0]['parent_unit_id'] = 'e2'; self.save_ledger()
        self.review['corpus_freeze_sha256'] = self.freeze_sha
        self.review['preserved_nonpending_entries'][0]['original_entry'] = copy.deepcopy(self.ledger['entries'][0])
        for row in self.review['reviewed_entries']: row['original_ledger_sha256'] = digest(self.ledger_path)
        self.save_review()
        with self.assertRaisesRegex(ReviewError, 'PARENT_GRAPH'): self.validate()

    def test_existing_output_and_source_overlap_preserve_sentinel(self):
        output = self.root / 'package'; output.mkdir(); sentinel = output / 'keep'; sentinel.write_text('keep')
        with self.assertRaisesRegex(ReviewError, 'OUTPUT_EXISTS'): self.make(output)
        self.assertEqual(sentinel.read_text(), 'keep')
        with self.assertRaisesRegex(ReviewError, 'OUTPUT_OVERLAP'): self.make(self.corpus / 'new')
        self.assertFalse((self.corpus / 'new').exists())

    def test_path_escape_alias_and_symlink_rejected(self):
        for name in ('../escape', 'development//alias'):
            with self.subTest(name=name):
                freeze = json.loads(self.freeze_path.read_text()); freeze['files'][name] = '0' * 64
                self.freeze_path.write_text(json.dumps(freeze)); self.freeze_sha = digest(self.freeze_path)
                with self.assertRaisesRegex(ReviewError, 'PATH_ESCAPE'): self.validate()
                self.save_ledger()
        alias = self.root / 'alias'; alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ReviewError, 'PATH_SYMLINK'): self.make(alias / 'new')
        image = self.corpus / self.pages[0]['image']; saved = self.root / 'saved'; image.rename(saved); image.symlink_to(saved)
        with self.assertRaisesRegex(ReviewError, 'PATH_SYMLINK'): self.validate()

    def test_package_changes_and_unlisted_files_rejected(self):
        receipt = self.make(); package = self.root / 'package'
        extra = package / 'unlisted'; extra.write_text('extra')
        with self.assertRaisesRegex(ReviewError, 'PACKAGE_FILE_CLOSURE'): verify(package, receipt['manifest_sha256'])
        extra.unlink()
        image = package / self.pages[0]['image']; image.write_bytes(b'changed')
        with self.assertRaisesRegex(ReviewError, 'SOURCE_HASH_CHANGED'): verify(package, receipt['manifest_sha256'])


if __name__ == '__main__':
    unittest.main()
