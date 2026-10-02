#!/usr/bin/env python3
"""Local geometry-only development comparison. Never reads holdout assets."""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import statistics
import time
import traceback

from run_geometry_bakeoff import bbox_iou, match_boxes, tesseract_lines

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'docs/evidence/geometry-dev-bakeoff-2026-09-06'
MANIFEST = OUT / 'reference-manifest.json'
TESSDATA = ROOT / 'target/distribution/ocr-build/models'
SURYA = Path('/Users/theo/Library/Caches/datalab/models/text_detection/2025_05_07')
PADDLE = Path('/Users/theo/.paddlex/official_models/PP-OCRv5_server_det')
LAYOUT = Path('/private/tmp/mpdf-geometry-dev-2026-09-06/paddlex/official_models/PP-DocLayout_plus-L')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')


def verify(manifest):
    assert manifest['schema'] == 'mpdf-verified-dev-geometry/1'
    assert len(manifest['pages']) == 25
    assert len({p['page_id'] for p in manifest['pages']}) == 25
    assert sum(len(p['lines']) for p in manifest['pages']) == 1378
    for p in manifest['pages']:
        assert sha(p['image_path']) == p['image_sha256'], p['page_id']
        assert sha(p['reference_path']) == p['reference_sha256'], p['page_id']


def xy_order(items, scale=None):
    if len(items) < 2:
        return items[:]
    if scale is None:
        scale = statistics.median(x['bbox'][3] - x['bbox'][1] for x in items)
    for axis, minimum in ((0, 1.5 * scale), (1, 0.5 * scale)):
        intervals = sorted((x['bbox'][axis], x['bbox'][axis+2]) for x in items)
        end = intervals[0][1]
        gaps = []
        for start, stop in intervals[1:]:
            if start - end >= minimum:
                gaps.append((start-end, (start+end)/2))
            end = max(end, stop)
        if gaps:
            _, cut = sorted(gaps, key=lambda v: (-v[0], v[1]))[0]
            left = [x for x in items if x['bbox'][axis+2] <= cut]
            right = [x for x in items if x['bbox'][axis] >= cut]
            if left and right and len(left)+len(right) == len(items):
                return xy_order(left, scale) + xy_order(right, scale)
    return sorted(items, key=lambda x: (x['bbox'][1], x['bbox'][0]))


def area(box):
    return (box[2]-box[0])*(box[3]-box[1])


def layout_order(lines, regions):
    if not lines:
        return []
    groups = {}
    for line in lines:
        b = line['bbox']; cx, cy = (b[0]+b[2])/2, (b[1]+b[3])/2
        eligible = [(area(r['coordinate']), i) for i, r in enumerate(regions)
                    if r['coordinate'][0] <= cx <= r['coordinate'][2]
                    and r['coordinate'][1] <= cy <= r['coordinate'][3]
                    and bbox_iou(b, r['coordinate'])[1] >= 0.5]
        key = min(eligible)[1] if eligible else ('unassigned', len(groups))
        groups.setdefault(key, []).append(line)
    blocks = []
    for group in groups.values():
        blocks.append({'bbox': [min(x['bbox'][0] for x in group),
                                min(x['bbox'][1] for x in group),
                                max(x['bbox'][2] for x in group),
                                max(x['bbox'][3] for x in group)], 'lines': group})
    scale = statistics.median(x['bbox'][3]-x['bbox'][1] for x in lines)
    return [line for block in xy_order(blocks, scale) for line in xy_order(block['lines'], scale)]


def envelope(polygon):
    return [float(min(p[0] for p in polygon)), float(min(p[1] for p in polygon)),
            float(max(p[0] for p in polygon)), float(max(p[1] for p in polygon))]


