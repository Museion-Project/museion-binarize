"""Read-only PDF consumer audit. No OCR, text normalization, or guessed word boxes.

Poppler nodes and their token offsets are separate objects. Multi-token nodes
retain aggregate geometry; that geometry never certifies individual positions.
Internal export order and independently supplied source reading order are
different gates. Missing source evidence returns INSUFFICIENT, never PASS.
"""
from collections import Counter, defaultdict
import argparse
import hashlib
import json
import math
import re
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path

import fitz

VERSION = 'pdf-consumer-audit-v7'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def extract_poppler(pdf_path, output_directory, *, timeout_seconds=30):
    """Capture two bounded local PDF consumers in a new, non-overwriting folder.

    This explicitly invoked helper performs no OCR. A caller must separately
    authorize its input and evaluation scope; an extraction receipt is not a
    quality, source-review or full-panel admission. Failed output is retained.
    """
    if type(timeout_seconds) is not int or not 1 <= timeout_seconds <= 60:
        raise ValueError('CONSUMER_TIMEOUT_RANGE')
    pdf = Path(pdf_path).resolve()
    pdf_hash = sha(pdf)
    executable = shutil.which('pdftotext')
    if not executable:
        raise ValueError('POPPLER_EXECUTABLE_MISSING')
    executable = Path(executable).resolve()
    tool_hash = sha(executable)
    root = Path(output_directory).resolve()
    root.mkdir()  # Reserve a fresh folder before any process is launched.
    receipt = dict(schema='pdf-poppler-extraction/1', producer_version=VERSION,
                   producer_path=str(Path(__file__).resolve()), producer_sha256=sha(__file__),
                   pdf_path=str(pdf), pdf_sha256=pdf_hash,
                   executable=str(executable), executable_sha256=tool_hash,
                   status='COMPLETE', calls=[], new_OCR=0, retries=0)
    for mode, name in [('-v', 'version'), ('-bbox-layout', 'bbox-layout'), ('-raw', 'raw')]:
        command = [str(executable), mode] + ([] if mode == '-v' else [str(pdf), '-'])
        start = time.monotonic()
        try:
            result = subprocess.run(command, capture_output=True, timeout=timeout_seconds,
                                    check=False)
            stdout, stderr, code = result.stdout, result.stderr, result.returncode
            state = 'COMPLETED' if code == 0 else 'FAILED'
        except subprocess.TimeoutExpired as error:
            stdout, stderr, code, state = error.stdout or b'', error.stderr or b'', None, 'TIMED_OUT'
        except OSError as error:
            stdout, stderr, code, state = b'', str(error).encode(), None, 'FAILED'
        outputs = {}
        for kind, data in [('stdout', stdout), ('stderr', stderr)]:
            data = data.encode() if isinstance(data, str) else data
            path = root / f'{name}.{kind}'
            with path.open('xb') as out:
                out.write(data)
            outputs[kind] = dict(path=str(path), sha256=sha(path))
        receipt['calls'].append(dict(mode=mode, command=command, returncode=code,
                                    state=state, timeout_seconds=timeout_seconds,
                                    wall_seconds=time.monotonic()-start, **outputs))
        if state != 'COMPLETED':
            receipt['status'] = 'FAILED'
            break
    receipt['pdf_sha256_after'] = sha(pdf)
    receipt['executable_sha256_after'] = sha(executable)
    if pdf_hash != receipt['pdf_sha256_after'] or tool_hash != receipt['executable_sha256_after']:
        receipt['status'] = 'FAILED'
    path = root / 'receipt.json'
    with path.open('x') as out:
        json.dump(receipt, out, ensure_ascii=False, indent=2)
    return path


