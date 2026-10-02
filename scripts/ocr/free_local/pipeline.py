"""Opt-in, offline basic searchable OCR. Original observations are immutable evidence."""
from __future__ import annotations

import argparse
import collections
import csv
import hashlib
import html
import io
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import time
import unicodedata

import fitz

ROOT = Path(__file__).resolve().parents[3]
FONT = ROOT / 'crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')


def greek(text):
    return sum('GREEK' in unicodedata.name(c, '') and c.isalpha() for c in text)


def base(text):
    return ''.join(c.lower() for c in unicodedata.normalize('NFD', text)
                   if not unicodedata.combining(c) and c.isalnum())


def area(box):
    return max(0, box[2]-box[0]) * max(0, box[3]-box[1])


def intersection(a, b):
    return max(0, min(a[2], b[2])-max(a[0], b[0])) * max(0, min(a[3], b[3])-max(a[1], b[1]))


def overlap(a, b):
    return intersection(a, b) / max(1, min(area(a), area(b)))


def apple_words(raw, width, height):
    words = []
    for obs in raw['observations']:
        tokens = obs.get('tokens') or [{'text': obs['text'], 'bbox': obs['bbox']}]
        for n, t in enumerate(tokens):
            x, y, w, h = t['bbox']
            words.append({'id': f"{obs['id']}-w{n}", 'text': t['text'],
                          'bbox': [x*width, (1-y-h)*height, (x+w)*width, (1-y)*height],
                          'confidence': obs['confidence'], 'engine': 'apple', 'line_id': obs['id']})
    return words


def tess_words(tsv):
    words = []
    for row in csv.DictReader(io.StringIO(tsv), delimiter='\t', quoting=csv.QUOTE_NONE):
        if row['level'] != '5' or not row.get('text', '').strip():
            continue
        x, y, w, h = [int(row[k]) for k in ['left', 'top', 'width', 'height']]
        words.append({'id': f'tess-w{len(words)}', 'text': row['text'], 'bbox': [x,y,x+w,y+h],
                      'confidence': float(row['conf']), 'engine': 'tesseract',
                      'line_id': '-'.join(row[k] for k in ['block_num','par_num','line_num'])})
    return words


def compose(original, independent):
    """Derived reading view; never mutate raw Apple. Use real reader word boxes only."""
    removed, extras, decisions = set(), [], []
    for word in independent:
        candidates = [a for a in original if overlap(a['bbox'], word['bbox']) >= .45]
        g = greek(word['text'])
        latin = sum(c.isalpha() and 'LATIN' in unicodedata.name(c, '') for c in word['text'])
        greek_neighbor = any(t['line_id']==word['line_id'] and greek(t['text'])>=2
                             for t in independent)
        greek_candidate = (g >= 2 or (g >= 1 and greek_neighbor)) and latin <= g*.25 and word['confidence'] >= 30
        if greek_candidate:
            # A broad OCR box may contain multiple words: do not silently erase that line.
            unsafe = any(area(a['bbox']) > area(word['bbox'])*3 for a in candidates)
            duplicate = any(overlap(word['bbox'], e['bbox']) >= .5 for e in extras)
            if unsafe or duplicate:
                decisions.append({'reader_id': word['id'], 'state': 'REVIEW', 'reason': 'ambiguous_overlap'})
                continue
            removed.update(a['id'] for a in candidates)
            extras.append(dict(word, review=True, source_members=[a['id'] for a in candidates]))
            decisions.append({'reader_id': word['id'], 'state': 'GREEK_DRAFT',
                              'source_members': [a['id'] for a in candidates]})
        elif not candidates and word['confidence'] >= 65 and base(word['text']):
            # Require separation from all original/new words, not just the enrolled subset.
            if any(intersection(word['bbox'], a['bbox']) > 0 for a in original+extras):
                decisions.append({'reader_id': word['id'], 'state': 'REVIEW', 'reason': 'partial_overlap'})
                continue
            extras.append(dict(word, review=True, source_members=[]))
            decisions.append({'reader_id': word['id'], 'state': 'RESIDUAL_DRAFT', 'source_members': []})
    words = [dict(a, review=False) for a in original if a['id'] not in removed] + extras
    # Rows are a convenience projection, not certified multi-column reading order.
    words.sort(key=lambda a: ((a['bbox'][1]+a['bbox'][3])/2, a['bbox'][0]))
    return words, decisions


def text_lines(words):
    rows = []
    for w in sorted(words, key=lambda a: ((a['bbox'][1]+a['bbox'][3])/2, a['bbox'][0])):
        cy = (w['bbox'][1]+w['bbox'][3])/2
        height = w['bbox'][3]-w['bbox'][1]
        if rows and abs(cy-rows[-1]['cy']) < .45*min(height, rows[-1]['height']):
            rows[-1]['words'].append(w)
        else:
            rows.append({'cy': cy, 'height': height, 'words': [w]})
    return [' '.join(w['text'] for w in sorted(row['words'], key=lambda a:a['bbox'][0])) for row in rows]


