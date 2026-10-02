"""Evaluation-only source entry reconciliation. Never imported by runtime.

Geometry establishes one-to-one ownership; expected titles/folios only score the
already matched source unit. No normalization, count-based rescue or mutation.
"""
import argparse
import json
from pathlib import Path
from .source_identity import ownership
from .source_coverage import digest


def reconcile(candidate,ledger):
 if candidate['source_sha256']!=ledger['source_sha256']:raise ValueError('SOURCE_LEDGER_PDF_MISMATCH')
 pages=[p for p in candidate['pages'] if p['page_number']==ledger['page']]
 if len(pages)!=1 or pages[0]['image_sha256']!=ledger['image_sha256']:raise ValueError('SOURCE_LEDGER_PAGE_IMAGE_MISMATCH')
 page=pages[0];units=ledger['units'];ids=[u['unit_id'] for u in units]
 if len(ids)!=len(set(ids)):raise ValueError('DUPLICATE_SOURCE_ENTRY_UNIT')
 scale=72/ledger['image_dpi'];entries=page['entries'];graph={}
 for u in units:
  x,y,r,b=[v*scale for v in u['bbox_pixels']]
  graph[u['unit_id']]=[e for e in entries if e['word_ids'] and all(x<=(w['bbox'][0]+w['bbox'][2])/2<=r and y<=(w['bbox'][1]+w['bbox'][3])/2<=b for line in e['members'] for w in line['words'])]
 reverse={e['entry_id']:sum(e['entry_id'] in {v['entry_id'] for v in choices} for choices in graph.values()) for e in entries}
 rows=[];used=set()
 for u in units:
  choices=graph[u['unit_id']];e=choices[0] if len(choices)==1 and reverse[choices[0]['entry_id']]==1 else None
  if e:used.add(e['entry_id'])
  rows.append(dict(unit_id=u['unit_id'],entry_id=e['entry_id'] if e else None,unique_source_layout_members=e is not None,
                   title_literal_match=bool(e and e['title_literal']==u['title_literal']),
                   author_literal_match=bool(e and e['authors_literal']==u['authors_literal']),
                   printed_folio_literal_match=bool(e and u['printed_folio_literal'] is not None and e['printed_folio'] and e['printed_folio']['literal']==u['printed_folio_literal']),
                   entry_state=e['state'] if e else 'UNMATCHED',source_folio_unknown=u['printed_folio_literal'] is None,
                   source_words=e['word_ids'] if e else [],candidate_entry_ids=[c['entry_id'] for c in choices]))
 return dict(schema='toc-source-entry-reconciliation/1',source_units=len(rows),rows=rows,
             one_to_one_complete_members=sum(r['unique_source_layout_members'] for r in rows),
             title_literal_matches=sum(r['title_literal_match'] for r in rows),author_literal_matches=sum(r['author_literal_match'] for r in rows),
             printed_folio_literal_matches=sum(r['printed_folio_literal_match'] for r in rows),
             unexpected_entry_ids=[e['entry_id'] for e in entries if e['entry_id'] not in used],
             counts_fed_runtime=False,human_checked=False,automatic_normal_admission=False,complete_quality_ready=False)


def main():
 p=argparse.ArgumentParser(description=__doc__)
 for arg in ('candidate','ledger','output'):p.add_argument('--'+arg,type=Path,required=True)
 a=p.parse_args();report=reconcile(json.loads(a.candidate.read_text()),json.loads(a.ledger.read_text()))
 report.update(candidate_sha256=digest(a.candidate.read_bytes()),source_ledger_sha256=digest(a.ledger.read_bytes()))
 with a.output.open('x') as out:json.dump(report,out,ensure_ascii=False,indent=2)
 print(json.dumps({k:report[k] for k in ('source_units','one_to_one_complete_members','title_literal_matches','author_literal_matches','printed_folio_literal_matches')}))


if __name__=='__main__':main()
