"""Current base-only selector. No answer, historical repair, or external OCR input."""
import re
from .repair_units import VERSION
from .confidence import score

def select(units, limit=12):
 selected=[];pending=[]
 for u in units:
  value=score(u.get('confidence'))
  suspicious=bool(re.search('[�□]',u['original_text'])) or (value is not None and value<.85)
  if u['locator']!='LOCATED':pending.append(dict(id=u['id'],reason=u['reason']));continue
  if suspicious:selected.append(u)
  elif value is None:pending.append(dict(id=u['id'],reason='confidence_missing_or_invalid'))
 return dict(protocol=VERSION,selected=selected if len(selected)<=limit else [],pending=pending,status='budget-blocked' if len(selected)>limit else 'selected',candidate_count=len(selected),limit=limit,confidence_missing=sum(u.get('confidence') is None for u in units))