def consumer_provenance_check(receipt_path, pdf_path, xml_path, raw_path):
    """Bind saved stdout to completed calls on this exact PDF and executable.

    Bytes supplied without a receipt remain useful diagnostics but cannot prove
    that Poppler exercised the PDF. A receipt is execution evidence, not an
    independent source oracle or a guarantee against fabricated evidence.
    """
    if receipt_path is None:
        return dict(state='INSUFFICIENT', reason='completed source-bound Poppler receipt missing')
    path = Path(receipt_path).resolve()
    receipt = json.loads(path.read_text())
    if receipt.get('schema') != 'pdf-poppler-extraction/1' or receipt.get('producer_version') != VERSION:
        raise ValueError('CONSUMER_RECEIPT_SCHEMA')
    if receipt.get('producer_path') != str(Path(__file__).resolve()) or receipt.get('producer_sha256') != sha(__file__):
        raise ValueError('CONSUMER_PRODUCER_CHANGED')
    pdf = Path(pdf_path).resolve()
    if (receipt.get('pdf_path') != str(pdf) or receipt.get('pdf_sha256') != sha(pdf)
        or receipt.get('pdf_sha256_after') != receipt['pdf_sha256']):
        raise ValueError('CONSUMER_PDF_IDENTITY')
    if receipt.get('status') != 'COMPLETE':
        return dict(state='FAIL', reason='consumer extraction failed or timed out', receipt_sha256=sha(path))
    executable = Path(receipt['executable']).resolve()
    if (str(executable) != receipt['executable'] or sha(executable) != receipt['executable_sha256']
        or receipt.get('executable_sha256_after') != receipt['executable_sha256']):
        raise ValueError('CONSUMER_EXECUTABLE_CHANGED')
    calls = receipt.get('calls', [])
    if [c.get('mode') for c in calls] != ['-v', '-bbox-layout', '-raw'] or receipt.get('retries') != 0 or receipt.get('new_OCR') != 0:
        raise ValueError('CONSUMER_CALL_COVERAGE')
    for call, name in zip(calls, ['version', 'bbox-layout', 'raw']):
        expected = [str(executable), call['mode']] + ([] if call['mode']=='-v' else [str(pdf), '-'])
        if (call.get('command') != expected or type(call.get('returncode')) is not int
            or call['returncode'] != 0 or call.get('state') != 'COMPLETED'):
            raise ValueError('CONSUMER_CALL_NOT_COMPLETED_OR_BOUND')
        for kind in ('stdout', 'stderr'):
            artifact = Path(call[kind]['path'])
            if (artifact.is_symlink() or artifact != path.parent/f'{name}.{kind}'
                or sha(artifact) != call[kind]['sha256']):
                raise ValueError('CONSUMER_OUTPUT_CHANGED')
    version = Path(calls[0]['stdout']['path']).read_bytes() + Path(calls[0]['stderr']['path']).read_bytes()
    if b'pdftotext version ' not in version:
        raise ValueError('CONSUMER_VERSION_UNVERIFIED')
    for call, supplied in zip(calls[1:], [xml_path, raw_path]):
        if Path(supplied).resolve() != Path(call['stdout']['path']) or sha(supplied) != call['stdout']['sha256']:
            raise ValueError('CONSUMER_SUPPLIED_OUTPUT_IDENTITY')
    return dict(state='PASS', basis='completed same-PDF local Poppler calls and exact captured stdout',
                receipt_path=str(path), receipt_sha256=sha(path), executable_sha256=receipt['executable_sha256'],
                version=version.decode('utf-8', errors='replace').strip(), cached_extraction_calls=2,
                new_extraction_calls=0, new_OCR=0, quality_or_source_review_proven=False)


def tokenize(text):
    return [(m.group(), m.start(), m.end()) for m in re.finditer(r'\S+', text)]


def project_members(parents):
    """Project exact literal tokens without dividing a parent's reader frame.

    Parent IDs/ownership stay in the snapshot. Single tokens keep legacy IDs;
    multipart token IDs bind the parent, whole literal and code-point offsets.
    The parent frame is only an aggregate diagnostic, never a child's box.
    """
    ids = [p['id'] for p in parents]
    if len(set(ids)) != len(ids):
        raise ValueError('DUPLICATE_SOURCE_MEMBER')
    result = []
    used = set(ids)
    for parent in parents:
        literal = parent['text']
        parts = tokenize(literal)
        if not parts:
            raise ValueError('EMPTY_SOURCE_MEMBER')
        for text, start, end in parts:
            identity = parent['id']
            if len(parts) > 1:
                binding = json.dumps([identity, literal, start, end], ensure_ascii=False,
                                     separators=(',', ':')).encode('utf-8')
                identity = 'source-token:' + hashlib.sha256(binding).hexdigest()
                if identity in used:
                    raise ValueError('SOURCE_TOKEN_ID_COLLISION')
                used.add(identity)
            result.append(dict(id=identity, parent_member_id=parent['id'],
                               text=text, literal_offset=[start, end],
                               bbox=list(parent['bbox']) if len(parts) == 1 else None,
                               aggregate_bbox=list(parent['bbox']),
                               precision='reader-frame' if len(parts) == 1 else 'aggregate-source-frame'))
    return result


