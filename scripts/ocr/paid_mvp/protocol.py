"""Explicit v2 repair schema; backend preserves keep/unknown bytes by construction."""
import json
from .pipeline import check,BoundaryError

def validate_v2(response,page):
 check(response.get('model')=='gpt-6-luna' and response.get('status')=='completed','Luna model/status');texts=[c['text'] for o in response.get('output',[]) if o.get('type')=='message' for c in o.get('content',[]) if c.get('type')=='output_text']
 try:x=json.loads(''.join(texts))
 except Exception:raise BoundaryError('strict JSON required')
 check(type(x) is dict and set(x)=={'regions'} and type(x['regions']) is list,'overlay schema');wanted={r['id']:r for r in page['regions']};seen=set();normalized=[]
 for r in x['regions']:
  check(type(r) is dict and r.get('decision') in ['keep','unknown','replace'],'decision schema');check(r.get('id') in wanted and r['id'] not in seen,'ownership/duplicate');seen.add(r['id'])
  expected={'id','decision','text'} if r['decision']=='replace' else {'id','decision'};check(set(r)==expected,'v2 keep/unknown must not carry text; no extra fields')
  if r['decision']=='replace':check(type(r['text']) is str and wanted[r['id']]['locator']=='LOCATED','replace ownership/text')
  normalized.append(dict(id=r['id'],decision=r['decision'],text=r['text'] if r['decision']=='replace' else wanted[r['id']]['original_text']))
 check(seen==set(wanted),'missing regions');return dict(regions=normalized)
