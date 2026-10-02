"""Unadmitted output-only candidate preserving smaller-token vertical position.

The formal publisher/bridge/App never select this module. A controlled glyph
fixture proves only its exercised placement; observed boxes are not certified
source geometry and this candidate has not passed the full consumer panel.
"""
import copy
import os
import statistics
from pathlib import Path

import fitz

from . import store
from .core import digest, invariant, reading_rows

CANDIDATE_VERSION = 'row-sequence-size-position-candidate-v1'
HEIGHT_BAND = .8


def insert_positioned(page, words, width, height, fonts):
    """Keep a row baseline for comparable sizes, own baseline for smaller tokens.

    No literal/ID/recognition confidence/source answer influences placement.
    The largest observed height is a size-band anchor, not a claim of body-text
    identity. Reader-frame errors and natural reading order remain unverified.
    """
    sx, sy = page.rect.width / width, page.rect.height / height
    for row in reading_rows(words):
        positioned = []
        for word in row:
            if word['export_status'] != 'EXPORTED':
                continue
            font = fonts[word['export_font_path']]
            x, y, r, b = word['bbox']
            rect = fitz.Rect(x * sx, y * sy, r * sx, b * sy)
            size = max(1, rect.height / (font.ascender - font.descender))
            positioned.append((word, font, rect, size, rect.y0 + font.ascender * size))
        if not positioned:
            continue
        tallest = max(p[2].height for p in positioned)
        common = statistics.median(p[4] for p in positioned if p[2].height >= HEIGHT_BAND * tallest)
        for word, font, rect, size, own_baseline in positioned:
            baseline = common if rect.height >= HEIGHT_BAND * tallest else own_baseline
            point = fitz.Point(rect.x0, baseline)
            writer = fitz.TextWriter(page.rect)
            writer.append(point, word['text'] + ' ', font=font, fontsize=size)
            writer.write_text(page, render_mode=3,
                              morph=(point, fitz.Matrix(rect.width / max(.001, font.text_length(word['text'], fontsize=size)), 1)))


def export_candidate(snapshot, path):
    """Write a private candidate PDF and return a distinct non-admission receipt.

    It neither publishes nor modifies an input snapshot/CURRENT/history. A
    caller cannot treat this receipt as a manifest, revision or product gate.
    """
    candidate = copy.deepcopy(snapshot)
    if 'pages_hash' in candidate:
        invariant(candidate)
    else:
        candidate['pages_hash'] = digest(candidate['pages'])
        invariant(candidate)
    store.prepare_export(candidate)
    candidate['pages_hash'] = digest(candidate['pages'])
    invariant(candidate)
    candidate['exporter_version'] = CANDIDATE_VERSION
    if store.sha(candidate['source_pdf']) != candidate['input_sha256']:
        raise ValueError('SOURCE_CHANGED')
    path = Path(path)
    # File-backed output reserves a new destination without replacing a saved
    # source/revision/unrelated artifact. Failed new output is never admission.
    with path.open('xb') as output:
        store._export_pdf_with_inserter(candidate, output, insert_positioned)
        output.flush()
        os.fsync(output.fileno())
    return dict(candidate_version=CANDIDATE_VERSION,
                source_sha256=candidate['input_sha256'], output_sha256=store.sha(path),
                formal_exporter_version=store.EXPORTER_VERSION,
                prepared_snapshot=candidate, candidate_only=True,
                quality_ready=False, app_admission=False,
                distribution_ready=False, release_ready=False)