def poppler_nodes(path):
    """Keep XML hierarchy, literal text, node bounds, offsets, and precision."""
    result = []
    for page_number, page in enumerate(ET.parse(path).getroot().iter(), 0):
        if page.tag.rsplit('}', 1)[-1] != 'page':
            continue
        nodes = []
        def visit(element, block=None, line=None):
            tag = element.tag.rsplit('}', 1)[-1]
            if tag in ('block', 'line'):
                identity = f'{tag}-{len(nodes)}-{element.attrib}'
                if tag == 'block':
                    block = identity
                else:
                    line = identity
            if tag == 'word':
                raw = ''.join(element.itertext())
                bbox = [float(element.attrib[k]) for k in ('xMin', 'yMin', 'xMax', 'yMax')]
                tokens = tokenize(raw)
                nodes.append(dict(node_id=len(nodes), block_id=block, line_id=line,
                                  bbox=bbox, raw_text=raw,
                                  tokens=[dict(text=t, offset=[a, b],
                                               bbox=bbox if len(tokens) == 1 else None,
                                               aggregate_bbox=bbox,
                                               precision='word' if len(tokens) == 1 else 'aggregate')
                                          for t, a, b in tokens]))
            for child in element:
                visit(child, block, line)
        visit(page)
        result.append(nodes)
    return result


def glyph_tokens(page):
    """Use actual consumer characters, including combining marks, without NFC."""
    result = []
    for bi, block in enumerate(page.get_text('rawdict')['blocks']):
        for li, line in enumerate(block.get('lines', [])):
            chars = [c for span in line['spans'] for c in span['chars']]
            text = ''.join(c['c'] for c in chars)
            # Offsets count Unicode code points; rawdict normally has one per char.
            expanded = [c for c in chars for _ in c['c']]
            for token, start, end in tokenize(text):
                rects = [fitz.Rect(c['bbox']) * page.rotation_matrix for c in expanded[start:end]]
                box = fitz.Rect(rects[0])
                for rect in rects[1:]:
                    box |= rect
                result.append(dict(text=token, bbox=list(box), precision='glyphs',
                                   block_id=bi, line_id=f'{bi}/{li}', offset=[start, end]))
    return result


def near(actual, target, tolerance=2):
    """Require center containment and substantial overlap, not any intersection."""
    a, t = fitz.Rect(actual), fitz.Rect(target) + (-tolerance, -tolerance, tolerance, tolerance)
    if a.is_empty or t.is_empty:
        return False
    center = (a.tl + a.br) / 2
    return t.contains(center) and (a & t).get_area() / a.get_area() >= .5


def correspondence(source, consumer):
    """Bijection on exact tokens + geometry; alternative perfect match is UNKNOWN.

    Matching by occurrence/context can establish text membership for an aggregate
    node. Individual source and consumer bounds are both required for position.
    Every consumer token is consumed at most once, even for repeated source words.
    """
    if len({s['id'] for s in source}) != len(source):
        raise ValueError('DUPLICATE_SOURCE_MEMBER')
    by_text = defaultdict(list)
    for j, token in enumerate(consumer):
        by_text[token['text']].append(j)
    graph = {}
    for i, member in enumerate(source):
        graph[i] = [j for j in by_text[member['text']]
                    if near(consumer[j].get('bbox') or consumer[j]['aggregate_bbox'],
                            member.get('bbox') or member['aggregate_bbox'])]

    def match(indices, forbidden=None):
        owners = {}
        def assign(i, seen):
            for j in graph[i]:
                if (i, j) == forbidden or j in seen:
                    continue
                seen.add(j)
                if j not in owners or assign(owners[j], seen):
                    owners[j] = i
                    return True
            return False
        for i in indices:
            assign(i, set())
        return {i: j for j, i in owners.items()}

    groups = defaultdict(list)
    for i, member in enumerate(source):
        groups[member['text']].append(i)
    mapping, ambiguous = {}, set()
    for indices in groups.values():
        chosen = match(indices)
        mapping.update(chosen)
        if len(chosen) == len(indices):
            for i, j in chosen.items():
                alternative = match(indices, (i, j))
                if len(alternative) == len(indices):
                    ambiguous.update(k for k in indices if alternative[k] != chosen[k])
        else:
            # An unmatched duplicate makes even the partial assignment uncertain.
            ambiguous.update(indices)
    rows = []
    for i, member in enumerate(source):
        j = mapping.get(i)
        token = consumer[j] if j is not None else None
        state = 'MISSING' if token is None else ('UNKNOWN_AMBIGUOUS' if i in ambiguous else
                ('TEXT_ONLY_SOURCE_AGGREGATE' if member.get('bbox') is None else
                 'POSITION_PROVEN' if token.get('bbox') is not None else 'TEXT_ONLY_AGGREGATE'))
        rows.append(dict(member_id=member['id'], parent_member_id=member.get('parent_member_id',member['id']),
                         text=member['text'], source_bbox=member.get('bbox'),
                         source_aggregate_bbox=member.get('aggregate_bbox'),
                         consumer_token=j, state=state, consumer=token))
    ec, ac = Counter(s['text'] for s in source), Counter(t['text'] for t in consumer)
    return dict(members=rows, missing=list((ec-ac).elements()),
                unexpected=list((ac-ec).elements()),
                unmatched_source=[source[i]['id'] for i in graph if i not in mapping],
                unmatched_consumer=[j for j in range(len(consumer)) if j not in mapping.values()],
                complete_positions=all(r['state'] == 'POSITION_PROVEN' for r in rows))


