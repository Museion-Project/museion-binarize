"""Self-contained source/fragment/logical-line viewer for the recorded finalists."""
import base64,json
from pathlib import Path
from run_dev_geometry_bakeoff import MANIFEST,verify
from run_finalist_bakeoff import OUT

def main():
    m=json.loads(MANIFEST.read_text());verify(m);r=json.loads((OUT/'results.json').read_text())
    data={'pages':[],'winner':r['ranking'][0]}
    for p in m['pages']:
        page={'id':p['page_id'],'width':p['width'],'height':p['height'],'gold':p['lines'],
              'image':'data:image/png;base64,'+base64.b64encode(Path(p['image_path']).read_bytes()).decode(),'providers':{}}
        for name in ['paddle','surya']:
            page['providers'][name]={'geometry':json.loads((OUT/'normalized'/name/f'{p["page_id"]}-pass1.json').read_text()),
             'metrics':next(x for x in r['candidates'][name]['passes'][0]['pages'] if x['page_id']==p['page_id'])}
        data['pages'].append(page)
    html='''<!doctype html><html lang="zh"><meta charset="utf-8"><title>Museion geometry finalists R2</title>
<style>body{margin:0;background:#f4f4f0;color:#24323a;font:14px system-ui}header{position:sticky;top:0;background:#fffffff5;padding:12px 18px;z-index:3;border-bottom:1px solid #ccd4d4}h1{font-size:20px;margin:0 0 7px}select,button{padding:5px;font:inherit}label{white-space:nowrap;margin:0 5px}main{display:grid;grid-template-columns:minmax(0,1fr) 350px;gap:15px;padding:14px}svg{width:100%;background:white}aside{background:white;padding:12px;overflow:auto;max-height:85vh;position:sticky;top:145px}pre{font-size:11px;white-space:pre-wrap;overflow-wrap:anywhere}.note{font-size:12px;color:#56656b}#metrics{margin:7px 0}</style>
<header><h1>Museion geometry · Paddle / Surya 决赛</h1><button id="prev">←</button> <select id="page"></select> <button id="next">→</button> <select id="provider"><option value="surya">Surya</option><option value="paddle">Paddle</option></select>
<div><label><input id="gold" type="checkbox" checked>参考（蓝）</label><label><input id="lines" type="checkbox" checked>logical lines（橙）</label><label><input id="frags" type="checkbox">fragments（绿）</label><label><input id="raw" type="checkbox">原始框（灰）</label><label><input id="layout" type="checkbox">原始layout（紫）</label><label><input id="cols" type="checkbox">column bands（青）</label><label><input id="nums" type="checkbox">顺序号</label></div>
<p id="metrics"></p><div class="note">仅25页dev；点击橙色框查看fragment/source链。几何hint不是最终D角色或TOC membership。评分为logical envelope；fragment并集覆盖单独报告。</div></header>
<main><svg id="canvas" xmlns="http://www.w3.org/2000/svg"></svg><aside><b>几何证据</b><p class="note">每个原始框都保留；拆分源polygon、子polygon、layout containment和column证据可追溯。</p><pre id="details">点击logical line查看。</pre></aside></main>
<script id="data" type="application/json">DATA</script><script>
const data=JSON.parse(document.getElementById('data').textContent),$=id=>document.getElementById(id),ns='http://www.w3.org/2000/svg';
data.pages.forEach((p,i)=>{const o=document.createElement('option');o.value=i;o.textContent=`${i+1}/25 · ${p.id}`;$('page').append(o)});
const params=new URLSearchParams(location.hash.slice(1));$('page').value=String(Math.max(0,data.pages.findIndex(p=>p.id===params.get('page'))));$('provider').value=params.get('provider')||data.winner;
const pct=x=>x===null?'N/A':(x*100).toFixed(2)+'%';
function node(t,a){const e=document.createElementNS(ns,t);for(const[k,v]of Object.entries(a))e.setAttribute(k,v);return e}
function draw(box,color,label,click){const[a,b,c,d]=box,e=node('rect',{x:a,y:b,width:c-a,height:d-b,fill:'none',stroke:color,'stroke-width':2,'pointer-events':'all'});if(click){e.style.cursor='pointer';e.onclick=click}const tt=node('title',{});tt.textContent=label;e.append(tt);$('canvas').append(e);if($('nums').checked){const t=node('text',{x:a,y:b,fill:color,'font-size':18,'paint-order':'stroke',stroke:'white','stroke-width':2});t.textContent=label;$('canvas').append(t)}}
function render(){const p=data.pages[+$('page').value],c=p.providers[$('provider').value],g=c.geometry,m=c.metrics;$('canvas').replaceChildren();$('canvas').setAttribute('viewBox',`0 0 ${p.width} ${p.height}`);$('canvas').append(node('image',{href:p.image,width:p.width,height:p.height}));
if($('layout').checked)g.layout_regions.forEach(x=>draw(x.bbox,'#975eb6',x.region_id));if($('cols').checked)g.column_bands.forEach(x=>draw(x.bbox,'#009ca6',x.band_id));
if($('raw').checked)g.source_fragments.forEach(x=>draw(x.bbox,'#7b8388',x.fragment_id));if($('gold').checked)p.gold.forEach((x,i)=>draw(x.bbox,'#176ec4',i+1));
if($('frags').checked)g.fragments.forEach(x=>draw(x.bbox,'#0a9453',x.fragment_id));if($('lines').checked)g.logical_lines.forEach(x=>draw(x.bbox,'#dc7b0b',x.reading_order+1,()=>{const fragments=g.fragments.filter(f=>x.fragment_ids.includes(f.fragment_id));const sources=g.source_fragments.filter(s=>fragments.some(f=>f.source_box_index===s.source_box_index));$('details').textContent=JSON.stringify({logical_line:x,fragments,source_fragments:sources,layout_regions:g.layout_regions.filter(r=>sources.some(s=>s.region_ids.includes(r.region_id))),column_bands:g.column_bands.filter(b=>b.band_id===x.band_id),provenance:g.provenance},null,2)}));
$('metrics').textContent=`参考 ${m.gold} / logical ${m.predicted} / 匹配 ${m.matched} · F1 ${pct(m.f1)} · IoU≥.50 F1 ${pct(m.f1_at_50)} · 顺序 ${pct(m.order_accuracy)} · 逆序 ${m.inverted_pairs} · fragment并集覆盖 ${pct(m.support_union_coverage_all_reference)}`;
$('details').textContent=JSON.stringify({page:p.id,normalization:m.normalization,column_metrics:m.column,catastrophic_omissions:m.catastrophic_omissions,specials:m.specials},null,2)}
for(const id of ['page','provider','gold','lines','frags','raw','layout','cols','nums'])$(id).onchange=render;
for(const[id,s]of[['prev',-1],['next',1]])$(id).onclick=()=>{$('page').value=(+$('page').value+s+25)%25;render()};render();
</script></html>'''
    (OUT/'viewer.html').write_text(html.replace('DATA',json.dumps(data,separators=(',',':'),ensure_ascii=False).replace('</','<\\/')))
    print('Finalist viewer: 25 pages, all source/fragment/layout/column evidence embedded')

if __name__=='__main__':main()
