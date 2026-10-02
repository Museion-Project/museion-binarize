"""Literal provider confidence adaptation; never invent word geometry or answers."""
import copy
import math


def score(value):
    return value if type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 1 else None


def adapt(page, block, start, end):
    container = page.get('confidence_scores')
    summary = {k: v for k, v in container.items() if k != 'word_confidence_scores'} if isinstance(container, dict) else container
    raw = copy.deepcopy(dict(block=block.get('confidence_scores'),
                             legacy_scalar=block.get('confidence'), page_summary=summary,
                             full_page_words_path='raw_response.pages[0].confidence_scores.word_confidence_scores'))
    result = dict(value=None, precision='UNKNOWN', reason='confidence_missing_or_invalid',
                  raw=raw, word_members=[], geometry_precision='paragraph')
    scores = block.get('confidence_scores')
    if isinstance(scores, dict):
        # Type confidence is not content recognition confidence.
        values = [score(scores.get(k)) for k in
                  ('average_content_confidence_score', 'minimum_content_confidence_score')]
        values = [v for v in values if v is not None]
        if values:
            result.update(value=min(values), precision='block_content', reason=None)
    legacy = score(block.get('confidence'))
    if result['value'] is None and legacy is not None:
        result.update(value=legacy, precision='legacy_block_scalar', reason=None)
    container = page.get('confidence_scores')
    words = container.get('word_confidence_scores') if isinstance(container, dict) else None
    if words is None:
        return result  # Page aggregates cannot be assigned to an individual line.
    text = page['markdown']
    if type(words) is not list or type(start) is not int or type(end) is not int:
        result.update(value=None, precision='UNKNOWN', reason='word_confidence_unbound')
        return result
    covered = set()
    previous = -1
    for index, word in enumerate(words):
        if not isinstance(word, dict):
            result.update(value=None, precision='UNKNOWN', reason='word_confidence_schema')
            return result
        a, token, value = word.get('start_index'), word.get('text'), score(word.get('confidence'))
        valid = type(a) is int and type(token) is str and bool(token) and value is not None
        b = a + len(token) if valid else -1
        if not valid or not (0 <= a < b <= len(text)) or a < previous or text[a:b] != token:
            result.update(value=None, precision='UNKNOWN', reason='word_confidence_offset_or_score')
            return result
        previous = b
        if b <= start or a >= end:
            continue
        if a < start or b > end:
            result.update(value=None, precision='UNKNOWN', reason='word_crosses_block_boundary')
            return result
        result['word_members'].append(dict(index=index, raw_start=a, raw_end=b,
                                            text=token, confidence=value))
        covered.update(range(a, b))
    if not result['word_members'] or any(not text[i].isspace() and i not in covered for i in range(start, end)):
        result.update(value=None, precision='UNKNOWN', reason='word_confidence_incomplete_coverage')
    else:
        result.update(value=min(w['confidence'] for w in result['word_members']),
                      precision='word_offsets_complete_block', reason=None)
    return result