def source_order_check(ledger, source, snapshot, pdf_hash):
    if ledger is None:
        return dict(state='INSUFFICIENT', reason='independent source row/column ledger missing')
    if ledger.get('consumer_ready') is not True:
        return dict(state='INSUFFICIENT', reason='source preparation is not a reviewed consumer ledger')
    if ledger.get('source_sha256') != snapshot['input_sha256'] or ledger.get('pdf_sha256') != pdf_hash:
        raise ValueError('SOURCE_LEDGER_IDENTITY')
    if ledger.get('basis') != 'source-pixels' or not ledger.get('evidence_sha256'):
        raise ValueError('SOURCE_LEDGER_PROVENANCE')
    if not ledger.get('evidence_path') or sha(ledger['evidence_path']) != ledger['evidence_sha256']:
        raise ValueError('SOURCE_LEDGER_EVIDENCE_CHANGED')
    members = [m for row in ledger['rows'] for m in row['member_ids']]
    if Counter(members) != Counter(s['id'] for s in source):
        raise ValueError('SOURCE_LEDGER_MEMBERS')
    # This checks declared order only; caller must verify the referenced evidence.
    return dict(state='PASS' if members == [s['id'] for s in source] else 'FAIL',
                basis='externally source-reviewed row ledger', rows=ledger['rows'])


