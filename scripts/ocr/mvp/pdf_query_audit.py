"""Query-only supplement using already audited complete source members.
Queries go unchanged to the actual PDF search consumer. Case-insensitive source
association follows that consumer's search semantics; word conservation remains
literal in pdf_consumer_audit. A broad probe with multiple eligible source words
does not establish which individual occurrence was intended.
"""
import argparse
import json
from pathlib import Path
import fitz
from .pdf_consumer_audit import near,sha


def audit_queries(report_path,query_path):
    data=json.loads(Path(report_path).read_text());pages={p['page']:p for p in data['pages']}
    probes=json.loads(Path(query_path).read_text());rows=[];used=set()
    pdf_path=next(k for k in data['inputs'] if k.endswith('searchable.pdf'))
    if sha(pdf_path)!=data['inputs'][pdf_path]:raise ValueError('AUDITED_PDF_CHANGED')
    with fitz.open(pdf_path) as pdf:
        for panel in probes:
            number=panel['page'];page=pdf[number-1]
            for index,q in enumerate(panel['probes']):
                hits=[r*page.rotation_matrix for r in page.search_for(q['word'])]
                sources=[m for m in pages[number]['mupdf']['members'] if m['state']=='POSITION_PROVEN'
                         and q['word'].casefold() in m['text'].casefold()
                         and near(m['source_bbox'],q['bbox'])
                         and any(near(r,m['consumer']['bbox']) for r in hits)]
                free=[m for m in sources if (number,m['member_id']) not in used]
                member=free[0] if len(free)==1 else None
                if member:used.add((number,member['member_id']))
                rows.append(dict(page=number,probe=index,query=q['word'],raw_hit=bool(hits),
                                 verified_complete_member=member['member_id'] if member else None,
                                 candidates=[m['member_id'] for m in sources],
                                 position_state='PASS' if member else ('UNKNOWN' if hits else 'MISS'),
                                 full_token_query=bool(member and member['text']==q['word']),
                                 literal_query_not_rewritten=True,
                                 reason=None if member else 'no unique unused source-bound complete-word glyph match'))
    return dict(schema='fixed-query-complete-member-supplement/1',
                scope='query-only supplement; not another complete consumer evaluation',
                report_sha256=sha(report_path),query_sha256=sha(query_path),pdf_sha256=sha(pdf_path),
                denominator=len(rows),raw_hits=sum(r['raw_hit'] for r in rows),
                source_bound_positions=sum(r['position_state']=='PASS' for r in rows),
                rows=rows,new_ocr_calls=0,new_reader_calls=0)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('report','queries','output'):p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args();result=audit_queries(a.report,a.queries)
    with a.output.open('x') as f:json.dump(result,f,ensure_ascii=False,indent=2)
    print(json.dumps({k:result[k] for k in ('denominator','raw_hits','source_bound_positions')}))


if __name__=='__main__':main()
