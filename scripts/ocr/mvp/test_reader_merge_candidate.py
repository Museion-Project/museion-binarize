"""Pixel/ownership counterexamples for the isolated merge-repair candidate."""
import copy
import tempfile
import unittest
from pathlib import Path

import cv2
import fitz
import numpy as np

from . import core, reader_merge_candidate as candidate, store


def word(member, text, box, engine='apple', line='row', confidence=95):
    return dict(id=member, text=text, bbox=box, engine=engine,
                line_id=line, confidence=confidence)


class ReaderMergeCandidateTests(unittest.TestCase):
    def run_candidate(self, apple, reader, pixels):
        before = copy.deepcopy((apple, reader))
        with tempfile.TemporaryDirectory(prefix='merge-mechanism-test-') as folder:
            path = Path(folder) / 'source.png'
            self.assertTrue(cv2.imwrite(str(path), pixels))
            words, decisions = candidate.compose(apple, reader, [], path)
        self.assertEqual((apple, reader), before)
        core.ownership(words)
        return words, decisions

    def split_case(self):
        apple = [word('owner', 'readercontract', [10, 20, 160, 60])]
        reader = [word('left', 'reader', [12, 24, 75, 50], 'tesseract'),
                  word('right', 'contract', [88, 24, 158, 50], 'tesseract')]
        pixels = np.full((90, 190), 255, np.uint8)
        pixels[28:47, 15:73] = 0
        pixels[28:47, 90:156] = 0
        return apple, reader, pixels

    def greek_case(self):
        apple = [word('upper', 'Anchor', [10, 5, 50, 37], line='upper'),
                 word('owner', 'logos', [30, 30, 82, 65])]
        reader = [word('anchor', 'Anchor', [10, 5, 50, 34], 'tesseract', 'upper'),
                  word('target', 'λόγος', [32, 31, 80, 61], 'tesseract')]
        pixels = np.full((90, 110), 255, np.uint8)
        pixels[9:25, 12:48] = 0
        pixels[40:60, 40:79] = 0
        pixels[33:36, 35:38] = 0
        return apple, reader, pixels

    def test_split_preserves_complete_members_once_and_is_a_review_draft(self):
        a, r, pixels = self.split_case()
        before, _ = core.compose(a, r, [])
        self.assertEqual([w['id'] for w in before], ['owner'])
        words, ds = self.run_candidate(a, r, pixels)
        self.assertEqual([w['text'] for w in words], ['reader', 'contract'])
        owners = core.ownership(words)
        self.assertEqual(len(owners), 1)
        self.assertEqual(owners[0]['source_members'], ['owner'])
        self.assertEqual(owners[0]['reader_members'], ['left', 'right'])
        self.assertTrue(all(w['review'] for w in words))
        self.assertEqual(ds[-1]['state'], 'TEXT_SEGMENTATION_DRAFT')

    def test_observed_quote_is_retained_without_character_rewriting(self):
        a, r, pixels = self.split_case()
        r[1].update(text='“contract', confidence=35)
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual([w['text'] for w in words], ['reader', '“contract'])
        self.assertEqual(words[1]['bbox'], r[1]['bbox'])

    def test_letter_or_digit_disagreement_does_not_become_a_split(self):
        for text in ('readerc0ntract', 'reader7contract'):
            with self.subTest(text=text):
                a, r, pixels = self.split_case()
                a[0]['text'] = text
                words, _ = self.run_candidate(a, r, pixels)
                self.assertEqual([w['id'] for w in words], ['owner'])

    def test_existing_punctuation_cannot_be_erased(self):
        a, r, pixels = self.split_case()
        a[0]['text'] = 'reader,contract'
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual([w['text'] for w in words], ['reader,contract'])

    def test_no_pixel_space_does_not_certify_a_word_boundary(self):
        a, r, pixels = self.split_case()
        pixels[28:47, 73:91] = 0
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual([w['id'] for w in words], ['owner'])

    def test_different_reader_rows_are_not_joined(self):
        a, r, pixels = self.split_case()
        r[1]['line_id'] = 'another'
        before, _ = core.compose(a, r, [])
        words, decisions = self.run_candidate(a, r, pixels)
        self.assertEqual(words, before)
        self.assertFalse(any(d.get('state') == 'TEXT_SEGMENTATION_DRAFT' for d in decisions))

    def test_incomplete_group_or_low_confidence_stays_in_baseline(self):
        for mode in ('short', 'low'):
            with self.subTest(mode=mode):
                a, r, pixels = self.split_case()
                if mode == 'short':
                    r[1]['bbox'][2] = 120
                else:
                    r[1]['confidence'] = 29
                words, _ = self.run_candidate(a, r, pixels)
                self.assertEqual([w['id'] for w in words], ['owner'])

    def test_separate_complete_ink_components_preserve_the_neighbor(self):
        a, r, pixels = self.greek_case()
        before, _ = core.compose(a, r, [])
        self.assertEqual({w['id'] for w in before}, {'upper', 'owner'})
        words, ds = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'target'})
        self.assertEqual(next(w for w in words if w['id'] == 'upper'), before[0])
        decision = next(d for d in ds if d['state'] == 'GREEK_PADDING_DRAFT')
        self.assertTrue(decision['support_conflicts_resolved'][0]['exclusive_complete_ink_components'])

    def test_shared_small_component_remains_ambiguous(self):
        a, r, pixels = self.greek_case()
        pixels[33:36, 35:38] = 255
        pixels[32:33, 35:36] = 0
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'owner'})

    def test_touching_rows_do_not_pass_as_disjoint_ink(self):
        a, r, pixels = self.greek_case()
        pixels[23:42, 35:43] = 0
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'owner'})

    def test_duplicate_exact_anchors_do_not_resolve_an_ambiguous_owner(self):
        a, r, pixels = self.greek_case()
        r.append(dict(r[0], id='another-anchor'))
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'owner'})

    def test_stricter_greek_confidence_does_not_promote_weak_readings(self):
        a, r, pixels = self.greek_case()
        r[1]['confidence'] = 79
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'owner'})

    def test_fractional_pixel_contact_is_not_discarded_by_rounding(self):
        ink = np.zeros((10, 10), np.uint8)
        ink[4, 4] = 255
        self.assertEqual(candidate._ink_count(ink, [4.9, 4.9, 8, 8], [0, 0, 5, 5]), 1)

    def test_empty_image_does_not_validate_a_new_transfer(self):
        a, r, pixels = self.greek_case()
        pixels[:] = 255
        words, _ = self.run_candidate(a, r, pixels)
        self.assertEqual({w['id'] for w in words}, {'upper', 'owner'})

    def test_permuting_input_order_keeps_complete_split_ownership(self):
        a, r, pixels = self.split_case()
        left, _ = self.run_candidate(a, r, pixels)
        right, _ = self.run_candidate(a[::-1], r[::-1], pixels)
        self.assertEqual(left, right)
        self.assertEqual(core.ownership(left), core.ownership(right))

    def test_candidate_accepts_no_reference_or_target_id_arguments(self):
        with self.assertRaises(TypeError):
            candidate.compose([], [], [], 'unused', source_reference='answer')


