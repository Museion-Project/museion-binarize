"""Audited, opt-in /2 strategy. A page is accepted only after every batch validates.

The geometry contract and original transcription instruction are unchanged.
Plans are deterministic and caller-pinned. No implicit retry, resume, partial
page, historical-response fill, or reference-based selection is supported.
"""
import copy
import json
import time
import unicodedata
from pathlib import Path

import spatial_verification as v
from transcription_fidelity import explicit_nfc, require_lossless_candidate

STRATEGY = 'mpdf-spatial-small-batch/1'
MAX_SUPPORTS = 4


def encoded(value):
    return v.canonical(value).encode()


def save(path, value):
    with Path(path).open('xb') as f:
        f.write(encoded(value))


def sum_usage(rows):
    result = {}
    for key in ['promptTokenCount', 'candidatesTokenCount', 'thoughtsTokenCount', 'totalTokenCount']:
        values = [row.get(key) for row in rows]
        result[key] = sum(values) if all(type(x) is int and x >= 0 for x in values) else None
    return result


def plan(prepared, model):
    full = prepared.checked_payload()
    geometry = v.parse(prepared.geometry_json)
    groups, current = [], []
    for unit in geometry['spatial']['units']:
        ids = unit['support_ids']
        v.require(len(ids) <= MAX_SUPPORTS, 'Unit exceeds batch limit; policy decision required')
        if current and len(current) + len(ids) > MAX_SUPPORTS:
            groups.append(current)
            current = []
        current.extend(ids)
    if current:
        groups.append(current)
    all_ids = [s['support_id'] for s in geometry['spatial']['supports']]
    indices = {sid: i for i, sid in enumerate(all_ids)}
    batches = []
    for index, ids in enumerate(groups):
        payload = copy.deepcopy(full)
        parts = full['contents'][0]['parts']
        payload['contents'][0]['parts'] = [copy.deepcopy(parts[0])]
        for sid in ids:
            i = indices[sid]
            payload['contents'][0]['parts'].extend(copy.deepcopy(parts[1 + 2*i:3 + 2*i]))
        batches.append({'index': index, 'support_ids': ids,
                        'payload_sha256': v.sha(encoded(payload)), 'payload': payload})
    return {'schema': STRATEGY, 'model': model, 'geometry_sha256': prepared.expected.contract_sha256,
            'input_manifest_sha256': prepared.manifest_sha256,
            'full_payload_sha256': v.sha(encoded(full)),
            'max_supports': MAX_SUPPORTS, 'unit_boundary_policy': 'preserve_or_reject_oversize',
            'automatic_retries': 0, 'batches': batches}


def verify_plan(prepared, raw, expected_sha):
    v.hash_matches(raw, expected_sha, 'Selected batch plan')
    doc = v.parse(raw)
    v.require(doc == plan(prepared, doc['model']), 'Batch plan differs from checked canonical inputs')
    return doc


def normalize_vendor(raw, batch, page_digest, model):
    v.require(len(raw) <= 8*1024*1024, 'Vendor response too large')
    vendor = v.parse(raw)
    candidates = vendor.get('candidates', [])
    v.require(vendor.get('modelVersion') == model and len(candidates) == 1
              and candidates[0].get('finishReason') == 'STOP', 'Model or finish mismatch')
    text = ''.join(p['text'] for p in candidates[0]['content']['parts']
                   if 'text' in p and not p.get('thought', False))
    before = v.parse(text)
    v.require(set(before) == {'geometry_sha256', 'supports'}
              and before['geometry_sha256'] == page_digest, 'Batch parent or fields mismatch')
    supports = before['supports']
    v.require(isinstance(supports, list) and len(supports) == len(batch['support_ids']), 'Batch count mismatch')
    normalized = copy.deepcopy(before)
    changes = []
    for item, sid in zip(normalized['supports'], batch['support_ids']):
        v.require(set(item) == {'support_id', 'text'} and item['support_id'] == sid
                  and isinstance(item['text'], str), 'Batch ID/order/type mismatch')
        original = item['text']
        item['text'], _ = explicit_nfc(original)
        v.require(0 < len(item['text'].encode()) <= 16384, 'Invalid batch text length')
        v.require(unicodedata.normalize('NFD', original) == unicodedata.normalize('NFD', item['text']),
                  'Noncanonical text modification')
        if original != item['text']:
            changes.append({'support_id': sid, 'raw': original, 'nfc': item['text']})
        # The existing raw/NFC receipt is retained; canonical equivalence does
        # not make controls, unresolved glyphs or multiline formulas usable.
        require_lossless_candidate(item['text'])
    return text, normalized, changes, vendor.get('usageMetadata', {})


