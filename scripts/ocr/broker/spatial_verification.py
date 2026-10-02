"""Checked, offline-capable /2 preparation with caller-selected artifact pins.

Pins must come from an independent selection/manifest, never the submitted JSON.
Hashes establish identity and reproducibility, not authentication. Existing
Python canonical hashes are verified here; Rust verifies exact prepared bytes.
"""
from dataclasses import dataclass
import base64
import hashlib
import io
import json
import re

import spatial_contract as legacy
import finalist_adapters as adapter

MANIFEST = 'mpdf-spatial-input-manifest/1'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=False, allow_nan=False)


def parse(raw):
    def object_pairs(pairs):
        result = {}
        for k, v in pairs:
            require(k not in result, 'Duplicate JSON key')
            result[k] = v
        return result
    def invalid_constant(_):
        raise ValueError('Nonfinite JSON number')
    return json.loads(raw, object_pairs_hook=object_pairs,
                      parse_constant=invalid_constant)


def require(ok, message):
    if not ok:
        raise ValueError(message)


def hash_matches(raw, expected, label):
    require(isinstance(expected, str) and re.fullmatch('[0-9a-f]{64}', expected)
            and sha(raw) == expected, label + ' digest mismatch')


def self_hash(doc, field):
    hash_matches(canonical({k: v for k, v in doc.items() if k != field}).encode(),
                 doc[field], field)


@dataclass(frozen=True)
class ExpectedArtifacts:
    contract_sha256: str
    parent_artifact_sha256: str
    parent_geometry_sha256: str
    spatial_artifact_sha256: str
    image_sha256: str


@dataclass(frozen=True)
class PreparedSupportPage:
    geometry_json: str
    manifest_json: str
    payload_json: str
    expected: ExpectedArtifacts
    _manifest_sha256: str

    @property
    def manifest_sha256(self):
        return self._manifest_sha256

    def checked_payload(self):
        """The dispatch boundary rechecks actual labeled PNG bytes and payload."""
        value = parse(self.payload_json)
        verify_payload(value, self.manifest_json, self.manifest_sha256)
        return value

    def validate_response(self, response_json, expected_response_sha256):
        hash_matches(self.geometry_json.encode(), self.expected.contract_sha256, 'Contract')
        hash_matches(response_json.encode(), expected_response_sha256, 'Selected response')
        legacy.validate_response(self.geometry_json, parse(response_json))