class CandidateReviewPersistenceTests(unittest.TestCase):
    def check_persisted_review(self, case, member, expected_text):
        apple, reader, pixels = case
        original_arrays = copy.deepcopy((apple, reader))
        with tempfile.TemporaryDirectory(prefix='merge-review-contract-') as folder:
            root = Path(folder)
            image = root / 'source.png'
            self.assertTrue(cv2.imwrite(str(image), pixels))
            source = root / 'source.pdf'
            doc = fitz.open()
            doc.new_page(width=pixels.shape[1], height=pixels.shape[0]).insert_image(
                fitz.Rect(0, 0, pixels.shape[1], pixels.shape[0]), filename=str(image))
            doc.new_page().insert_text((20, 30), 'Untouched native control')
            doc.set_toc([[1, 'Preserved source bookmark', 2]])
            doc.save(source)
            doc.close()
            words, decisions = candidate.compose(apple, reader, [], image)
            observed = copy.deepcopy(words)
            page = dict(page=1, route='ocr', status='OCR_DRAFT', width=pixels.shape[1],
                        height=pixels.shape[0], image_path=str(image), image_sha256=core.sha(image),
                        words=words, decisions=decisions, original_apple=copy.deepcopy(apple),
                        independent_reader=copy.deepcopy(reader), residual_reader=[],
                        contributions=core.ownership(words))
            snapshot = dict(schema_version=1, revision=0, source_pdf=str(source),
                            input_sha256=core.sha(source), pages=[page], receipts=[],
                            mode='development-cached-merge-review',
                            evidence_scope='Synthetic composition and persisted review contract; no source spelling, OCR or App acceptance')
            session = root / 'session'
            initial = store.publish(session, snapshot)
            initial_hashes = {p.name: core.sha(p) for p in initial.iterdir() if p.is_file()}
            before, _ = store.load_snapshot(session)
            original_ownership = before['pages'][0]['contributions']
            patch = dict(expected_revision=0, input_sha256=snapshot['input_sha256'],
                         actions=[dict(page=1, member_id=member, action='accept')])
            receipt = store.review_save(session, patch)
            after, saved = store.load_snapshot(session)
            self.assertEqual((apple, reader), original_arrays)
            self.assertEqual(after['revision'], 1)
            self.assertEqual(before['pages'][0]['contributions'], after['pages'][0]['contributions'])
            self.assertEqual(after['pages'][0]['contributions'], original_ownership)
            self.assertEqual(after['pages'][0]['original_apple'], apple)
            self.assertEqual(after['pages'][0]['independent_reader'], reader)
            self.assertEqual(after['receipts'][-1]['actions'], patch['actions'])
            self.assertFalse(after['receipts'][-1]['human_approval_claimed'])
            self.assertEqual(receipt['revision'], 1)
            for prior, current in zip(before['pages'][0]['words'], after['pages'][0]['words']):
                expected = dict(prior, review=False, review_status='user_action_recorded') if prior['id'] == member else prior
                self.assertEqual(current, expected)
            self.assertEqual(initial_hashes, {p.name: core.sha(p) for p in initial.iterdir() if p.is_file()})
            for p in (initial / 'searchable.pdf', saved / 'searchable.pdf'):
                with fitz.open(p) as output, fitz.open(source) as original:
                    self.assertEqual(len(output), len(original))
                    self.assertEqual(output.get_toc(), original.get_toc())
                    self.assertEqual([w[4] for w in output[0].get_text('words')], [w['text'] for w in observed])
                    for a, b in zip(output, original):
                        self.assertEqual(a.get_pixmap().samples, b.get_pixmap().samples)
                    self.assertEqual(output[1].get_text(), original[1].get_text())
            accepted = next(w for w in after['pages'][0]['words'] if w['id'] == member)
            self.assertEqual(accepted['text'], expected_text)
            pointer = core.sha(session / 'CURRENT.json')
            with self.assertRaisesRegex(ValueError, 'STALE_REVISION'):
                store.review_save(session, patch)
            self.assertEqual(core.sha(session / 'CURRENT.json'), pointer)
            return after['pages'][0]['words'], original_ownership

    def test_accept_one_split_member_preserves_quote_sibling_and_joint_ownership(self):
        case = ReaderMergeCandidateTests.split_case(self)
        case[1][1].update(text='“contract', confidence=35)
        words, owners = self.check_persisted_review(case, 'left', 'reader')
        sibling = next(w for w in words if w['id'] == 'right')
        self.assertEqual(sibling['text'], '“contract')
        self.assertTrue(sibling['review'])
        self.assertEqual(len(owners), 1)
        self.assertEqual(owners[0]['reader_members'], ['left', 'right'])

    def test_accept_greek_padding_repair_preserves_adjacent_original_and_history(self):
        case = ReaderMergeCandidateTests.greek_case(self)
        words, _ = self.check_persisted_review(case, 'target', 'λόγος')
        self.assertEqual({w['id'] for w in words}, {'upper', 'target'})
        self.assertFalse(next(w for w in words if w['id'] == 'upper')['review'])


if __name__ == '__main__':
    unittest.main()
