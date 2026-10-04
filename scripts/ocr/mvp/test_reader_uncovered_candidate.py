"""Counterexamples for revisiting partial-overlap Greek observations."""
import copy
import tempfile
import unittest
from pathlib import Path

import cv2
import numpy as np

from . import core, reader_merge_candidate as v1, reader_uncovered_candidate as v2
from .test_reader_merge_candidate import word


class UncoveredCandidateTests(unittest.TestCase):
    def case(self):
        apple = [word('neighbor', 'peer', [49.999, 20, 80, 40])]
        reader = [word('target', 'λόγος', [10, 20, 50, 40], 'tesseract')]
        pixels = np.full((90, 100), 255, np.uint8)
        pixels[24:37, 15:44] = 0
        pixels[24:37, 57:75] = 0
        return apple, reader, pixels

    def compare(self, apple, reader, pixels, residual=None):
        residual = [] if residual is None else residual
        original = copy.deepcopy((apple, reader, residual))
        with tempfile.TemporaryDirectory(prefix='uncovered-merge-test-') as folder:
            p = Path(folder) / 'image.png'
            self.assertTrue(cv2.imwrite(str(p), pixels))
            before, ds = v1.compose(apple, reader, residual, p)
            after, result = v2.compose(apple, reader, residual, p)
        self.assertEqual((apple, reader, residual), original)
        core.ownership(after)
        for w in before:
            self.assertEqual(next(q for q in after if q['id'] == w['id']), w)
        return before, after, ds, result

    def test_empty_fractional_intersection_recovers_without_changing_neighbor(self):
        a, r, p = self.case()
        before, after, _, decisions = self.compare(a, r, p)
        self.assertEqual([w['id'] for w in before], ['neighbor'])
        target = next(w for w in after if w['id'] == 'target')
        self.assertEqual(target['text'], r[0]['text'])
        self.assertEqual(target['bbox'], r[0]['bbox'])
        self.assertEqual(target['source_members'], [])
        self.assertTrue(target['review'])
        self.assertFalse(decisions[-1]['spelling_confirmed'])

    def test_separate_complete_component_at_padding_contact_is_allowed(self):
        a, r, p = self.case()
        a[0]['bbox'] = [10, 45, 50, 75]
        r[0]['bbox'] = [10, 25, 50, 50]
        p[:] = 255
        p[28:38, 15:44] = 0
        p[44:47, 23:25] = 0
        p[60:70, 15:44] = 0
        _, after, _, ds = self.compare(a, r, p)
        self.assertIn('target', {w['id'] for w in after})
        proof = ds[-1]['source_pixel_conflicts_resolved'][0]['pixel_proof']
        self.assertEqual(proof['kind'], 'exclusive_complete_ink_components')

    def test_shared_ink_component_is_not_a_safe_padding_contact(self):
        a, r, p = self.case()
        a[0]['bbox'] = [10, 45, 50, 75]
        r[0]['bbox'] = [10, 25, 50, 50]
        p[:] = 255
        p[28:38, 15:44] = 0
        p[46:48, 23:25] = 0
        p[60:70, 15:44] = 0
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_low_confidence_cannot_gain_admission_from_pixel_separation(self):
        a, r, p = self.case()
        r[0]['confidence'] = 79
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_mixed_latin_greek_stays_in_the_original_policy(self):
        a, r, p = self.case()
        r[0]['text'] = 'λόyoς'
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_duplicate_missing_observations_are_both_left_pending(self):
        a, r, p = self.case()
        r.append(dict(r[0], id='duplicate'))
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_actual_touching_ink_is_not_discarded_as_box_padding(self):
        a, r, p = self.case()
        p[28:33, 43:58] = 0
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_source_pixel_in_fractional_contact_is_not_rounded_away(self):
        a, r, p = self.case()
        a[0]['bbox'][0] = 50.02
        r[0]['bbox'][2] = 50.1
        p[28, 50] = 0
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_conflicting_support_retired_by_baseline_does_not_block_forever(self):
        a, r, p = self.case()
        a.insert(0, word('survivor', 'peer', [50.1, 19.9, 80, 40]))
        before, after, _, ds = self.compare(a, r, p)
        self.assertEqual([w['id'] for w in before], ['survivor'])
        self.assertIn('target', {w['id'] for w in after})
        self.assertEqual(ds[-1]['prior_conflicting_supports_no_longer_active'], ['neighbor'])

    def test_large_existing_owner_is_not_replaced_by_an_uncovered_rule(self):
        a, r, p = self.case()
        a[0]['bbox'][0] = 30
        before, after, _, ds = self.compare(a, r, p)
        self.assertEqual(after, before)
        self.assertFalse(any(d.get('state') == 'UNCOVERED_GREEK_PADDING_DRAFT' for d in ds))

    def test_no_ink_cannot_establish_a_new_observation(self):
        a, r, p = self.case()
        p[:] = 255
        before, after, _, _ = self.compare(a, r, p)
        self.assertEqual(after, before)

    def test_residual_observations_are_not_promoted_by_the_full_reader_rule(self):
        a, r, p = self.case()
        r[0]['engine'] = 'residual'
        before, after, _, _ = self.compare(a, [], p, residual=r)
        self.assertEqual(after, before)

    def test_input_order_does_not_choose_between_duplicate_sources(self):
        a, r, p = self.case()
        r.append(dict(r[0], id='duplicate'))
        first = self.compare(a, r, p)[1]
        second = self.compare(list(reversed(a)), list(reversed(r)), p)[1]
        self.assertEqual(first, second)


if __name__ == '__main__':
    unittest.main()
