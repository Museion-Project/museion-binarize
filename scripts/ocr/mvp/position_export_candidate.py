"""Unadmitted output-only candidate preserving smaller-token vertical position.

The formal publisher/bridge/App never select this module. A controlled glyph
fixture proves only its exercised placement; observed boxes are not certified
source geometry and this candidate has not passed the full consumer panel.
"""
import copy
import hashlib
import os
import re
import statistics
from pathlib import Path

import fitz

from . import store
from .core import digest, invariant, reading_rows

CANDIDATE_VERSION = 'row-sequence-size-position-actualtext-candidate-v2'
HEIGHT_BAND = .8

# This is a compatibility contract for the locally generated TextWriter stream,
# not a parser for arbitrary PDF content. Tj/literal strings, segmented TJ,
# multiple fonts/text objects, inherited CMaps and array ranges are refused.
_NUMBER = rb'[+-]?(?:\d+(?:\.\d*)?|\.\d+)'
_MATRIX = (rb'(?:' + _NUMBER + rb'\s+){6}')
_WRITER_STREAM = re.compile(
    rb'\s*q\s+(?:' + _MATRIX + rb'cm\s+)?BT\s+3\s+Tr\s+' +
    _NUMBER + rb'\s+w\s+/(?P<font>[A-Za-z0-9]+)\s+' + _NUMBER +
    rb'\s+Tf\s+' + _MATRIX + rb'Tm\s+(?P<operator>\[\s*' +
    rb'<(?P<codes>[0-9A-Fa-f\s]+)>\s*\]\s*TJ)\s+ET\s+Q\s*')
_HEX = rb'<([0-9A-Fa-f\s]+)>'


def _hex_bytes(value):
    value = re.sub(rb'\s+', b'', value)
    if len(value) % 2:
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    return bytes.fromhex(value.decode('ascii'))


def _cmap_mappings(cmap):
    """Bounded Identity-H map: bfchar UTF-16BE and sequential BMP bfrange.

    Parse every declared mapping and its count. Never infer codes from font
    glyph indices, override overlaps, or silently ignore an array-range entry.
    Multi-codepoint/supplementary bfchar values are supported; sequential ranges
    with more than one UTF-16 code unit are explicitly unsupported.
    """
    cmap = re.sub(rb'%[^\r\n]*', b'', cmap)
    if (len(re.findall(rb'\bbegincmap\b', cmap)) != 1 or
            len(re.findall(rb'\bendcmap\b', cmap)) != 1 or
            re.search(rb'\b(?:usecmap|usefont|begin(?:cidchar|cidrange|notdefchar|notdefrange))\b', cmap)):
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    blocks = list(re.finditer(rb'(\d+)\s+begin(codespacerange|bfchar|bfrange)\b(.*?)\bend\2\b', cmap, re.S))
    if 2 * len(blocks) != len(re.findall(rb'\b(?:begin|end)(?:codespacerange|bfchar|bfrange)\b', cmap)):
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    begin = re.search(rb'\bbegincmap\b', cmap).end()
    end = re.search(rb'\bendcmap\b', cmap).start()
    if not begin < end or any(not begin <= block.start() < block.end() <= end for block in blocks):
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    spaces, ranges = [], []
    for block in blocks:
        kind, body = block.group(2), block.group(3)
        count = int(block.group(1))
        columns = 3 if kind == b'bfrange' else 2
        pattern = rb'\s*'.join([_HEX] * columns)
        entries = list(re.finditer(pattern, body))
        if count != len(entries) or re.sub(pattern, b'', body).strip():
            raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
        for entry in entries:
            values = [_hex_bytes(v) for v in entry.groups()]
            if len(values[0]) != 2:
                raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
            first = int.from_bytes(values[0], 'big')
            if kind == b'bfchar':
                last, destination = first, values[1]
            else:
                if len(values[1]) != 2:
                    raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
                last = int.from_bytes(values[1], 'big')
                destination = values[2] if kind == b'bfrange' else None
            if first > last:
                raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
            if kind == b'codespacerange':
                spaces.append((first, last))
                continue
            if not destination or len(destination) % 2:
                raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
            if kind == b'bfrange' and (len(destination) != 2 or
                    int.from_bytes(destination, 'big') + last - first > 65535):
                raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
            ranges.append((first, last, destination, kind))
    if spaces != [(0, 65535)] or not ranges:
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    ranges.sort(key=lambda item: item[0])
    if any(a[1] >= b[0] for a, b in zip(ranges, ranges[1:])):
        raise ValueError('EXPORT_FONT_CMAP_AMBIGUOUS')
    return ranges


