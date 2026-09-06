#!/usr/bin/env python3
"""Build a self-contained viewer from recorded development geometry only."""
import base64
import json
from pathlib import Path

from run_dev_geometry_bakeoff import MANIFEST, OUT, match_boxes, verify


def main():
    manifest = json.loads(MANIFEST.read_text()); verify(manifest)
    results = json.loads((OUT/'results.json').read_text())
    pages = []
    for page in manifest['pages']:
        gold = sorted(page['lines'], key=lambda x: x['reading_order'])
        item = {'id': page['page_id'], 'width': page['width'], 'height': page['height'],
                'image': 'data:image/png;base64,' + base64.b64encode(Path(page['image_path']).read_bytes()).decode(),
                'gold': [x['bbox'] for x in gold], 'candidates': {}}
        for name, candidate in results['candidates'].items():
            raw = json.loads((OUT/'raw'/name/f'{page["page_id"]}-pass1.json').read_text())
            matches = match_boxes(gold, raw['lines'])
            item['candidates'][name] = {'boxes': [x['bbox'] for x in raw['lines']], 'matches': matches,
                'metrics': next(p for p in candidate['pages'] if p['page_id']==page['page_id'])}
        pages.append(item)
    data = json.dumps({'winner': results['winner'], 'pages': pages}, separators=(',', ':'))
    html = '''<!doctype html><html lang="zh"><meta charset="utf-8">
<title>Geometry development bake-off · 25 pages</title>
<style>
body{margin:0;background:#f4f4f0;color:#202c32;font:15px system-ui,sans-serif}
header{position:sticky;top:0;background:#fffffff2;padding:14px 22px;z-index:2;border-bottom:1px solid #cbd3d4}
h1{font-size:20px;margin:0 0 8px}select,button{padding:6px;margin:3px;font:inherit}label{margin:0 8px;white-space:nowrap}
#metrics{margin:8px 0 0;font-variant-numeric:tabular-nums}main{padding:16px;max-width:1600px;margin:auto}
svg{width:100%;background:white;box-shadow:0 2px 8px #0002}p{margin:6px 0}.note{font-size:13px;color:#536267}
</style><header><h1>Geometry provider development bake-off</h1>
<div><button id="prev">←</button><select id="page"></select><button id="next">→</button>
<select id="engine"><option value="tesseract">Tesseract PSM 3</option><option value="surya">Surya + XY-cut</option><option value="paddle">Paddle detection + layout</option></select></div>
<label><input type="checkbox" id="gold" checked>参考框（蓝）</label>
<label><input type="checkbox" id="pred" checked>候选框（橙；未匹配红）</label>
<label><input type="checkbox" id="numbers">显示阅读顺序（从 1 开始）</label>
<p id="metrics"></p><p class="note">25 页 development · 第一遍输出 · 一对一 IoU ≥ 0.30 · 红框表示未匹配，不等同于图像中无文字。排序指标仅计算已匹配行对。未运行 frozen holdout 或 production E2E。</p></header>
<main><svg id="canvas" xmlns="http://www.w3.org/2000/svg"></svg></main>
<script id="data" type="application/json">DATA_PLACEHOLDER</script>
<script>
const data=JSON.parse(document.getElementById('data').textContent), $=id=>document.getElementById(id);
data.pages.forEach((p,i)=>{const o=document.createElement('option');o.value=i;o.textContent=`${i+1}/25 · ${p.id}`;$('page').appendChild(o)});
$('engine').value=data.winner;
function node(tag,attrs){const e=document.createElementNS('http://www.w3.org/2000/svg',tag);for(const [k,v] of Object.entries(attrs))e.setAttribute(k,v);return e}
function boxes(list,color,matched){list.forEach((b,i)=>{const c=matched&&!matched.has(i)?'#d02032':color;
const r=node('rect',{x:b[0],y:b[1],width:b[2]-b[0],height:b[3]-b[1],fill:'none',stroke:c,'stroke-width':2.5});
const title=node('title',{});title.textContent=`${i+1}: [${b.map(x=>x.toFixed(1)).join(', ')}]`;r.appendChild(title);$('canvas').appendChild(r);
if($('numbers').checked){const t=node('text',{x:b[0],y:Math.max(18,b[1]-3),fill:c,'font-size':22,'font-weight':700,'paint-order':'stroke',stroke:'white','stroke-width':3});t.textContent=i+1;$('canvas').appendChild(t)}})}
const pct=x=>x===null?'N/A':(100*x).toFixed(2)+'%';
function render(){const p=data.pages[+$('page').value],c=p.candidates[$('engine').value],m=c.metrics;
$('canvas').replaceChildren();$('canvas').setAttribute('viewBox',`0 0 ${p.width} ${p.height}`);
$('canvas').appendChild(node('image',{href:p.image,width:p.width,height:p.height}));
if($('gold').checked)boxes(p.gold,'#096bcc');if($('pred').checked)boxes(c.boxes,'#dc7b0b',new Set(c.matches.map(x=>x.candidate)));
$('metrics').textContent=`参考 ${m.gold} 行 / 候选 ${m.predicted} 行 / 匹配 ${m.matched} 行 · F1 ${pct(m.f1)} · Recall ${pct(m.recall)} · Mean IoU ${m.mean_iou.toFixed(3)} · 顺序 ${pct(m.order_accuracy)} · 行对覆盖 ${pct(m.pair_coverage)}`}
for(const id of ['page','engine','gold','pred','numbers'])$(id).addEventListener('change',render);
for(const [id,step] of [['prev',-1],['next',1]])$(id).onclick=()=>{$('page').value=(+$('page').value+step+data.pages.length)%data.pages.length;render()};render();
</script></html>'''
    (OUT/'viewer.html').write_text(html.replace('DATA_PLACEHOLDER', data.replace('</', '<\\/')))
    print(f'Viewer written: {len(pages)} development pages, three candidates, original PNGs embedded')


if __name__ == '__main__':
    main()
