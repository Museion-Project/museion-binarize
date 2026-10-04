"""Actual output geometry on source-first controlled print, no readers/quality panel."""
import copy
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import fitz

from scripts.ocr.free_local import pipeline
from . import core, store, position_export_candidate as candidate


class PositionExportCandidateTests(unittest.TestCase):
    def fixture(self, root, *, rotated=False, observation_scale=1, script_majority=False, literals=False):
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
        if literals:
            members = [('comma-1', 'Steel,', [30, 30, 70, 48]),
                       ('multi', 'qualify the', [82, 30, 160, 48]),
                       ('comma-2', 'Steel,', [30, 72, 70, 90]),
                       ('quote', '‘quote’', [82, 72, 145, 90]),
                       ('greek', 'α\u0313', [30, 114, 60, 132])]
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

    def emitted(self, text='A'):
        """Real local producer; mutations below test compatibility, not source truth."""
        doc = fitz.open()
        page = doc.new_page()
        writer = fitz.TextWriter(page.rect)
        writer.append((20, 40), text + ' ', font=fitz.Font(fontfile=str(pipeline.FONT)), fontsize=10)
        writer.write_text(page, render_mode=3)
        xref = page.get_contents()[-1]
        fontxref = page.get_fonts()[0][0]
        cmapxref = int(doc.xref_get_key(fontxref, 'ToUnicode')[1].split()[0])
        return doc, page, xref, fontxref, cmapxref

    def tiny_cmap(self, body=b'2 beginbfchar\n<0001> <0041>\n<0003> <0020>\nendbfchar'):
        return (b'begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n' +
                body + b'\nendcmap')

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

    def test_invalid_member_projection_refused_before_new_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            _, snapshot, _ = self.fixture(root)
            duplicate = copy.deepcopy(snapshot)
            duplicate['pages'][0]['words'].append(copy.deepcopy(duplicate['pages'][0]['words'][0]))
            bad_box = copy.deepcopy(snapshot)
            bad_box['pages'][0]['words'][0]['bbox'] = [-1, 60, 80, 80]
            stale = copy.deepcopy(snapshot)
            stale['pages_hash'] = core.digest(stale['pages'])
            stale['pages'][0]['words'][0]['text'] = 'Changed without revision hash'
            for index, invalid in enumerate((duplicate, bad_box, stale)):
                with self.subTest(index=index):
                    frozen = copy.deepcopy(invalid)
                    output = root / f'invalid-{index}.pdf'
                    with self.assertRaises(ValueError):
                        candidate.export_candidate(invalid, output)
                    self.assertFalse(output.exists())
                    self.assertEqual(invalid, frozen)

    def test_whole_actualtext_retains_multiword_repeats_quotes_and_combining_greek(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, snapshot, _ = self.fixture(root, literals=True)
            before = copy.deepcopy(snapshot)
            output = root / 'candidate.pdf'
            candidate.export_candidate(snapshot, output)
            expected = [w['text'] for w in snapshot['pages'][0]['words']]
            with fitz.open(output) as doc:
                actuals = []
                for xref in doc[0].get_contents():
                    for value in re.findall(rb'/ActualText <([0-9a-f]+)>', doc.xref_stream(xref)):
                        actuals.append(bytes.fromhex(value.decode()).decode('utf-16'))
                self.assertEqual(actuals, [text + ' ' for text in expected])
                self.assertEqual(doc[0].get_text().split(), ' '.join(expected).split())
                self.assertEqual(doc[0].get_text().split().count('Steel,'), 2)
                self.assertNotIn('=', doc[0].get_text())
            self.assert_preserved(source, output)
            self.assertEqual(snapshot, before)

    def test_actual_generated_font_map_and_whitespace_TJ_are_supported(self):
        doc, page, xref, _, _ = self.emitted('Steel,')
        try:
            original = doc.xref_stream(xref)
            codes = re.search(rb'<([0-9a-f]+)>', original).group(1)
            modified = original.replace(b'[<' + codes + b'>]TJ', b'[ <' + b' '.join(codes[i:i+4] for i in range(0, len(codes), 4)) + b'> ] TJ')
            doc.update_stream(xref, modified)
            candidate.mark_complete_text(page, xref, 'Steel,', {})
            wrapped = doc.xref_stream(xref)
            self.assertEqual(wrapped.count(b'/ActualText'), 1)
            self.assertIn(modified[modified.index(b'['):modified.index(b'ET')].strip(), wrapped)
        finally:
            doc.close()

    def test_bfchar_unicode_sequences_and_supplementary_character_supported(self):
        doc, page, xref, _, cmapxref = self.emitted()
        try:
            stream = re.sub(rb'\[<[^>]+>\]TJ', b'[<000100020003>]TJ', doc.xref_stream(xref))
            doc.update_stream(xref, stream)
            doc.update_stream(cmapxref, self.tiny_cmap(b'3 beginbfchar\n<0001><00660069>\n<0002><D83DDE00>\n<0003><0020>\nendbfchar'))
            candidate.mark_complete_text(page, xref, 'fi😀', {})
            value = re.search(rb'/ActualText <([^>]+)>', doc.xref_stream(xref)).group(1)
            self.assertEqual(bytes.fromhex(value.decode()).decode('utf-16'), 'fi😀 ')
        finally:
            doc.close()

    def test_wrong_internal_mapping_and_changed_cached_map_refused_without_stream_write(self):
        doc, page, xref, _, cmapxref = self.emitted()
        try:
            stream = re.sub(rb'\[<[^>]+>\]TJ', b'[<00010003>]TJ', doc.xref_stream(xref))
            doc.update_stream(xref, stream)
            doc.update_stream(cmapxref, self.tiny_cmap())
            cache = {}
            candidate.mark_complete_text(page, xref, 'A', cache)
            doc.update_stream(xref, stream)
            for broken in (self.tiny_cmap().replace(b'<0041>', b'<0042>'),
                           self.tiny_cmap().replace(b'<0020>', b'<003d>'),
                           self.tiny_cmap().replace(b'<0041>', b'<d800>')):
                doc.update_stream(cmapxref, broken)
                with self.assertRaisesRegex(ValueError, 'EXPORT_TEXT_CMAP_MISMATCH'):
                    candidate.mark_complete_text(page, xref, 'A', cache)
                self.assertEqual(doc.xref_stream(xref), stream)
        finally:
            doc.close()

    def test_unsupported_or_ambiguous_cmap_shapes_fail_explicitly(self):
        valid = self.tiny_cmap()
        invalid = [valid.replace(b'2 beginbfchar', b'3 beginbfchar'),
                   valid + b'\n1 beginbfchar',
                   valid.replace(b'<0000> <FFFF>', b'<00> <FF>'),
                   valid.replace(b'<0001> <0041>', b'<0003> <0041>'),
                   valid.replace(b'endcmap', b'/Other usecmap\nendcmap'),
                   valid.replace(b'endcmap', b'1 begincidchar\nendcidchar\nendcmap'),
                   valid.replace(b'endcmap', b'1 usefont\nendcmap'),
                   valid.replace(b'endbfchar', b'endbfrange'),
                   self.tiny_cmap(b'1 beginbfrange\n<0001><0002>[<0041><0042>]\nendbfrange'),
                   self.tiny_cmap(b'1 beginbfrange\n<0002><0001><0041>\nendbfrange'),
                   self.tiny_cmap(b'1 beginbfrange\n<0001><0002><ffff>\nendbfrange'),
                   self.tiny_cmap(b'1 beginbfrange\n<0001><0002><00660069>\nendbfrange')]
        for index, cmap in enumerate(invalid):
            with self.subTest(index=index), self.assertRaisesRegex(ValueError, 'EXPORT_FONT_CMAP_'):
                candidate._cmap_mappings(cmap)

    def test_selected_range_with_last_unicode_byte_rollover_is_refused(self):
        doc, page, xref, _, cmapxref = self.emitted()
        try:
            stream = re.sub(rb'\[<[^>]+>\]TJ', b'[<00010003>]TJ', doc.xref_stream(xref))
            doc.update_stream(xref, stream)
            body = b'1 beginbfrange\n<0001><0002><00ff>\nendbfrange\n1 beginbfchar\n<0003><0020>\nendbfchar'
            doc.update_stream(cmapxref, self.tiny_cmap(body))
            with self.assertRaisesRegex(ValueError, 'EXPORT_FONT_CMAP_RANGE_UNSUPPORTED'):
                candidate.mark_complete_text(page, xref, 'ÿ', {})
            self.assertEqual(doc.xref_stream(xref), stream)
            doc.update_stream(cmapxref, self.tiny_cmap(body.replace(b'<00ff>', b'<0041>')))
            candidate.mark_complete_text(page, xref, 'A', {})
            self.assertEqual(doc.xref_stream(xref).count(b'/ActualText'), 1)
        finally:
            doc.close()

    def test_unsupported_text_operators_are_not_partially_wrapped(self):
        doc, page, xref, _, _ = self.emitted()
        try:
            original = doc.xref_stream(xref)
            operator = re.search(rb'\[<[^>]+>\]TJ', original).group()
            invalid = [original.replace(operator, b'(A ) Tj'),
                       original.replace(operator, b'[<0024> 10 <0003>]TJ'),
                       original.replace(operator, operator + b'\n' + operator),
                       original.replace(operator, b'[<0024000>]TJ'),
                       original.replace(operator, b'[<>]TJ'),
                       original.replace(b'/F0 10 Tf', b'/F0 10 Tf\n/F1 10 Tf'),
                       original.replace(b'3 Tr', b'0 Tr')]
            for index, stream in enumerate(invalid):
                with self.subTest(index=index):
                    doc.update_stream(xref, stream)
                    with self.assertRaisesRegex(ValueError, 'EXPORT_'):
                        candidate.mark_complete_text(page, xref, 'A', {})
                    self.assertEqual(doc.xref_stream(xref), stream)
        finally:
            doc.close()

    def test_inherited_or_wrong_encoding_and_missing_font_resource_are_refused(self):
        mutations = [('encoding', 'Encoding', '/Identity-V'),
                     ('subtype', 'Subtype', '/TrueType'),
                     ('missing-map', 'ToUnicode', 'null'),
                     ('inherited-map', 'UseCMap', '42 0 R')]
        for name, key, value in mutations:
            doc, page, xref, fontxref, cmapxref = self.emitted()
            try:
                original = doc.xref_stream(xref)
                doc.xref_set_key(cmapxref if key == 'UseCMap' else fontxref, key, value)
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'EXPORT_FONT_'):
                    candidate.mark_complete_text(page, xref, 'A', {})
                self.assertEqual(doc.xref_stream(xref), original)
            finally:
                doc.close()
        doc, page, xref, _, _ = self.emitted()
        try:
            original = doc.xref_stream(xref)
            doc.xref_set_key(page.xref, 'Resources/Font/F0', 'null')
            with self.assertRaisesRegex(ValueError, 'EXPORT_FONT_RESOURCE_UNSUPPORTED'):
                candidate.mark_complete_text(page, xref, 'A', {})
            self.assertEqual(doc.xref_stream(xref), original)
        finally:
            doc.close()

    def test_multiple_producer_streams_refuse_candidate_without_admission(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            _, snapshot, _ = self.fixture(root)
            original = fitz.TextWriter.write_text
            before = copy.deepcopy(snapshot)
            def twice(writer, *args, **kwargs):
                original(writer, *args, **kwargs)
                return original(writer, *args, **kwargs)
            output = root / 'failed-new-candidate.pdf'
            with patch.object(fitz.TextWriter, 'write_text', twice):
                with self.assertRaisesRegex(ValueError, 'EXPORT_TEXT_STREAM_UNSUPPORTED'):
                    candidate.export_candidate(snapshot, output)
            self.assertEqual(output.stat().st_size, 0)
            self.assertEqual(snapshot, before)