def source_geometry_check(ledger, source, snapshot, pdf_hash, number, original):
    """Verify reviewed source geometry before measuring actual consumer positions.

    A row proposal or old reader frame is not per-member source evidence. The
    review is an explicit AI/human observation, not Gold or recognition quality.
    Rendering binds its image to the actual physical page. No inference runs.
    """
    if not ledger or ledger.get('consumer_ready') is not True:
        return dict(state='INSUFFICIENT',reason='independent reviewed source-member geometry missing',members=[])
    binding=ledger.get('member_review')
    if not binding:return dict(state='INSUFFICIENT',reason='source-member review absent',members=[])
    path=Path(binding['path'])
    if sha(path)!=binding['sha256']:raise ValueError('SOURCE_MEMBER_REVIEW_CHANGED')
    review=json.loads(path.read_text())
    if (review.get('schema') not in ('source-member-geometry-review/1','source-member-geometry-review/2')
        or review.get('basis')!='source-pixel-review'
        or review.get('source_sha256')!=snapshot['input_sha256']
        or review.get('pdf_sha256')!=pdf_hash or review.get('physical_page')!=number):
        raise ValueError('SOURCE_MEMBER_REVIEW_IDENTITY')
    if not review.get('reviewer') or review.get('review_kind') not in ('AI_SOURCE_REVIEW','HUMAN_SOURCE_REVIEW'):
        raise ValueError('SOURCE_MEMBER_REVIEW_PROVENANCE')
    image=review['source_image'];dpi=image['dpi']
    if type(dpi) is not int or not 72<=dpi<=600:raise ValueError('SOURCE_MEMBER_REVIEW_DPI')
    if sha(image['path'])!=image['sha256']:raise ValueError('SOURCE_MEMBER_IMAGE_CHANGED')
    from PIL import Image
    with Image.open(image['path']) as frozen:
        frozen=frozen.convert('RGB');pix=original.get_pixmap(dpi=dpi,alpha=False)
        if frozen.size!=(pix.width,pix.height) or frozen.tobytes()!=pix.samples:
            raise ValueError('SOURCE_MEMBER_IMAGE_PAGE_MISMATCH')
    records=review['members'];expected={s['id']:s for s in source}
    if Counter(r['member_id'] for r in records)!=Counter(expected.keys()):raise ValueError('SOURCE_MEMBER_REVIEW_COVERAGE')
    projected=project_members(source);by_parent=defaultdict(list)
    for token in projected:by_parent[token['parent_member_id']].append(token)
    members=[];gaps=[];rejected=[];token_gaps=[];token_rejected=[]
    # Every independently reviewed support belongs to one token on this page,
    # including legacy single-token parents and leaves of different parents.
    # Distinct word regions may overlap (e.g. italic glyph extents); sharing a
    # source cell or the exact same physical crop is not independent evidence.
    source_cell_owners={};source_pixel_box_owners={}
    def checked_box(record):
        box=record.get('bbox')
        if (record.get('position_basis')!='source-pixels' or not record.get('note')
            or not box or len(box)!=4 or not all(type(v) in (int,float) and math.isfinite(v) for v in box)
            or fitz.Rect(box).is_empty or not original.rect.contains(fitz.Rect(box))):
            raise ValueError('SOURCE_MEMBER_GEOMETRY_UNPROVED')
        scale=dpi/72
        pixel_box=[math.floor(box[0]*scale),math.floor(box[1]*scale),math.ceil(box[2]*scale),math.ceil(box[3]*scale)]
        return list(box),frozen.crop(pixel_box).convert('L').getextrema()[0]<180,pixel_box
    for record in records:
        member=expected[record['member_id']]
        if record.get('export_literal')!=member['text']:raise ValueError('SOURCE_MEMBER_REVIEW_LITERAL_CHANGED')
        tokens=by_parent[member['id']]
        if review['schema']=='source-member-geometry-review/2':
            proofs=record.get('tokens',[])
            if Counter(t.get('token_id') for t in proofs)!=Counter(t['id'] for t in tokens):
                raise ValueError('SOURCE_TOKEN_REVIEW_COVERAGE')
            token_by_id={t['id']:t for t in tokens}
            for proof in proofs:
                token=token_by_id[proof['token_id']];offset=proof.get('offset')
                if (offset!=token['literal_offset'] or not isinstance(offset,list)
                    or any(type(v) is not int for v in offset) or proof.get('text')!=token['text']):
                    raise ValueError('SOURCE_TOKEN_REVIEW_LITERAL_CHANGED')
        elif len(tokens)>1:
            # An old single-box review cannot certify a multipart parent.
            proofs=[]
        else:
            proofs=[dict(record,token_id=tokens[0]['id'])]
        if record.get('state')=='REJECTED':rejected.append(member['id']);continue
        if record.get('state')!='SOURCE_POSITION_REVIEWED':gaps.append(member['id']);continue
        if not proofs:
            gaps.append(member['id']);token_gaps.extend(t['id'] for t in tokens);continue
        accepted=[];boxes=[];pending=False;failed=False
        for proof in proofs:
            token=next(t for t in tokens if t['id']==proof['token_id'])
            if proof.get('state')=='REJECTED':
                token_rejected.append(token['id']);failed=True;continue
            if proof.get('state')!='SOURCE_POSITION_REVIEWED':
                token_gaps.append(token['id']);pending=True;continue
            box,ink,pixel_box=checked_box(proof)
            # Independent word bounds may overlap even within one multipart
            # parent (e.g. italic extents). Parent grouping does not change the
            # source support. Exact boxes/pixels/cells still cannot be reused.
            if len(tokens)>1 and (box==list(member['bbox'])
                or any(box==other for other in boxes)):
                raise ValueError('SOURCE_TOKEN_GEOMETRY_OVERLAP_OR_AGGREGATE')
            boxes.append(box)
            if not ink:
                token_gaps.append(token['id']);pending=True;continue
            cell=proof.get('source_cell_id')
            if cell is not None:
                if not isinstance(cell,str) or not cell:
                    raise ValueError('SOURCE_TOKEN_SOURCE_CELL_IDENTITY')
                if cell in source_cell_owners:
                    raise ValueError('SOURCE_TOKEN_SOURCE_CELL_REUSED')
            pixel_key=tuple(pixel_box)
            if pixel_key in source_pixel_box_owners:
                raise ValueError('SOURCE_TOKEN_GEOMETRY_REUSED')
            if cell is not None:source_cell_owners[cell]=token['id']
            source_pixel_box_owners[pixel_key]=token['id']
            accepted.append(dict(id=token['id'],parent_member_id=member['id'],text=token['text'],
                                 literal_offset=token['literal_offset'],bbox=box,
                                 source_literal=proof.get('source_literal'),review_kind=review['review_kind'],
                                 source_cell_id=cell,source_support_pixel_bbox=pixel_box))
        # Partial token evidence is retained but never promotes the parent/gate.
        members.extend(accepted)
        if pending:gaps.append(member['id'])
        if failed:rejected.append(member['id'])
    return dict(state='FAIL' if rejected else 'INSUFFICIENT' if gaps else 'PASS',members=members,
                unresolved_member_ids=gaps,rejected_member_ids=rejected,
                unresolved_token_ids=token_gaps,rejected_token_ids=token_rejected,
                review_sha256=binding['sha256'],source_image_sha256=image['sha256'],
                recognition_quality_verified=False,human_checked=review['review_kind']=='HUMAN_SOURCE_REVIEW')


