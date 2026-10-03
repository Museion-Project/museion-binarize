"""Bind an explicit, frozen source-content record to an inspected runtime.

This revalidates record/package identity. It does not replay upstream byte
predictions, authenticate publishers, or grant installation/legal admission.
"""
from collections import Counter
import hashlib
import json
from pathlib import Path, PurePosixPath
import re

PREFIX = 'Contents/Resources/local-ocr/'
SCHEMAS = {'current-App-public-artifact-content-reference-SBOM/' + str(v) for v in (3, 4, 5)}
PUBLIC_KINDS = {
    'EXACT_PUBLIC_ARTIFACT_MEMBER_TO_RECORDED_INPUT_AND_CURRENT_APP',
    'EXACT_DETERMINISTIC_PUBLIC_BOTTLE_TRANSFORM_TO_RECORDED_INPUT_AND_CURRENT_APP',
}


def valid_sha(value):
    return isinstance(value, str) and re.fullmatch(r'[0-9a-f]{64}', value) is not None


def bind_native_content_record(record_file, expected_record_sha256, native_objects):
    path = Path(record_file).absolute()
    if (not valid_sha(expected_record_sha256) or path.is_symlink() or
            any(p.is_symlink() for p in path.parents) or not path.is_file()):
        raise ValueError('NATIVE_CONTENT_RECORD_IDENTITY')
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected_record_sha256:
        raise ValueError('NATIVE_CONTENT_RECORD_HASH')
    data = json.loads(raw)
    if data.get('schema') not in SCHEMAS or not isinstance(data.get('objects'), list):
        raise ValueError('NATIVE_CONTENT_RECORD_SCHEMA')
    actual = {x['path']: x['sha256'] for x in native_objects}
    if len(actual) != len(native_objects):
        raise ValueError('NATIVE_CONTENT_PACKAGE_DUPLICATE')
    selected = {}
    seen = set()
    outside = []
    for obj in data['objects']:
        name = obj.get('path')
        if (not isinstance(name, str) or not name or '\\' in name or
                str(PurePosixPath(name)) != name or PurePosixPath(name).is_absolute() or
                '..' in PurePosixPath(name).parts):
            raise ValueError('NATIVE_CONTENT_RECORD_PATH')
        if name in seen:
            raise ValueError('NATIVE_CONTENT_RECORD_DUPLICATE')
        seen.add(name)
        if not name.startswith(PREFIX):
            outside.append(name)
            continue
        target = name[len(PREFIX):]
        if target not in actual or obj.get('sha256') != actual[target]:
            raise ValueError('NATIVE_CONTENT_PACKAGE_IDENTITY')
        kind = obj.get('source_reference_kind')
        graph = obj.get('current_local_transform_graph')
        public = kind in PUBLIC_KINDS
        if public:
            if not isinstance(graph, dict):
                raise ValueError('NATIVE_CONTENT_RECORDED_CHAIN')
            chain = graph.get('content_hash_path')
            if (obj.get('upstream_content_correspondence_verified') is not True or
                    not isinstance(chain, list) or not chain or
                    not all(valid_sha(h) for h in chain) or
                    chain[0] != graph.get('original_input_sha256') or
                    chain[-1] != obj['sha256'] or graph.get('current_App_sha256') != obj['sha256']):
                raise ValueError('NATIVE_CONTENT_RECORDED_CHAIN')
        selected[target] = dict(record_object_path=name, recorded_source_reference_kind=kind,
            recorded_public_content_correspondence=public,
            recorded_original_input_sha256=graph.get('original_input_sha256') if isinstance(graph, dict) else None,
            original_build_install_event='UNKNOWN', package_sha256=actual[target],
            package_hash_bound=True, upstream_prediction_replayed=False,
            publisher_attestation_verified=False, legal_admission=False)
    if set(selected) != set(actual):
        raise ValueError('NATIVE_CONTENT_SCOPE_COVERAGE')
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected_record_sha256:
        raise ValueError('NATIVE_CONTENT_RECORD_CHANGED_DURING_INSPECTION')
    return dict(schema='runtime-native-content-record-binding/1',
        state='HASH_BOUND_RECORDED_CONTENT_AND_PACKAGE', record_path=str(path.resolve()),
        record_sha256=expected_record_sha256, record_schema=data['schema'], scope_prefix=PREFIX,
        scope_native_objects=len(selected), recorded_public_content_correspondences=sum(
            x['recorded_public_content_correspondence'] for x in selected.values()),
        recorded_source_kinds=dict(Counter(x['recorded_source_reference_kind'] for x in selected.values())),
        objects=selected, outside_scope_record_paths=outside,
        upstream_prediction_replayed=False, publisher_attestation_verified=False,
        original_build_install_events_verified=False, legal_admission=False,
        quality_ready=False, app_admission=False, distribution_ready=False, release_ready=False)
