"""Rebuild a completed /2 page from pinned vendor bodies before consumption."""
from pathlib import Path

import spatial_batch_strategy as s
import spatial_verification as v


def verify_completed_run(prepared, directory, plan_sha, aggregation_sha):
    root = Path(directory)
    raw_plan = (root/'plan.json').read_bytes()
    plan = s.verify_plan(prepared, raw_plan, plan_sha)
    receipt_raw = (root/'aggregation.json').read_bytes()
    v.hash_matches(receipt_raw, aggregation_sha, 'Selected aggregation')
    receipt = v.parse(receipt_raw)
    v.require(len(receipt['batches']) == len(plan['batches']), 'Incomplete aggregation receipt')
    expected_dirs = {f"batch-{b['index']:04d}" for b in plan['batches']}
    v.require({p.name for p in root.glob('batch-*')} == expected_dirs, 'Unexpected or missing batch directory')
    bodies, hashes, usages = [], [], []
    for batch, binding in zip(plan['batches'], receipt['batches']):
        dest = root/f"batch-{batch['index']:04d}"
        v.require(not (dest/'failure.json').exists(), 'Failed batch cannot be consumed')
        v.hash_matches((dest/'payload.json').read_bytes(), batch['payload_sha256'], 'Saved request payload')
        attempt = v.parse((dest/'attempt.json').read_bytes())
        v.require(attempt['attempt'] == 1 and attempt['batch_index'] == batch['index']
                  and attempt['plan_sha256'] == plan_sha, 'Attempt binding mismatch')
        raw = (dest/'vendor-response.raw.json').read_bytes()
        v.hash_matches(raw, binding['vendor_sha256'], 'Selected vendor body')
        text, normalized, changes, usage = s.normalize_vendor(raw, batch, plan['geometry_sha256'], plan['model'])
        v.require((dest/'raw-response.json').read_text() == text
                  and (dest/'normalized-response.json').read_bytes() == s.encoded(normalized),
                  'Saved transcription differs from vendor derivation')
        normalization = v.parse((dest/'normalization.json').read_bytes())
        v.require(normalization == {'schema':'mpdf-nfc-derivation/1','changes':changes,
                  'raw_sha256':v.sha(text.encode()),'normalized_sha256':v.sha(s.encoded(normalized))},
                  'Normalization ledger differs')
        accepted = v.parse((dest/'accepted.json').read_bytes())
        v.require(accepted == {'usage':usage,'vendor_sha256':v.sha(raw),'batch_index':batch['index']},
                  'Batch acceptance record differs')
        bodies.append(raw);hashes.append(binding['vendor_sha256']);usages.append(usage)
    response, rebuilt = s.aggregate(prepared, raw_plan, plan_sha, bodies, hashes)
    rebuilt['usage'] = s.sum_usage(usages)
    rebuilt['currency_cost'] = None
    v.require(receipt == rebuilt and (root/'response.json').read_text() == response,
              'Aggregated page differs from complete vendor collection')
    return response, rebuilt
