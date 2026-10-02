#!/usr/bin/env python3
"""Generate a standalone offline editor. Save an auditable patch, apply with CLI.
No external scripts, network server, browser storage or implicit confirmation.
"""
import json,sys
from pathlib import Path
from bookmarks import load

def write(table_path,output):
    table=load(table_path);payload=json.dumps(table,ensure_ascii=False).replace('<','\\u003c').replace('&','\\u0026')
    html='''<!doctype html><meta charset="utf-8"><title>本地目录书签编辑</title>
<style>body{font:15px system-ui;margin:24px;color:#222}button,input,select{font:inherit}td{padding:8px;border-bottom:1px solid #ddd}input.title{width:38vw}table{border-collapse:collapse}small{display:block;color:#666}button{margin:4px} .warn{color:#934600}</style>
<h1>目录书签</h1><p>PDF 页数从 1 开始。修改后需明确确认该条目，再保存编辑补丁；后端会重新验证目标页和树结构。删除包含子条目。</p>
<button id="add">新增条目</button><button id="save">保存编辑补丁</button><p id="message"></p><table><thead><tr><th>标题／原始证据</th><th>父条目</th><th>目标 PDF 页</th><th>审核</th><th></th></tr></thead><tbody id="rows"></tbody></table>
<script id="data" type="application/json">PAYLOAD</script><script>
const original=JSON.parse(document.getElementById('data').textContent);let entries=structuredClone(original.entries),ops=[],serial=0;
const el=(tag,text)=>{const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e};
function changed(e,fields){Object.assign(e,fields);e.state='needs_review';ops.push({op:'update',id:e.id,fields});render()}
function render(){const body=document.getElementById('rows');body.replaceChildren();for(const e of entries){const tr=el('tr');const title=el('td'),input=el('input');input.className='title';input.value=e.title;input.onchange=()=>changed(e,{title:input.value});title.append(input);const a=el('a','查看源页／证据位置');a.href=e.source_url||('file://'+original.source+'#page='+((e.source_page||0)+1));a.target='_blank';title.append(el('br'),a,el('small',e.id+' · 印刷页 '+(e.printed_page??'待审')));tr.append(title);
const p=el('td'),select=el('select');const root=el('option','顶层');root.value='';select.append(root);for(const other of entries.filter(x=>x.id!==e.id)){const o=el('option',other.id+' '+other.title.slice(0,28));o.value=other.id;select.append(o)}select.value=e.parent||'';select.onchange=()=>changed(e,{parent:select.value||null});p.append(select);tr.append(p);
const target=el('td'),n=el('input');n.type='number';n.min=1;n.max=original.page_count;n.style.width='70px';n.value=e.target_pdf_page===null?'':e.target_pdf_page+1;n.onchange=()=>changed(e,{target_pdf_page:n.value===''?null:Number(n.value)-1});target.append(n);tr.append(target);
const state=el('td'),confirm=el('button',e.state==='manually_confirmed'?'已确认':'确认标题、层级及目标');confirm.onclick=()=>{if(e.target_pdf_page===null){document.getElementById('message').textContent='请先填写目标 PDF 页。';return}ops.push({op:'confirm',id:e.id,evidence_ref:'offline-editor:source-page-'+((e.source_page||0)+1)});e.state='manually_confirmed';render()};state.append(confirm,el('small',(e.review_reasons||[]).join(', ')));tr.append(state);
const action=el('td'),del=el('button','删除');del.onclick=()=>{let ids=new Set([e.id]);let old=-1;while(old!==ids.size){old=ids.size;for(const x of entries)if(ids.has(x.parent))ids.add(x.id)}entries=entries.filter(x=>!ids.has(x.id));ops.push({op:'delete',id:e.id});render()};action.append(del);tr.append(action);body.append(tr)}}
document.getElementById('add').onclick=()=>{const e={id:'manual-'+Date.now()+'-'+serial++,title:'新条目',parent:null,level:0,target_pdf_page:null,printed_page:null,printed_value:null,printed_family:null,source_page:0,source_bbox:{x:0,y:0,width:1,height:1},evidence_ids:[],section_label:null,hierarchy_reason:'manual',review_reasons:['manual_confirmation_required'],state:'needs_review'};entries.push(e);ops.push({op:'add',entry:structuredClone(e)});render()};
document.getElementById('save').onclick=()=>{const patch={source_sha256:original.source_sha256,operations:ops};const url=URL.createObjectURL(new Blob([JSON.stringify(patch,null,2)],{type:'application/json'})),a=el('a');a.href=url;a.download='bookmark-edits.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);document.getElementById('message').textContent='已下载补丁。使用 edit 命令应用，再 export 写出 PDF。'};render();
</script>'''.replace('PAYLOAD',payload)
    with open(output,'x') as f:f.write(html)
if __name__=='__main__':write(*sys.argv[1:])