def physical_row_relations(source, nodes, ledger):
    """Prove real node/row membership without inventing aggregate word boxes.

    Independent source rows must include physical display-coordinate bounds,
    role and column. Exact contiguous members plus measured node bounds bind a
    node to one row. Repeated/competing spans remain UNKNOWN. This separate
    gate does not promote an aggregate node to individual word precision.
    """
    if not ledger or not ledger.get('rows'):
        return dict(state='INSUFFICIENT', reason='independent physical rows missing', nodes=[])
    rows=ledger['rows'];members={m['id']:m for m in source}
    if len(members)!=len(source):raise ValueError('DUPLICATE_SOURCE_MEMBER')
    declared=[m for r in rows for m in r['member_ids']]
    if Counter(declared)!=Counter(members.keys()):raise ValueError('SOURCE_LEDGER_MEMBERS')
    if any(not all(r.get(k) is not None for k in ('row_id','bbox','role','column_id')) for r in rows):
        return dict(state='INSUFFICIENT', reason='physical row bounds/role/column absent', nodes=[])
    if len({r['row_id'] for r in rows})!=len(rows):raise ValueError('DUPLICATE_SOURCE_ROW')
    for row in rows:
        box=row['bbox']
        if len(box)!=4 or not all(type(v) in (int,float) and math.isfinite(v) for v in box) or fitz.Rect(box).is_empty:
            raise ValueError('INVALID_SOURCE_ROW_BBOX')
        if any(not fitz.Rect(box).contains(fitz.Rect(members[m]['bbox'])) for m in row['member_ids']):
            raise ValueError('SOURCE_MEMBER_OUTSIDE_ROW')
    candidates=[]
    for node in nodes:
        texts=[t['text'] for t in node['tokens']];options=[]
        actual=fitz.Rect(node['bbox'])
        for row in rows:
            ids=row['member_ids'];sequence=[members[m]['text'] for m in ids]
            if not (fitz.Rect(row['bbox'])+(-2,-2,2,2)).contains(actual):continue
            for start in range(len(ids)-len(texts)+1):
                if not texts or sequence[start:start+len(texts)]!=texts:continue
                span=ids[start:start+len(texts)];bound=fitz.Rect(members[span[0]]['bbox'])
                for mid in span[1:]:bound|=fitz.Rect(members[mid]['bbox'])
                if near(actual,list(bound)):
                    options.append(dict(row_id=row['row_id'],member_ids=span,role=row['role'],column_id=row['column_id']))
        candidates.append(dict(node_id=node['node_id'],raw_text=node['raw_text'],bbox=node['bbox'],options=options))
    # Every occurrence is consumed once. Resolve only forced assignments;
    # existence of a plausible mapping is insufficient to pick an occurrence.
    used=set();resolved={};progress=True
    while progress:
        progress=False
        forced=[]
        for i,item in enumerate(candidates):
            if i in resolved:continue
            options=[o for o in item['options'] if not used.intersection(o['member_ids'])]
            item['options']=options
            if len(options)==1:forced.append((i,options[0]))
        counts=Counter(mid for _,option in forced for mid in option['member_ids'])
        for i,option in forced:
            if any(counts[mid]>1 for mid in option['member_ids']):continue
            resolved[i]=option;used.update(option['member_ids']);progress=True
    report=[]
    for i,item in enumerate(candidates):
        option=resolved.get(i)
        report.append(dict(**item,state='ROW_PROVEN' if option else 'UNKNOWN',binding=option,
                           individual_positions_proven=bool(option and len(option['member_ids'])==1)))
    order=[mid for i in range(len(candidates)) for mid in resolved.get(i,{}).get('member_ids',[])]
    complete=len(resolved)==len(nodes) and used==set(members)
    return dict(state='PASS' if complete and order==declared else ('FAIL' if complete else 'INSUFFICIENT'),
                nodes=report,unresolved_member_ids=[m for m in members if m not in used],
                complete_membership=complete,source_order_equal=order==declared,
                individual_positions_proven=all(r['individual_positions_proven'] for r in report),
                limitation='aggregate row bounds cannot establish individual word positions')