def native_text(page):
    text = page.get_text()
    blocks = page.get_text('dict')['blocks']
    visible = sum(len(s['text'].strip()) for b in blocks if b['type']==0
                  for l in b['lines'] for s in l['spans'])
    bad = any(c == '\ufffd' or unicodedata.category(c) == 'Co' for c in text)
    # Usable candidate, not a source-collated certificate. Sparse/defective pages run OCR.
    visible_trace = sum(len(s['chars']) for s in page.get_texttrace() if s['type'] != 3)
    scan_image = any(area(b['bbox']) > page.rect.width*page.rect.height*.5 for b in blocks if b['type']==1)
    return text if visible >= 80 and visible_trace >= 80 and not bad and not scan_image else None


def insert_hidden(page, words, width, height):
    font = fitz.Font(fontfile=str(FONT))
    missing = sorted({c for w in words for c in w['text'] if not font.has_glyph(ord(c))})
    if missing:
        raise ValueError(f'font cannot encode {missing!r}')
    sx, sy = page.rect.width/width, page.rect.height/height
    for w in words:
        x, y, r, b = w['bbox']
        rect = fitz.Rect(x*sx,y*sy,r*sx,b*sy)
        text = w['text']
        fs = max(1, rect.height / (font.ascender-font.descender))
        tw = fitz.TextWriter(page.rect)
        # No invented word geometry: a horizontal transform fits the observed box.
        natural = font.text_length(text, fontsize=fs)
        pt = fitz.Point(rect.x0, rect.y0+font.ascender*fs)
        # Explicit separator prevents readers from joining adjacent independently positioned words.
        tw.append(pt, text+' ', font=font, fontsize=fs)
        tw.write_text(page, render_mode=3, morph=(pt, fitz.Matrix(rect.width/max(.001,natural),1)))


def review_html(pages):
    cards = []
    for p in pages:
        lines = '\n'.join(text_lines(p.get('words', []))) if p['route']=='ocr' else p.get('native_text','')
        cards.append(f'<section><h2>第 {p["page"]} 页 · {html.escape(p["status"])}</h2>'
                     f'<p>{html.escape(", ".join(p.get("review_reasons", [])))}</p>'
                     f'<div><img src="{html.escape(p["image"])}"><textarea spellcheck="false">'
                     f'{html.escape(lines)}</textarea></div></section>')
    return '''<!doctype html><meta charset="utf-8"><title>免费 OCR 草稿复核</title>
<style>body{font:16px system-ui;margin:24px;max-width:1500px}div{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}textarea{width:100%;min-height:650px;font:16px/1.7 serif}section{border-top:1px solid #ccc;margin:24px 0}button{padding:10px}</style>
<h1>免费 OCR 草稿复核</h1><p>机器草稿。希腊文、补充文字与多栏顺序需复核。修改仅保存在导出的文本中；PDF仍是原机器草稿。</p>
<button onclick="download()">下载修改后的文本</button>''' + ''.join(cards) + '''
<script>function download(){let t=[...document.querySelectorAll('textarea')].map((x,i)=>'第 '+(i+1)+' 页\\n'+x.value).join('\\n\\n');let a=document.createElement('a');a.href=URL.createObjectURL(new Blob([t],{type:'text/plain;charset=utf-8'}));a.download='reviewed-text.txt';a.click();URL.revokeObjectURL(a.href)}</script>'''


