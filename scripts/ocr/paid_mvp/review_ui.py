"""Offline editable source review; downloads actions for revision-checked save CLI/App."""
import html,json
from pathlib import Path

def render(out,task,pages,receipt=None):
 revision=receipt['revision'] if receipt else 0;rows=[]
 from .selective import review_context
 from .pipeline import read
 _,review_identity=review_context(out,read(Path(out)/'pages-v0.json'))
 for p in pages:
  image=task['images'][task['page_numbers'].index(p['page_number'])]['path']
  rows.append('<h2>Page '+str(p['page_number'])+'</h2><img style="max-width:90vw" src="'+html.escape(Path(image).as_uri(),quote=True)+'">')
  if 'observation' not in p:rows.append('<p>'+html.escape(p['status'])+'</p>');continue
  rows.append('<h3>Full immutable Mistral base / EXPORT_REVIEW</h3><pre>'+html.escape(p['observation']['text'])+'</pre><p>Unlocated nonspace characters: '+str(p['observation']['uncovered_nonspace_characters'])+'</p>')
  proposals={r['id']:r for r in p.get('overlay',{}).get('regions',[])}
  for r in p['observation']['regions']:
   proposed=proposals.get(r['id'],{});attrs=html.escape(json.dumps(dict(region_id=r['id'],member_ids=r['member_ids'],source_image_sha256=r['source_image_sha256'],review_identity=review_identity)),quote=True)
   rows.append('<fieldset data-region="'+attrs+'"><legend>'+html.escape(r['id'][:12]+' '+r['locator'])+'</legend><p>Source bbox: '+html.escape(str(r['bbox']))+'</p><pre>'+html.escape(r['original_text'])+'</pre><p>Proposal: '+html.escape(str(proposed.get('decision','none')))+'</p><textarea rows="5" style="width:95%">'+html.escape(proposed.get('text',r['original_text']))+'</textarea><select><option value="pending">Pending</option><option value="reject">Reject</option><option value="accept">Accept proposal</option><option value="change">Correct text</option></select><input placeholder="Source evidence / uncertainty"><input class="reviewer" placeholder="Reviewer identity"><label><input type="checkbox"> Human approved</label></fieldset>')
 metadata=json.dumps(dict(source_hash=task['input_sha256'],expected_revision=revision)).replace('<','\\u003c')
 script='''<script>const meta=META;document.querySelector('button').onclick=()=>{let actions=[];for(const f of document.querySelectorAll('fieldset')){const action=f.querySelector('select').value;if(action==='pending')continue;const r=JSON.parse(f.dataset.region);const images=[...document.querySelectorAll('img')];if(!images.every(i=>i.complete&&i.naturalWidth>0)){alert('Load source images before review');return;}actions.push({...r,source_image_loaded:true,action,text:f.querySelector('textarea').value,source_evidence:f.querySelector('input').value,reviewer:f.querySelector('.reviewer').value,human_approved:f.querySelector('[type=checkbox]').checked});}const data={...meta,actions};const a=document.createElement('a');a.href=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}));a.download='review-actions.json';a.click();};</script>'''.replace('META',metadata)
 target=Path(out)/f'review-v{revision}.html';target.write_text('<meta charset="utf-8"><title>Paid OCR source review</title><h1>Source review — revision '+str(revision)+'</h1><p>Review proposals against source pixels. UNKNOWN cannot be adopted. Download actions, then save with the review-import command or App. Source PDF remains untouched.</p><button>Download review actions</button>'+''.join(rows)+script);return str(target.resolve())