def setup(name):
    if name == 'tesseract':
        return lambda p: {'lines': tesseract_lines(Path(p), Path('/opt/homebrew/bin/tesseract'), TESSDATA, 3)}
    if name == 'surya':
        import torch
        from PIL import Image
        from surya.detection import DetectionPredictor
        torch.set_num_threads(4)
        predictor = DetectionPredictor(checkpoint=str(SURYA), device='cpu', dtype=torch.float32)
        def predict(p):
            with Image.open(p) as image:
                result = predictor([image.convert('RGB')], batch_size=1)[0]
            lines = [{'bbox': [float(v) for v in b.bbox]} for b in result.bboxes]
            return {'lines': xy_order(lines), 'native_lines': lines}
        return predict
    from paddleocr import TextDetection, LayoutDetection
    detector = TextDetection(model_dir=str(PADDLE), device='cpu', enable_mkldnn=False, cpu_threads=4)
    layout = LayoutDetection(model_dir=str(LAYOUT), device='cpu', enable_mkldnn=False, cpu_threads=4)
    def predict(p):
        dt = list(detector.predict(p))[0].json['res']
        lr = list(layout.predict(p))[0].json['res']
        lines = [{'bbox': envelope(poly)} for poly in dt['dt_polys']]
        regions = lr['boxes']
        return {'lines': layout_order(lines, regions), 'native_lines': lines, 'layout_regions': regions}
    return predict


def model_hashes(name):
    paths = {'tesseract': [TESSDATA], 'surya': [SURYA], 'paddle': [PADDLE, LAYOUT]}[name]
    return {str(f): sha(f) for path in paths for f in sorted(path.rglob('*')) if f.is_file() and f.suffix in {'.json', '.yml', '.yaml', '.safetensors', '.pdiparams', '.traineddata'}}


def run(name):
    manifest = json.loads(MANIFEST.read_text()); verify(manifest)
    os.environ.update(OMP_NUM_THREADS='4', OPENBLAS_NUM_THREADS='4', VECLIB_MAXIMUM_THREADS='4',
                      TOKENIZERS_PARALLELISM='false', MPLCONFIGDIR='/private/tmp/mpdf-geometry-dev-2026-09-06/mpl',
                      PADDLE_PDX_CACHE_HOME='/private/tmp/mpdf-geometry-dev-2026-09-06/paddlex')
    rawdir = OUT / 'raw' / name; rawdir.mkdir(parents=True, exist_ok=True)
    assert not (OUT / f'{name}-run.json').exists(), 'Use a new evidence directory for a rerun'
    info = {'candidate': name, 'device': 'cpu', 'platform': platform.platform(), 'python': platform.python_version(),
            'manifest_sha256': sha(MANIFEST), 'protocol_sha256': sha(OUT/'protocol.md'), 'runner_sha256': sha(__file__),
            'versions': {p: importlib.metadata.version(p) for p in ['surya-ocr','paddleocr','paddlepaddle','torch','transformers']},
            'model_sha256': model_hashes(name), 'pages': []}
    started = time.perf_counter(); runner = setup(name)
    info['initialization_seconds'] = time.perf_counter()-started
    print(f'{name}: initialized in {info["initialization_seconds"]:.2f}s', flush=True)
    for attempt in (1, 2):
        for page in manifest['pages']:
            item = {'page_id': page['page_id'], 'attempt': attempt}
            t = time.perf_counter()
            try:
                payload = runner(page['image_path'])
                item['latency_seconds'] = time.perf_counter()-t
                for line in payload['lines']:
                    b = line['bbox']
                    assert len(b) == 4 and all(isinstance(v, (int,float)) for v in b)
                    assert 0 <= b[0] < b[2] <= page['width'] and 0 <= b[1] < b[3] <= page['height'], b
                item.update(payload); item['status'] = 'ok'
            except Exception:
                item.update(status='error', error=traceback.format_exc(), lines=[], latency_seconds=time.perf_counter()-t)
            target = rawdir / f'{page["page_id"]}-pass{attempt}.json'
            write(target, item)
            info['pages'].append({'page_id': item['page_id'], 'attempt': attempt, 'status': item['status'],
                                  'file': str(target.relative_to(OUT)), 'sha256': sha(target), 'latency_seconds': item['latency_seconds']})
            write(OUT / f'{name}-run.json', info)
            print(f'{name} pass{attempt} {page["page_id"]}: {item["status"]}, {len(item["lines"])} lines, {item["latency_seconds"]:.2f}s', flush=True)
    verify(manifest); info['input_integrity_after'] = 'PASS'; write(OUT/f'{name}-run.json', info)


