"""Actual output geometry on source-first controlled print, no readers/quality panel."""
import copy
import tempfile
import unittest
from pathlib import Path

import fitz

from scripts.ocr.free_local import pipeline
from . import core, store, position_export_candidate as candidate


class PositionExportCandidateTests(unittest.TestCase):
    def fixture(self, root, *, rotated=False, observation_scale=1, script_majority=False):
        font = fitz.Font(fontfile=str(pipeline.FONT))
        members = [('body-a', 'Alpha', [30, 60, 80, 80]),
                   ('small-up', '7', [84, 58, 92, 68]),
                   ('body-b', 'omega', [100, 60, 155, 80]),
                   ('body-c', 'H', [30, 112, 44, 132]),
                   ('small-down', '2', [47, 124, 55, 134]),
                   ('body-d', 'oxygen', [64, 112, 120, 132]),
                   ('jitter-a', 'same', [30, 166, 78, 186]),
                   ('jitter-b', 'again', [90, 166, 145, 186])]
        if script_majority:
            members[2:3] = [('up-n', 'n', [96, 58, 104, 68]),
                            ('up-m', 'm', [108, 58, 116, 68]),
                            ('body-b', 'omega', [120, 60, 175, 80])]
        printed = fitz.open()
        page = printed.new_page(width=220, height=240)
        for _, text, box in members:
            rect = fitz.Rect(box)
            size = rect.height / (font.ascender - font.descender)
            point = fitz.Point(rect.x0, rect.y0 + font.ascender * size)
            writer = fitz.TextWriter(page.rect)
            writer.append(point, text, font=font, fontsize=size)
            writer.write_text(page, morph=(point, fitz.Matrix(rect.width / font.text_length(text, fontsize=size), 1)))
        expected = {w[4]: tuple(w[:4]) for w in page.get_text('words')}
        pixels = page.get_pixmap(matrix=fitz.Matrix(2, 2), alpha=False)
        source = root / 'source.pdf'
        image = fitz.open()
        stored_width, stored_height = (240, 220) if rotated else (220, 240)
        image.new_page(width=stored_width, height=stored_height).insert_image(fitz.Rect(0, 0, stored_width, stored_height), pixmap=pixels,
                                                        rotate=90 if rotated else 0)
        if rotated:
            image[0].set_rotation(90)
        image.new_page(width=230, height=130).insert_text((20, 30), 'Untouched native control')
        image.set_toc([[1, 'Selected', 1], [1, 'Untouched', 2]])
        image.save(source)
        image.close()
        printed.close()
        words = []
        for member_id, text, box in members:
            observed = list(box)
            if member_id.startswith('jitter-'):
                delta = -1 if member_id.endswith('a') else 1
                observed[1] += delta
                observed[3] += delta
            words.append(dict(id=member_id, text=text, bbox=[x * observation_scale for x in observed],
                              engine='apple', line_id='raw', confidence=95, source_members=[member_id]))
        page = dict(page=1, route='ocr', status='OCR_DRAFT', width=220 * observation_scale,
                    height=240 * observation_scale, words=words, original_apple=copy.deepcopy(words),
                    contributions=core.ownership(words))
        snapshot = dict(schema_version=1, revision=0, source_pdf=str(source), input_sha256=core.sha(source),
                        font_path=str(pipeline.FONT), fallback_font_paths=[], pages=[page], receipts=[])
        return source, snapshot, expected

    def assert_geometry(self, output, expected):
        with fitz.open(output) as doc:
            words = doc[0].get_text('words')
            self.assertEqual(len(words), len(expected))
            actual = {w[4]: w[:4] for w in words}
            self.assertEqual(set(actual), set(expected))
            for text, rect in expected.items():
                for found, target in zip(actual[text], rect):
                    self.assertAlmostEqual(found, target, delta=.25, msg=text)

    def assert_preserved(self, source, output):
        with fitz.open(source) as original, fitz.open(output) as exported:
            self.assertEqual(len(exported), len(original))
            self.assertEqual(exported.get_toc(), original.get_toc())
            for a, b in zip(original, exported):
                self.assertEqual(a.rect, b.rect)
                self.assertEqual(a.get_pixmap(matrix=fitz.Matrix(2, 2)).samples,
                                 b.get_pixmap(matrix=fitz.Matrix(2, 2)).samples)
            self.assertEqual(exported[1].get_text(), original[1].get_text())

    def test_scripts_body_jitter_source_snapshot_ownership_and_history(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, snapshot, expected = self.fixture(root)
            frozen = copy.deepcopy(snapshot)
            folder = store.publish(root / 'history', snapshot)
            pointer = core.sha(root / 'history/CURRENT.json')
            existing = {p.name: core.sha(p) for p in folder.iterdir() if p.is_file()}
            stored = store.load_snapshot(root / 'history')[0]
            stored_before = copy.deepcopy(stored)
            output = root / 'candidate.pdf'
            receipt = candidate.export_candidate(stored, output)
            self.assert_geometry(output, expected)
            self.assert_preserved(source, output)
            self.assertEqual(stored, stored_before)
            self.assertEqual(core.sha(root / 'history/CURRENT.json'), pointer)
            self.assertEqual({p.name: core.sha(p) for p in folder.iterdir() if p.is_file()}, existing)
            self.assertEqual(stored['pages'][0]['original_apple'], frozen['pages'][0]['original_apple'])
            self.assertEqual(stored['pages'][0]['contributions'], frozen['pages'][0]['contributions'])
            self.assertTrue(receipt['candidate_only'])
            self.assertFalse(receipt['app_admission'])
            self.assertEqual(receipt['prepared_snapshot']['exporter_version'], candidate.CANDIDATE_VERSION)
            self.assertEqual(stored['exporter_version'], store.EXPORTER_VERSION)

    def test_rotation_and_observation_scaling_preserve_real_coordinates(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, snapshot, expected = self.fixture(root, rotated=True, observation_scale=2)
            before = copy.deepcopy(snapshot)
            output = root / 'candidate.pdf'
            candidate.export_candidate(snapshot, output)
            self.assert_geometry(output, expected)
            self.assert_preserved(source, output)
            self.assertEqual(snapshot, before)

    def test_formal_default_still_preserves_exact_prior_output_and_version(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            _, snapshot, expected = self.fixture(root)
            store.prepare_export(snapshot)
            output = root / 'formal.pdf'
            store.export_pdf(snapshot, output)
            with fitz.open(output) as doc:
                actual = {w[4]: w[:4] for w in doc[0].get_text('words')}
            self.assertGreater(abs(actual['7'][1] - expected['7'][1]), 9)
            self.assertGreater(abs(actual['2'][1] - expected['2'][1]), 4)
            self.assertEqual(snapshot['exporter_version'], 'row-sequence-glyph-v1')

    def test_source_change_rejected_without_output_or_snapshot_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, snapshot, _ = self.fixture(root)
            before = copy.deepcopy(snapshot)
            source.write_bytes(source.read_bytes() + b'\nchanged')
            output = root / 'candidate.pdf'
            with self.assertRaisesRegex(ValueError, 'SOURCE_CHANGED'):
                candidate.export_candidate(snapshot, output)
            self.assertFalse(output.exists())
            self.assertEqual(snapshot, before)

    def test_several_small_tokens_cannot_pull_body_to_their_baseline(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, snapshot, expected = self.fixture(root, script_majority=True)
            output = root / 'candidate.pdf'
            candidate.export_candidate(snapshot, output)
            self.assert_geometry(output, expected)
            self.assert_preserved(source, output)

    def test_existing_destination_cannot_be_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            _, snapshot, _ = self.fixture(root)
            before = copy.deepcopy(snapshot)
            output = root / 'existing.pdf'
            output.write_bytes(b'Unrelated existing artifact')
            frozen_hash = core.sha(output)
            with self.assertRaises(FileExistsError):
                candidate.export_candidate(snapshot, output)
            self.assertEqual(core.sha(output), frozen_hash)
            self.assertEqual(snapshot, before)