def audit(snapshot_path, pdf_path, xml_path, raw_path, ledger=None, consumer_receipt=None):
    provenance = consumer_provenance_check(consumer_receipt, pdf_path, xml_path, raw_path)
    snapshot = json.loads(Path(snapshot_path).read_text())
    folder=Path(snapshot_path).resolve().parent
    manifest=json.loads((folder/'manifest.json').read_text())
    for name,h in manifest['files'].items():
        artifact=(folder/name).resolve()
        if not artifact.is_relative_to(folder) or sha(artifact)!=h:
            raise ValueError('CONSUMER_ARTIFACT_CORRUPT')
    from .core import invariant
    invariant(snapshot)
    for p in snapshot['pages']:
        for name,h in p.get('raw_files',{}).items():
            artifact=(folder.parent/name).resolve()
            if not artifact.is_relative_to(folder.parent) or sha(artifact)!=h:
                raise ValueError('RAW_EVIDENCE_CORRUPT')
    if sha(snapshot['source_pdf']) != snapshot['input_sha256']:
        raise ValueError('SOURCE_CHANGED')
    nodes = poppler_nodes(xml_path)
    raw = Path(raw_path).read_text().split('\f')
    pages = []
    selected = {p['page']: p for p in snapshot['pages']}
    # Internal sorting is only the content-stream contract, never a source oracle.
    from .core import reading_rows
    with fitz.open(pdf_path) as pdf, fitz.open(snapshot['source_pdf']) as original:
        if len(pdf) != len(original) or len(nodes) != len(pdf):
            raise ValueError('CONSUMER_PAGE_COUNT')
        for n, page in enumerate(pdf, 1):
            # Render both fresh pages before text extraction. MuPDF's lazy image
            # caches can otherwise depend on which document was extracted first.
            actual_pixels = page.get_pixmap(dpi=144,alpha=False).samples
            original_pixels = original[n-1].get_pixmap(dpi=144,alpha=False).samples
            p = selected.get(n)
            source = []
            if p and p['route'] == 'ocr':
                for row in reading_rows(p['words']):
                    for w in row:
                        if w.get('export_status') != 'EXPORTED':
                            continue
                        x, y, r, b = w['bbox']
                        sx, sy = page.rect.width / p['width'], page.rect.height / p['height']
                        source.append(dict(id=w['id'], text=w['text'], bbox=[x*sx,y*sy,r*sx,b*sy]))
            parents=source
            source=project_members(parents)
            tokens = [dict(t, node_id=node['node_id'], block_id=node['block_id'],
                           line_id=node['line_id']) for node in nodes[n-1] for t in node['tokens']]
            glyphs = glyph_tokens(page)
            expected = [s['text'] for s in source]
            native = not source
            rows = dict(page=n, selected=p is not None, supported_words=len(source),
                        supported_parent_members=len(parents), source_parent_members=parents,
                        pending_words=[w['id'] for w in p.get('words',[]) if w.get('export_status')!='EXPORTED'] if p and p['route']=='ocr' else [],
                        nodes=nodes[n-1], source_members=source,
                        pixels_equal=actual_pixels == original_pixels,
                        actual_pixel_sha256=hashlib.sha256(actual_pixels).hexdigest(),
                        original_pixel_sha256=hashlib.sha256(original_pixels).hexdigest(),
                        dimensions_equal=page.rect == original[n-1].rect,
                        rotation_equal=page.rotation == original[n-1].rotation,
                        native_or_unselected_text_equal=page.get_text() == original[n-1].get_text() if native else None)
            if source:
                page_ledger=(ledger or {}).get(str(n))
                geometry=source_geometry_check(page_ledger,parents,snapshot,sha(pdf_path),n,original[n-1])
                independent_members=geometry['members']
                rows.update(poppler=correspondence(source,tokens), mupdf=correspondence(source,glyphs),
                            source_geometry=geometry,
                            independent_mupdf_positions=correspondence(independent_members,glyphs) if geometry['state']=='PASS' else None,
                            independent_poppler_positions=correspondence(independent_members,tokens) if geometry['state']=='PASS' else None,
                            mupdf_raw_order=[w[4] for w in page.get_text('words')] == expected,
                            poppler_raw_order=raw[n-1].split() == expected,
                            source_order=source_order_check(page_ledger,source,snapshot,sha(pdf_path)),
                            poppler_physical_rows=physical_row_relations(independent_members,nodes[n-1],page_ledger)
                                if geometry['state']=='PASS' else dict(state='INSUFFICIENT',reason='independent source-member geometry not ready',nodes=[]))
            pages.append(rows)
        def toc(doc):
            return [[a,b,c,{k:str(v) for k,v in d.items() if k != 'xref'}] for a,b,c,d in doc.get_toc(False)]
        bookmarks = toc(pdf) == toc(original)
    failures, gaps = [], []
    if provenance['state'] != 'PASS':
        (failures if provenance['state']=='FAIL' else gaps).append('Poppler execution provenance unverified')
    if not bookmarks:
        failures.append('bookmarks changed')
    for p in pages:
        if p['pending_words']:
            gaps.append(f'page {p["page"]}: pending PDF export members')
        for k in ('pixels_equal','dimensions_equal','rotation_equal','native_or_unselected_text_equal'):
            if p[k] is False:
                failures.append(f'page {p["page"]}: {k}')
        if not p['supported_words']:
            continue
        for k in ('mupdf_raw_order','poppler_raw_order'):
            if not p[k]:
                failures.append(f'page {p["page"]}: {k}')
        for engine in ('mupdf','poppler'):
            c = p[engine]
            if c['missing'] or c['unexpected']:
                failures.append(f'page {p["page"]}: {engine} word conservation')
            # Reader frames remain diagnostics. Reviewed source pixels are the
            # position authority when their complete, bound review passes.
            if p['source_geometry']['state']!='PASS' and not c['complete_positions']:
                gaps.append(f'page {p["page"]}: {engine} individual positions unresolved')
        if p['source_order']['state'] != 'PASS':
            (failures if p['source_order']['state'] == 'FAIL' else gaps).append(f'page {p["page"]}: source reading relations')
        if p['poppler_physical_rows']['state']!='PASS':
            (failures if p['poppler_physical_rows']['state']=='FAIL' else gaps).append(f'page {p["page"]}: consumer physical row relations')
        if p['source_geometry']['state']!='PASS':
            (failures if p['source_geometry']['state']=='FAIL' else gaps).append(f'page {p["page"]}: independently reviewed source-member geometry')
        else:
            for engine in ('mupdf','poppler'):
                if not p['independent_'+engine+'_positions']['complete_positions']:
                    gaps.append(f'page {p["page"]}: {engine} independent source positions unresolved')
    return dict(schema=VERSION, state='FAIL' if failures else ('INSUFFICIENT' if gaps else 'PASS'),
                inputs={str(p):sha(p) for p in (snapshot_path,pdf_path,xml_path,raw_path)},
                failures=failures, gaps=gaps, bookmarks_equal=bookmarks, pages=pages,
                consumer_provenance=provenance,
                poppler_path_verified=provenance['state']=='PASS',
                new_ocr_calls=0, new_reader_calls=0, cached_exposed_regression=True,
                position_authority='complete independent source-token review; reader frames are diagnostics',
                recognition_quality_verified=False, human_approval_claimed=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('snapshot','pdf','xml','raw','output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--source-ledger',type=Path)
    parser.add_argument('--consumer-receipt',type=Path)
    args = parser.parse_args()
    ledger=json.loads(args.source_ledger.read_text()) if args.source_ledger else None
    result = audit(args.snapshot,args.pdf,args.xml,args.raw,ledger,args.consumer_receipt)
    if args.source_ledger:result['inputs'][str(args.source_ledger)]=sha(args.source_ledger)
    if args.consumer_receipt:result['inputs'][str(args.consumer_receipt)]=sha(args.consumer_receipt)
    with args.output.open('x') as out:
        json.dump(result,out,ensure_ascii=False,indent=2)
    print(json.dumps({k:result[k] for k in ('state','failures','gaps')}))


if __name__ == '__main__':
    main()