def verify_parent(parent, receipt_bytes, image_bytes=None):
    """Verify raw/source mappings, including separately retained local detections."""
    self_hash(parent, 'geometry_sha256')
    recovery = parent.get('provenance', {}).get('component_recovery')
    if recovery is not None:
        require(image_bytes is not None and recovery.get('version') == 1,
                'Component recovery requires source image')
        base = recovery['base']
        require('component_recovery' not in base.get('provenance', {}), 'Nested component recovery')
        verify_parent(base, receipt_bytes, image_bytes)
        from local_component_recovery import recover
        records = [{'path': r['path'], 'sha256': r['sha256'], 'receipt': parse(receipt_bytes[r['path']])}
                   for r in base['provenance']['local_redetection']['receipts']]
        with legacy.Image.open(io.BytesIO(image_bytes)) as im:
            recomputed = recover(base, im.convert('RGB'), records)
        require(recomputed == parent, 'Component recovery evidence or geometry mismatch')
        return {f['fragment_id']: f for f in parent['fragments']}
    hash_matches(canonical(parent['provider_raw']).encode(),
                 parent['provider_raw_sha256'], 'Raw detector')
    originals = parent['source_fragments']
    require([{k: f[k] for k in ('bbox', 'polygon', 'confidence')} for f in originals]
            == adapter.raw_boxes(parent['provider'], parent['provider_raw']),
            'Raw source geometry mismatch')
    source = {f['fragment_id']: f for f in originals}
    require(len(source) == len(originals), 'Duplicate raw source IDs')
    for i, f in enumerate(originals):
        key = 'dt_polys' if parent['provider'] == 'paddle' else 'bboxes'
        require(f['source_pointer'] == f'/provider_raw/detection/{key}/{i}'
                and f['source_box_index'] == i, 'Raw source pointer mismatch')
    local = parent.get('provenance', {}).get('local_redetection', {})
    receipts = {}
    for entry in local.get('receipts', []):
        path = entry['path']
        require(path not in receipts and path in receipt_bytes, 'Missing/duplicate local receipt')
        hash_matches(receipt_bytes[path], entry['sha256'], 'Local receipt')
        receipt = parse(receipt_bytes[path])
        require(receipt['page_id'] == parent['page_id'] and receipt['provider'] == parent['provider']
                and receipt['image_sha256'] == (sha(image_bytes) if image_bytes is not None
                    else parent.get('provenance', {}).get('image_sha256')), 'Local receipt source mismatch')
        require(receipt['scale'] == 2 and receipt['status'] == 'ok', 'Unsupported local receipt transform')
        raw_rows = adapter.raw_boxes(receipt['provider'], receipt['raw'])
        require(len(raw_rows) == len(receipt['mapped']), 'Local raw detector count mismatch')
        roi = receipt['roi']['bbox']
        for i, (raw, mapped) in enumerate(zip(raw_rows, receipt['mapped'])):
            polygon = [[x / 2 + roi[0], y / 2 + roi[1]] for x, y in raw['polygon']]
            require(mapped['source_local_index'] == i and mapped['polygon'] == polygon
                    and mapped['bbox'] == adapter.envelope([[x,y,x,y] for x,y in polygon])
                    and mapped['confidence'] == raw['confidence'], 'Local raw detector mapping mismatch')
        if image_bytes is not None:
            with legacy.Image.open(io.BytesIO(image_bytes)) as source_image:
                crop = source_image.convert('RGB').crop(tuple(roi))
                crop = crop.resize((crop.width * 2, crop.height * 2))
                encoded = io.BytesIO()
                crop.save(encoded, format='PNG')
                hash_matches(encoded.getvalue(), receipt['crop_sha256'], 'Local detector input crop')
        receipts[path] = receipt
    for f in local.get('source_fragments', []):
        path, sep, index = f['source_pointer'].rpartition('#/mapped/')
        require(sep and path in receipts and index.isdigit(), 'Local source pointer mismatch')
        rows = receipts[path]['mapped']
        require(int(index) < len(rows), 'Local source pointer out of range')
        row = rows[int(index)]
        require(row['accepted'] is True and all(f[k] == row[k] for k in ('bbox', 'polygon', 'confidence'))
                and f['source_box_index'] == row['source_local_index'], 'Local source geometry mismatch')
        require(f['fragment_id'] not in source, 'Duplicate supplemental source ID')
        source[f['fragment_id']] = f
    fragments = {f['fragment_id']: f for f in parent['fragments']}
    require(len(fragments) == len(parent['fragments']), 'Duplicate parent fragments')
    for f in fragments.values():
        sid = f.get('source_fragment_id', f['fragment_id'])
        require(sid in source and f['source_pointer'] == source[sid]['source_pointer'],
                'Derived source mapping mismatch')
        if 'source_polygon' in f:
            require(f['source_polygon'] == source[sid]['polygon'], 'Derived source polygon mismatch')
    return fragments


