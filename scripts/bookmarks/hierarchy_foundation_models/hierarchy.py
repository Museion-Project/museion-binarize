"""Optional hierarchy-only projection of retained recovery evidence.

Gold is not an input to this module. Neither titles nor baseline hierarchy are
sent in the textual prompt. Failed/unsupported replies never change a group.
"""
import argparse
import copy
import json
from pathlib import Path
import re
import subprocess


def write_new(path, value):
    with Path(path).open('x') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)


def numbering_shape(label):
    if label is None or not str(label).strip():
        return 'none'
    label = str(label).strip().rstrip('.):')
    if re.fullmatch(r'\d+(?:\.\d+)*', label):
        return 'decimal_depth_' + str(label.count('.')+1)
    if re.fullmatch(r'[IVXLCDMivxlcdm]+', label):
        return 'roman'
    if len(label) == 1 and label.isalpha():
        return 'single_letter'
    return 'other_explicit_label'


def group_request(group, pages, image_paths, mode):
    if mode != 'image':
        raise ValueError('User requires source images; no feature-only mode')
    if not 1 <= len(group['entries']) <= 60:
        raise ValueError('MVP supports 1–60 fixed entries per group; no silent truncation')
    locations = {line['id']: (page, line) for page in pages for line in page['lines']}
    items = []
    for i, entry in enumerate(group['entries']):
        evidence = [locations[eid] for eid in entry['evidence_ids'] if eid in locations]
        if not evidence:
            raise ValueError(f'Missing measured support for entry {i}')
        fragments = []
        for page, line in evidence:
            b = line['bbox']; w, h = page['width'], page['height']
            fragments.append(dict(page=page['page_index']+1,
                                  box=[round(b['x']/w, 5), round(b['y']/h, 5),
                                       round(b['width']/w, 5), round(b['height']/h, 5)],
                                  text_start_x=round(line.get('title_start_x', b['x'])/w, 5),
                                  height_proxy_pt=round(line.get('size_proxy', b['height']), 3)))
        items.append(dict(index=i, fragments=fragments,
                          numbering_shape=numbering_shape(entry.get('numbering')),
                          has_printed_reference=entry.get('printed_page_ref') is not None))
    payload = dict(coordinates='normalized visible top-left xywh',
                   pages=[dict(page=p['page_index']+1, width=p['width'], height=p['height']) for p in pages],
                   entries=items)
    prompt = ('The fixed entries below are from one explicit TOC group. '
              'Use only these indices. Infer immediate parent and zero-based level for every index. '
              'No transcription is requested. Geometry is measured from retained observations; '
              'height_proxy_pt is an observed size proxy, not a guaranteed font point size. '
              'Numbering shape alone is not sufficient to establish hierarchy. '
              'Do not treat an indented continuation fragment as a new child. '
              'Cross-page continuation does not reset the hierarchy. '
              'Abstain if layout evidence is insufficient.\n' + json.dumps(payload, separators=(',', ':')))
    if mode == 'image':
        if len(image_paths) != len(pages):
            raise ValueError('One source image per evidence page required')
        prompt += '\nThe attached source pages are in the page order listed above. Inspect their typography and layout; do not output their text.'
    return dict(mode=mode, prompt=prompt, images=[str(Path(p).resolve()) for p in image_paths] if mode == 'image' else [])


def apply_relations(group, reply):
    """Validate an entire ordered tree before changing either hierarchy field."""
    if reply.get('status') != 'completed' or reply.get('supported') is not True:
        return copy.deepcopy(group), 'not_applied:' + str(reply.get('status', 'missing_status'))
    relations = reply.get('relations')
    if not isinstance(relations, list) or len(relations) != len(group['entries']):
        raise ValueError('Missing or extra relation')
    levels = []
    for i, relation in enumerate(relations):
        if not isinstance(relation, dict) or set(relation) != {'index', 'level', 'parent'}:
            raise ValueError('Unexpected relation fields')
        index, level, parent = (relation[k] for k in ('index', 'level', 'parent'))
        if any(type(v) is not int for v in (index, level, parent)) or index != i:
            raise ValueError('Duplicate, reordered or invalid entry index')
        if not (-1 <= parent < i) or not (0 <= level <= i):
            raise ValueError('Parent must precede child and levels must be nonnegative')
        if level != (0 if parent == -1 else levels[parent]+1):
            raise ValueError('Parent and level disagree')
        # A TOC tree in reading order cannot return to a closed subtree.
        if parent >= 0:
            active = i-1
            while active >= 0 and active != parent:
                active = relations[active]['parent']
            if active != parent:
                raise ValueError('Parent is in a closed subtree')
        levels.append(level)
    updated = copy.deepcopy(group)
    for entry, relation in zip(updated['entries'], relations):
        entry['level'] = relation['level']
        entry['parent_entry_id'] = None if relation['parent'] == -1 else updated['entries'][relation['parent']]['entry_id']
    return updated, 'applied'


def run_one(baseline_path, request_paths, worker, output, timeout=90):
    output = Path(output); output.mkdir(parents=True, exist_ok=False)
    baseline = json.loads(Path(baseline_path).read_text()); result = copy.deepcopy(baseline)
    if len(request_paths) != len(result['toc_groups']):
        raise ValueError('Request/group count mismatch')
    receipts = []
    for gi, request_path in enumerate(request_paths):
        request = json.loads(Path(request_path).read_text())
        write_new(output/f'group-{gi}-request.json', request)
        try:
            proc = subprocess.run([str(Path(worker).resolve())], input=json.dumps(request), text=True,
                                  capture_output=True, timeout=timeout, check=True)
            (output/f'group-{gi}-stdout.txt').write_text(proc.stdout)
            (output/f'group-{gi}-stderr.txt').write_text(proc.stderr)
            reply = json.loads(proc.stdout)
        except (subprocess.SubprocessError, json.JSONDecodeError, OSError) as error:
            reply = dict(status='transport_error', error=str(error))
        write_new(output/f'group-{gi}-response.json', reply)
        try:
            group, status = apply_relations(result['toc_groups'][gi], reply)
            result['toc_groups'][gi] = group
        except ValueError as error:
            status = 'rejected_invalid_tree:' + str(error)
        receipts.append(dict(group=gi, status=status, model_status=reply.get('status'),
                             model_error=reply.get('error'), seconds=reply.get('seconds')))
    write_new(output/'prediction.json', result)
    receipt = dict(status='completed' if all(r['status']=='applied' for r in receipts) else 'incomplete',
                   groups=receipts, baseline=str(Path(baseline_path).resolve()),
                   scope='Only level/parent may change; unresolved groups retain baseline, never counted as model success')
    write_new(output/'receipt.json', receipt)
    return receipt


if __name__ == '__main__':
    p = argparse.ArgumentParser(); p.add_argument('baseline'); p.add_argument('output')
    p.add_argument('--worker', required=True); p.add_argument('--request', action='append', required=True)
    p.add_argument('--timeout', type=int, default=90); a = p.parse_args()
    print(json.dumps(run_one(a.baseline, a.request, a.worker, a.output, a.timeout)))