def metrics(gold, lines):
    matches = match_boxes(gold, lines)
    order = [m['gold'] for m in sorted(matches, key=lambda m: m['candidate'])]
    pairs = len(order)*(len(order)-1)//2
    correct = sum(order[i] < order[j] for i in range(len(order)) for j in range(i+1,len(order)))
    return {'gold': len(gold), 'predicted': len(lines), 'matched': len(matches),
            'matched_at_50': len(match_boxes(gold, lines, .5)),
            'iou_sum': sum(m['iou'] for m in matches), 'coverage_sum': sum(m['gold_coverage'] for m in matches),
            'pairs': pairs, 'correct_pairs': correct, 'gold_pairs': len(gold)*(len(gold)-1)//2}


def summarize(rows):
    counts = {k: sum(r[k] for r in rows) for k in rows[0]}
    g, p, m = counts['gold'], counts['predicted'], counts['matched']
    return {**counts, 'precision': m/p if p else 0, 'recall': m/g if g else 0,
            'f1': 2*m/(g+p) if g+p else 0, 'f1_at_50': 2*counts['matched_at_50']/(g+p) if g+p else 0,
            'mean_iou': counts['iou_sum']/m if m else 0, 'matched_gold_coverage': counts['coverage_sum']/m if m else 0,
            'all_gold_coverage': counts['coverage_sum']/g if g else 0,
            'order_accuracy': counts['correct_pairs']/counts['pairs'] if counts['pairs'] else None,
            'pair_coverage': counts['pairs']/counts['gold_pairs'] if counts['gold_pairs'] else 0}


def score():
    manifest = json.loads(MANIFEST.read_text()); verify(manifest)
    report = {'schema': 'mpdf-dev-geometry-bakeoff/1', 'manifest_sha256': sha(MANIFEST), 'candidates': {}}
    for name in ['tesseract', 'surya', 'paddle']:
        info = json.loads((OUT/f'{name}-run.json').read_text())
        assert info['input_integrity_after'] == 'PASS' and info['manifest_sha256'] == sha(MANIFEST)
        assert info['protocol_sha256'] == sha(OUT/'protocol.md') and info['runner_sha256'] == sha(__file__)
        assert len(info['pages']) == 50
        for item in info['pages']:
            assert sha(OUT/item['file']) == item['sha256']
        rows, common, pages, stable, rounded_stable = [], [], [], 0, 0
        for p in manifest['pages']:
            gold = sorted(p['lines'], key=lambda x: x['reading_order'])
            a, b = [json.loads((OUT/'raw'/name/f'{p["page_id"]}-pass{i}.json').read_text()) for i in (1,2)]
            r = metrics(gold, a['lines']); rows.append(r)
            common.append(metrics(gold, xy_order(a['lines'])))
            exact = a['status'] == b['status'] == 'ok' and a['lines'] == b['lines']
            rounded = a['status'] == b['status'] == 'ok' and [[round(v,3) for v in x['bbox']] for x in a['lines']] == [[round(v,3) for v in x['bbox']] for x in b['lines']]
            stable += exact; rounded_stable += rounded
            pages.append({'page_id': p['page_id'], **summarize([r]), 'exact_repeat': exact, 'rounded_repeat': rounded,
                          'pass1_status': a['status'], 'pass2_status': b['status']})
        durations = sorted(x['latency_seconds'] for x in info['pages'] if x['status']=='ok')
        failures = [x for x in info['pages'] if x['status']!='ok']
        report['candidates'][name] = {'primary': summarize(rows), 'common_xy_order': summarize(common), 'pages': pages,
             'exact_repeat_pages': stable, 'rounded_repeat_pages': rounded_stable, 'failures': failures,
             'initialization_seconds': info['initialization_seconds'], 'median_seconds': statistics.median(durations),
             'p95_seconds': durations[int(.95*len(durations)+.999999)-1], 'max_seconds': max(durations)}
    eligible = [(n,c) for n,c in report['candidates'].items() if not c['failures']]
    ranking = sorted(eligible, key=lambda nc: (-nc[1]['primary']['f1'], -nc[1]['primary']['mean_iou'],
                            -(nc[1]['primary']['order_accuracy'] or 0), nc[1]['median_seconds']))
    report['ranking'] = [n for n,c in ranking]
    report['winner'] = ranking[0][0] if ranking else None
    write(OUT/'results.json', report)
    print(json.dumps({n: {k:v for k,v in c.items() if k!='pages'} for n,c in report['candidates'].items()}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('action', choices=['tesseract','surya','paddle','score'])
    action = parser.parse_args().action
    score() if action == 'score' else run(action)