def mark_complete_text(page, xref, text, cache):
    """Validate the emitted complete reading, then wrap its whole TJ once.

    The trailing space belongs to the same operator. Splitting it off can
    duplicate a consumer glyph; checking only the last CID would miss a wrong
    internal mapping. PDF resources/CMap bytes, rather than guessed IDs, bind
    this output-only repair. No stream is changed on a validation failure.
    """
    document = page.parent
    stream = document.xref_stream(xref)
    match = _WRITER_STREAM.fullmatch(stream)
    if not match:
        raise ValueError('EXPORT_TEXT_OPERATOR_UNSUPPORTED')
    resource = document.xref_get_key(page.xref, 'Resources/Font/' + match.group('font').decode('ascii'))
    if resource[0] != 'xref':
        raise ValueError('EXPORT_FONT_RESOURCE_UNSUPPORTED')
    fontxref = int(resource[1].split()[0])
    if (document.xref_get_key(fontxref, 'Subtype') != ('name', '/Type0') or
            document.xref_get_key(fontxref, 'Encoding') != ('name', '/Identity-H')):
        raise ValueError('EXPORT_FONT_RESOURCE_UNSUPPORTED')
    cmapref = document.xref_get_key(fontxref, 'ToUnicode')
    if cmapref[0] != 'xref':
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    cmapxref = int(cmapref[1].split()[0])
    if document.xref_get_key(cmapxref, 'UseCMap')[0] != 'null':
        raise ValueError('EXPORT_FONT_CMAP_UNSUPPORTED')
    cmap = document.xref_stream(cmapxref)
    key = (fontxref, hashlib.sha256(cmap).hexdigest())
    if key not in cache:
        cache[key] = _cmap_mappings(cmap)
    codes = _hex_bytes(match.group('codes'))
    if not codes or len(codes) % 2:
        raise ValueError('EXPORT_TEXT_OPERATOR_UNSUPPORTED')
    reading = []
    for index in range(0, len(codes), 2):
        code = int.from_bytes(codes[index:index + 2], 'big')
        entry = next((r for r in cache[key] if r[0] <= code <= r[1]), None)
        if entry is None:
            raise ValueError('EXPORT_TEXT_CMAP_MISMATCH')
        first, last, value, kind = entry
        if kind == b'bfrange':
            # ISO 32000-2 9.10.3: increment the final byte without rollover;
            # a range that would exceed 255 has undefined mapping. Some local
            # producer fonts contain such unused ranges. Never consume one as
            # evidence for this word, even when its first code precedes rollover.
            if value[-1] + last - first > 255:
                raise ValueError('EXPORT_FONT_CMAP_RANGE_UNSUPPORTED')
            value = (int.from_bytes(value, 'big') + code - first).to_bytes(2, 'big')
        try:
            reading.append(value.decode('utf-16-be'))
        except UnicodeDecodeError as error:
            raise ValueError('EXPORT_TEXT_CMAP_MISMATCH') from error
    if ''.join(reading) != text + ' ':
        raise ValueError('EXPORT_TEXT_CMAP_MISMATCH')
    actual = ('\ufeff' + text + ' ').encode('utf-16-be').hex().encode('ascii')
    start, end = match.span('operator')
    document.update_stream(xref, stream[:start] + b'/Span << /ActualText <' + actual +
                           b'> >> BDC\n' + stream[start:end] + b'\nEMC' + stream[end:])


def insert_positioned(page, words, width, height, fonts):
    """Keep a row baseline for comparable sizes, own baseline for smaller tokens.

    No literal/ID/recognition confidence/source answer influences placement.
    The largest observed height is a size-band anchor, not a claim of body-text
    identity. Reader-frame errors and natural reading order remain unverified.
    """
    sx, sy = page.rect.width / width, page.rect.height / height
    cmap_cache = {}
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
            before = set(page.get_contents())
            writer.write_text(page, render_mode=3,
                              morph=(point, fitz.Matrix(rect.width / max(.001, font.text_length(word['text'], fontsize=size)), 1)))
            added = [xref for xref in page.get_contents() if xref not in before]
            if len(added) != 1:
                raise ValueError('EXPORT_TEXT_STREAM_UNSUPPORTED')
            mark_complete_text(page, added[0], word['text'], cmap_cache)


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