def aggregate(prepared, plan_raw, plan_sha, vendor_bodies, vendor_shas):
    p = verify_plan(prepared, plan_raw, plan_sha)
    v.require(len(vendor_bodies) == len(vendor_shas) == len(p['batches']), 'Incomplete batch collection')
    response = {'geometry_sha256': p['geometry_sha256'], 'supports': []}
    bindings = []
    for batch, raw, raw_sha in zip(p['batches'], vendor_bodies, vendor_shas):
        v.hash_matches(raw, raw_sha, 'Selected vendor response')
        _, normalized, _, _ = normalize_vendor(raw, batch, p['geometry_sha256'], p['model'])
        response['supports'].extend(normalized['supports'])
        bindings.append({'batch_index': batch['index'], 'payload_sha256': batch['payload_sha256'],
                         'vendor_sha256': raw_sha, 'normalized_sha256': v.sha(encoded(normalized))})
    result = v.canonical(response)
    prepared.validate_response(result, v.sha(result.encode()))
    return result, {'schema': 'mpdf-spatial-batch-aggregation/1', 'strategy': STRATEGY,
                    'plan_sha256': plan_sha, 'input_manifest_sha256': prepared.manifest_sha256,
                    'geometry_sha256': p['geometry_sha256'], 'batches': bindings,
                    'response_sha256': v.sha(result.encode()), 'complete_page': True,
                    'historical_fill_supports': 0}


def execute(prepared, plan_raw, plan_sha, output, transport, input_stop=90000):
    """transport(payload)->raw HTTP body; one attempt per batch, no hidden retries.

    Caller selects a new output directory and independently pinned plan. A failed
    directory is intentionally not resumable; completed batches remain evidence.
    """
    p = verify_plan(prepared, plan_raw, plan_sha)
    output = Path(output)
    output.mkdir(exist_ok=False)
    (output/'plan.json').write_bytes(plan_raw)
    bodies, hashes, usage_rows = [], [], []
    input_used = 0
    for batch in p['batches']:
        dest = output/f"batch-{batch['index']:04d}"
        dest.mkdir()
        save(dest/'payload.json', batch['payload'])
        try:
            v.require(input_used <= input_stop, 'Recorded input token stop reached')
            save(dest/'attempt.json', {'batch_index': batch['index'], 'attempt': 1,
                                      'plan_sha256': plan_sha, 'started_at_unix': time.time()})
            raw = transport(copy.deepcopy(batch['payload']))
            with (dest/'vendor-response.raw.json').open('xb') as f:
                f.write(raw)
            text, normalized, changes, usage = normalize_vendor(raw, batch, p['geometry_sha256'], p['model'])
            v.require(isinstance(usage, dict) and type(usage.get('promptTokenCount')) is int
                      and usage['promptTokenCount'] >= 0, 'Cannot enforce input budget without token usage')
            (dest/'raw-response.json').write_text(text)
            save(dest/'normalized-response.json', normalized)
            save(dest/'normalization.json', {'schema': 'mpdf-nfc-derivation/1', 'changes': changes,
                 'raw_sha256': v.sha(text.encode()), 'normalized_sha256': v.sha(encoded(normalized))})
            save(dest/'accepted.json', {'usage': usage, 'vendor_sha256': v.sha(raw), 'batch_index': batch['index']})
            bodies.append(raw)
            hashes.append(v.sha(raw))
            usage_rows.append(usage)
            input_used += usage.get('promptTokenCount', 0)
        except Exception as error:
            save(dest/'failure.json', {'error_type': type(error).__name__, 'automatic_retry': False})
            raise
    result, receipt = aggregate(prepared, plan_raw, plan_sha, bodies, hashes)
    receipt['usage'] = sum_usage(usage_rows)
    receipt['currency_cost'] = None
    (output/'response.json').write_text(result)
    save(output/'aggregation.json', receipt)
    return receipt