def run(source, out, helper, force=False, psm=3):
    source, out = Path(source).resolve(), Path(out).resolve()
    if out.exists():
        raise FileExistsError('Use a new output directory; existing output is never overwritten')
    source_hash = sha(source)
    doc = fitz.open(source)
    if doc.is_encrypted:
        raise ValueError('Encrypted PDF requires an unlocked local copy')
    if force and any(native_text(p) is not None for p in doc):
        raise ValueError('Force OCR requires image-only input; native text would duplicate the new layer')
    out.mkdir(parents=True)
    (out/'raw').mkdir()
    pages, started, output = [], time.monotonic(), fitz.open()
    output.insert_pdf(doc)
    for idx, page in enumerate(doc):
        start = time.monotonic()
        record = {'page': idx+1, 'route': 'ocr', 'status': 'FAILED', 'review_reasons': [],
                  'source_sha256': source_hash, 'image': f'page-{idx+1:04}.png'}
        try:
            pix = page.get_pixmap(dpi=300, alpha=False)
            image = out/record['image']; pix.save(image)
            record.update(width=pix.width, height=pix.height, image_sha256=sha(image))
            native = None if force else native_text(page)
            if native is not None:
                record.update(route='native', status='NATIVE_PRESERVED', native_text=native,
                              review_reasons=['native text preserved; not source-collated'])
            elif page.get_text().strip():
                record.update(route='existing', status='EXISTING_TEXT_REVIEW', native_text=page.get_text(),
                              review_reasons=['existing text is not a reliable native candidate; page kept without a duplicate OCR layer'])
            elif page.rotation:
                record.update(status='ROTATED_REVIEW', review_reasons=['rotated scan requires normalization'])
            else:
                inp=out/'raw'/f'p{idx+1}-input.json'; rawdir=out/'raw'/f'p{idx+1}-apple'
                save(inp,[{'id':'page','image_path':str(image)}])
                stage=time.monotonic()
                result=subprocess.run([str(helper),str(inp),str(rawdir)],capture_output=True,text=True,timeout=60)
                (out/'raw'/f'p{idx+1}-apple.stderr').write_text(result.stderr)
                if result.returncode:
                    raise RuntimeError('Apple helper failed; see raw stderr')
                raw=json.loads((rawdir/'page.json').read_text())
                if raw['state']!='locally_recognized':
                    raise RuntimeError('Apple inference failed: '+raw.get('error','unknown'))
                original=apple_words(raw,pix.width,pix.height)
                record['original_apple']=original
                record['apple_seconds']=time.monotonic()-stage
                stage=time.monotonic()
                result=subprocess.run(['tesseract',str(image),'stdout','-l','grc+eng','--oem','1','--psm',str(psm),'tsv'],
                                      capture_output=True,text=True,timeout=60,
                                      env=dict(os.environ,OMP_THREAD_LIMIT='4'))
                (out/'raw'/f'p{idx+1}-tesseract.tsv').write_text(result.stdout)
                (out/'raw'/f'p{idx+1}-tesseract.stderr').write_text(result.stderr)
                if result.returncode:
                    raise RuntimeError('Independent reader failed; raw Apple retained')
                independent=tess_words(result.stdout)
                record['reader_seconds']=time.monotonic()-stage
                words,decisions=compose(original,independent)
                record.update(original_apple=original,independent_reader=independent,words=words,decisions=decisions,
                              status='OCR_DRAFT' if words else 'EMPTY',
                              review_reasons=['Greek and supplements are machine drafts',
                                              'complex page reading order is unverified'])
                insert_hidden(output[idx],words,pix.width,pix.height)
        except Exception as exc:
            record.update(status='FAILED',error=repr(exc),review_reasons=['page kept; OCR failed'])
        record['wall_seconds']=time.monotonic()-start
        pages.append(record);save(out/'pages.json',pages)
        print(f"page {idx+1}/{len(doc)} {record['status']} {record['wall_seconds']:.2f}s",flush=True)
    output.save(out/'searchable.pdf',garbage=3,deflate=True)
    output.close()
    text='\n\n'.join(f"--- Page {p['page']} [{p['status']}] ---\n"+
                     (p.get('native_text','') if p['route']!='ocr' else '\n'.join(text_lines(p.get('words',[]))))
                     for p in pages)
    (out/'text.txt').write_text(text,encoding='utf-8')
    (out/'review.html').write_text(review_html(pages),encoding='utf-8')
    assert sha(source)==source_hash,'source changed'
    elapsed=time.monotonic()-started
    save(out/'run.json',{'schema':'free-local-ocr/1','source':str(source),'source_sha256':source_hash,
                         'pages':len(pages),'statuses':dict(collections.Counter(p['status'] for p in pages)),
                         'wall_seconds':elapsed,'cloud_calls':0,'downloads':0,'psm':psm,
                         'pipeline_sha256':sha(__file__),'apple_helper_sha256':sha(helper),
                         'tesseract_version':subprocess.run(['tesseract','--version'],capture_output=True,text=True).stdout.splitlines()[0],
                         'draft':True,'fullpage_order_certified':False})
    return pages


def main():
    parser=argparse.ArgumentParser(description='Offline basic OCR: PDF -> searchable PDF + reviewable text/JSON')
    parser.add_argument('pdf');parser.add_argument('output')
    parser.add_argument('--apple-helper',type=Path,required=True)
    parser.add_argument('--force-ocr',action='store_true',help='Only use on image-only PDFs to avoid duplicate native text')
    parser.add_argument('--psm',type=int,default=3,choices=[3,6,11])
    args=parser.parse_args()
    if not args.apple_helper.is_file() or not shutil.which('tesseract'):
        parser.error('Local Apple helper and Tesseract are required; no automatic downloads')
    def denied(*a,**kw):raise OSError('free OCR network disabled')
    socket.create_connection=denied;socket.socket.connect=denied;socket.getaddrinfo=denied
    run(args.pdf,args.output,args.apple_helper,args.force_ocr,args.psm)


if __name__=='__main__':
    main()