def prepare(geometry_json, parent_bytes, spatial_bytes, image_bytes, expected, receipt_bytes=None):
    """Validate pinned artifacts and materialize the operational request once.

    receipt_bytes maps explicitly selected receipt paths to bytes; no path from
    submitted JSON is automatically opened. All checks survive python -O.
    """
    hash_matches(geometry_json.encode(), expected.contract_sha256, 'Contract')
    hash_matches(parent_bytes, expected.parent_artifact_sha256, 'Parent artifact')
    hash_matches(spatial_bytes, expected.spatial_artifact_sha256, 'Spatial artifact')
    hash_matches(image_bytes, expected.image_sha256, 'Image')
    wrapper = parse(geometry_json)
    parent, spatial = parse(parent_bytes), parse(spatial_bytes)
    require(wrapper['spatial'] == spatial, 'Contract/spatial artifact mismatch')
    require(wrapper['contract'] == legacy.CONTRACT and wrapper['image_sha256'] == expected.image_sha256,
            'Contract/image identity mismatch')
    require(spatial['parent_geometry_sha256'] == parent['geometry_sha256'] == expected.parent_geometry_sha256,
            'Expected parent mismatch')
    require(all(spatial[k] == parent[k] for k in ('page_id', 'provider', 'width', 'height')),
            'Parent page identity mismatch')
    if 'image_sha256' in parent.get('provenance', {}):
        require(parent['provenance']['image_sha256'] == expected.image_sha256, 'Parent image mismatch')
    fragments = verify_parent(parent, receipt_bytes or {}, image_bytes)
    self_hash(spatial, 'spatial_sha256')
    legacy.validate(spatial)
    owners = {fid: u['line_id'] for u in parent['logical_lines'] for fid in u['fragment_ids']}
    require(sum(len(u['fragment_ids']) for u in parent['logical_lines']) == len(owners) == len(fragments)
            and set(owners) == set(fragments), 'Parent membership mismatch')
    regions = [r for s in spatial['supports'] for r in s['regions']]
    require(len(regions) == len(fragments) and {r['fragment_id'] for r in regions} == set(fragments),
            'Spatial fragment coverage mismatch')
    for r in regions:
        f = fragments[r['fragment_id']]
        require(all(r[k] == f[k] for k in ('bbox', 'polygon', 'source_pointer', 'column_id', 'band_id'))
                and r['source_fragment_id'] == f.get('source_fragment_id', f['fragment_id'])
                and r['crop_parent_unit_id'] == owners[r['fragment_id']], 'Spatial source mapping mismatch')
    payload, inputs = legacy.payload(geometry_json, image_bytes)
    manifest = {'schema': MANIFEST, 'geometry_sha256': expected.contract_sha256,
                'parent_geometry_sha256': expected.parent_geometry_sha256,
                'parent_artifact_sha256': expected.parent_artifact_sha256,
                'spatial_artifact_sha256': expected.spatial_artifact_sha256,
                'spatial_sha256': spatial['spatial_sha256'], 'image_sha256': expected.image_sha256,
                'payload_sha256': sha(canonical(payload).encode()), 'inputs': inputs}
    manifest_json = canonical(manifest)
    result = PreparedSupportPage(geometry_json, manifest_json, canonical(payload), expected,
                                 sha(manifest_json.encode()))
    result.checked_payload()
    return result


def verify_payload(payload, manifest_json, expected_manifest_sha256):
    """Check a prepared manifest from an independent pin before dispatch."""
    hash_matches(manifest_json.encode(), expected_manifest_sha256, 'Input manifest')
    m = parse(manifest_json)
    require(m['schema'] == MANIFEST, 'Input manifest schema mismatch')
    hash_matches(canonical(payload).encode(), m['payload_sha256'], 'Materialized payload')
    parts = payload['contents'][0]['parts']
    require(len(parts) == 1 + 2 * len(m['inputs']), 'Materialized input count mismatch')
    for i, item in enumerate(m['inputs']):
        require(parts[1+2*i] == {'text': 'support_id='+item['support_id']}, 'Materialized input ID mismatch')
        data = parts[2+2*i]['inlineData']
        require(data['mimeType'] == 'image/png', 'Materialized MIME mismatch')
        hash_matches(base64.b64decode(data['data'], validate=True), item['png_sha256'], 'Materialized PNG')


def verify_semantic_sources(cases_bytes, expected_inputs_sha256, prepared_pages, context_bytes):
    """Offline Phase 3 gate; verifies source claims without changing historic calls.

    geometry_sha256 in these historical cases means the spatial FILE byte hash,
    not the canonical spatial or parent hash. Context images remain context only.
    """
    hash_matches(cases_bytes, expected_inputs_sha256, 'Semantic inputs')
    cases = parse(cases_bytes)
    for case in cases:
        page = prepared_pages[case['page_id']]
        g = parse(page.geometry_json)['spatial']
        require(case['geometry_sha256'] == page.expected.spatial_artifact_sha256
                and case['image_source_sha256'] == page.expected.image_sha256, 'Semantic source claim mismatch')
        units = {u['unit_id']: u for u in g['units']}
        require(case['units'] == [units[i] for i in case['unit_ids']], 'Semantic unit source mismatch')
        for im in case['images']:
            require(im['path'] in context_bytes, 'Missing selected context image')
            hash_matches(context_bytes[im['path']], im['sha256'], 'Semantic context image')
    return {'cases': len(cases), 'context_images': sum(len(c['images']) for c in cases),
            'inputs_sha256': expected_inputs_sha256, 'source_claims_verified': True,
            'transcription_or_semantic_gold': False}
