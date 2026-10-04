"""Evaluation-only development eligibility; runtime must never import this module.

An exact-line mismatch is not a missing word. Multiplicity is conserved. Raw
reader recovery is classified as a projection problem, not residual net gain.
No panel discovery, recognition, reference edits, or automatic page replacement.
"""
from collections import Counter
import argparse
import ast
import hashlib
import json
import re
import unicodedata
from pathlib import Path
from PIL import Image

RULES = 'source-complete-unit-v2'


def complete_greek_word(text):
    """Recognize one audited Greek word without normalizing source bytes.

    Completeness still comes from the source audit. Unicode alone cannot prove
    it. Mixed words, numerals, hyphen fragments and orphan marks do not count.
    """
    if not isinstance(text, str) or len(text.split()) != 1:
        return False
    word = text.strip('\"\'“”‘’.,;:!?()[]{}«»··')
    letters = 0
    for char in word:
        category = unicodedata.category(char)
        if category in ('Ll', 'Lu', 'Lt') and 'GREEK' in unicodedata.name(char, ''):
            letters += 1
        elif category.startswith('M') and 0x0300 <= ord(char) <= 0x036f and letters:
            continue
        else:
            return False
    return letters > 0


def greek_source_word_ids(units, rows):
    """Count individually confirmed physical occurrences, including repeats.

    Legacy compound transcriptions lack individual complete-word evidence and
    remain outside this denominator until source-audited; they are not split.
    """
    known = {row['unit_id'] for row in rows
             if row['state'] in ('SOURCE_AUDITED_GAP', 'BASELINE_COMPLETE', 'PROJECTION_ONLY')}
    return [unit['unit_id'] for unit in units
            if unit['unit_id'] in known and unit.get('bbox')
            and unit.get('complete_printed_word') is True
            and unit.get('source_literal') == unit.get('source_visual_transcription')
            and complete_greek_word(unit.get('source_literal'))]


def completed_off_readers(page, job, operation, image):
    """A saved array cannot stand in for successful full-page recognition.

    Raw hashes are verified by the caller before these records are inspected.
    EMPTY is a valid completed OFF observation; failed or native pages are not.
    The Apple input must bind the full frozen page rather than a residual crop.
    """
    if job['task'].get('mode') != 'local':
        return 'baseline did not use the full local OFF path'
    calls = []
    for name in page['raw_files']:
        if name.endswith('.call.json'):
            record = json.loads((operation / name).read_text())
            if (not isinstance(record, dict) or record.get('status') != 'OK'
                    or record.get('returncode') != 0):
                return 'baseline reader did not complete successfully'
            command = record.get('command')
            if not isinstance(command, list) or not all(isinstance(x, str) for x in command):
                return 'baseline reader command is unverified'
            calls.append(command)
    config = job['config']
    apple = [c for c in calls if len(c) == 3 and c[0] == config['apple_helper']]
    reader = [c for c in calls if len(c) == 10 and c[0] == config['tesseract']
              and Path(c[1]).resolve() == image.resolve()
              and c[2:] == ['stdout', '-l', 'grc+eng', '--oem', '1', '--psm', '3', 'tsv']]
    if len(calls) != 2 or len(apple) != 1 or len(reader) != 1:
        return 'complete Apple and full-page Tesseract receipts absent or repeated'
    apple_input = Path(apple[0][1]).resolve()
    apple_output = Path(apple[0][2]).resolve()
    if (not apple_input.is_relative_to(operation) or not apple_output.is_relative_to(operation)
            or str(apple_input.relative_to(operation)) not in page['raw_files']):
        return 'Apple input/output is outside the frozen raw ledger'
    records = json.loads(apple_input.read_text())
    if (not isinstance(records, list) or len(records) != 1 or not isinstance(records[0], dict)
            or not isinstance(records[0].get('image_path'), str)
            or Path(records[0]['image_path']).resolve() != image.resolve()):
        return 'Apple did not read the same complete source image'
    return None


def verify_reuse(units, subset, freeze):
    """Bind a qualified subset to real OFF snapshots and execution identities.

    A data label, current binary hash, or copied member list alone cannot admit
    execution. This is evaluation-side validation; it never invokes recognition.
    """
    if freeze is None:return dict(state='UNVERIFIED',reasons=['baseline reuse freeze absent'])
    if freeze.get('schema')!='development-reuse/1':raise ValueError('INVALID_REUSE_FREEZE')
    selected=[u for u in units if u['unit_id'] in (subset or [])]
    if not selected:return dict(state='UNVERIFIED',reasons=['execution subset absent'])
    records=freeze.get('units',{})
    reasons=[];validated=[]
    def frozen_file(record):
        path=Path(record['path'])
        if sha(path)!=record['sha256']:raise ValueError('REUSE_ARTIFACT_CHANGED: '+str(path))
        return path
    for unit in selected:
        record=records.get(unit['unit_id'])
        if record is None:reasons.append(unit['unit_id']+': reuse binding absent');continue
        if record.get('book')!=unit['book'] or record.get('source_id')!=(unit.get('source_id') or unit.get('source_page') or unit['page']):
            raise ValueError('REUSE_UNIT_SOURCE_SCOPE_MISMATCH')
        snapshot=json.loads(frozen_file(record['snapshot']).read_text())
        job=json.loads(frozen_file(record['job']).read_text());task=job['task']
        if snapshot['config_version']!=task['config_version'] or snapshot['config_version']!=freeze['candidate_config_version']:
            reasons.append(unit['unit_id']+': baseline config version differs from candidate');continue
        if job['config'].get('residual_enabled') is not False:
            reasons.append(unit['unit_id']+': baseline is not frozen OFF');continue
        source=frozen_file(record['source_pdf'])
        if sha(source)!=snapshot['input_sha256'] or task['input_sha256']!=snapshot['input_sha256'] or Path(snapshot['source_pdf']).resolve()!=source.resolve() or Path(task['input_pdf']).resolve()!=source.resolve():
            raise ValueError('REUSE_SOURCE_PDF_MISMATCH')
        number=record['baseline_page'];pages=[p for p in snapshot['pages'] if p['page']==number]
        if len(pages)!=1 or number not in task['page_numbers']:raise ValueError('REUSE_PAGE_SCOPE_MISMATCH')
        page=pages[0]
        if (page.get('status') not in ('OCR_DRAFT', 'EMPTY') or page.get('route') != 'ocr'
                or page.get('error')):
            reasons.append(unit['unit_id']+': OFF page recognition incomplete');continue
        if any(not isinstance(page.get(key), list) for key in ('original_apple', 'independent_reader', 'words')):
            reasons.append(unit['unit_id']+': complete OFF reader/member arrays absent');continue
        image=frozen_file(record['source_image'])
        if page['image_sha256']!=sha(image) or Path(page['image_path']).resolve()!=image.resolve():raise ValueError('REUSE_SOURCE_IMAGE_MISMATCH')
        crop_box=record.get('crop_bbox')
        if not crop_box or len(crop_box)!=4 or any(type(v) is not int for v in crop_box):raise ValueError('REUSE_CROP_GEOMETRY_ABSENT')
        box=unit['bbox']
        with Image.open(image) as full,Image.open(unit['crop']) as crop:
            if not (0<=crop_box[0]<=box[0]<box[2]<=crop_box[2]<=full.width and 0<=crop_box[1]<=box[1]<box[3]<=crop_box[3]<=full.height):
                raise ValueError('REUSE_CROP_GEOMETRY_MISMATCH')
            expected_crop=full.convert('RGB').crop(crop_box);actual_crop=crop.convert('RGB')
            if expected_crop.size!=actual_crop.size or expected_crop.tobytes()!=actual_crop.tobytes():raise ValueError('REUSE_CROP_PIXELS_MISMATCH')
        operation=Path(record['operation']).resolve()
        raw=page.get('raw_files',{})
        if not raw:reasons.append(unit['unit_id']+': raw ledger absent');continue
        for name,h in raw.items():
            path=(operation/name).resolve()
            if not path.is_relative_to(operation) or sha(path)!=h:raise ValueError('REUSE_RAW_CHANGED')
        incomplete=completed_off_readers(page,job,operation,image)
        if incomplete:
            reasons.append(unit['unit_id']+': '+incomplete);continue
        runtime_path=frozen_file(record['runtime_receipt'])
        if str(runtime_path.resolve().relative_to(operation)) not in raw:raise ValueError('UNBOUND_RUNTIME_RECEIPT')
        runtime=json.loads(runtime_path.read_text())
        if runtime.get('config')!=job['config']:raise ValueError('REUSE_RUNTIME_CONFIG_MISMATCH')
        if not runtime.get('loaded_modules') or not runtime.get('apple_helper_sha256') or not runtime.get('tesseract_executable_sha256') or not runtime.get('tessdata'):
            reasons.append(unit['unit_id']+': execution identity incomplete');continue
        expected=freeze.get('expected_execution_identity')
        actual={k:runtime.get(k) for k in ('code_sha256','apple_helper_sha256','tesseract_executable_sha256','tessdata')}
        if actual!=expected:
            reasons.append(unit['unit_id']+': execution code/helper/reader identity differs from frozen candidate');continue
        for module in runtime['loaded_modules'].values():
            if sha(module['path'])!=module['sha256']:raise ValueError('REUSE_LOADED_SOURCE_CHANGED')
        core_module=runtime['loaded_modules'].get('scripts.ocr.mvp.core')
        if not core_module:
            reasons.append(unit['unit_id']+': loaded recognition config module absent');continue
        constants=[ast.literal_eval(node.value) for node in ast.parse(Path(core_module['path']).read_text()).body
                   if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='CONFIG_VERSION' for t in node.targets)]
        if constants!=[freeze['candidate_config_version']]:
            reasons.append(unit['unit_id']+': loaded recognition source config differs');continue
        for key,hash_key in [('apple_helper','apple_helper_sha256'),('tesseract','tesseract_executable_sha256')]:
            if sha(job['config'][key])!=runtime[hash_key]:raise ValueError('REUSE_HELPER_CHANGED')
        for trained in runtime['tessdata']:
            if sha(trained['path'])!=trained['sha256']:raise ValueError('REUSE_TESSDATA_CHANGED')
        # Member copies must be the exact same observations, in this source crop.
        box=unit['bbox'];members=unit.get('baseline_members',{})
        for key in ('independent_reader','words'):
            expected_members=[w for w in page[key] if box[0]<=(w['bbox'][0]+w['bbox'][2])/2<=box[2] and box[1]<=(w['bbox'][1]+w['bbox'][3])/2<=box[3]]
            if members.get(key)!=expected_members:raise ValueError('REUSE_MEMBER_LEDGER_MISMATCH')
        validated.append(unit['unit_id'])
    return dict(state='VERIFIED' if not reasons and len(validated)==len(selected) else 'UNVERIFIED',
                reasons=reasons,validated_unit_ids=validated,new_ocr_calls=0,
                limitation='snapshot reuse contract only; not residual net gain or product admission')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def classify(unit):
    if not unit.get('pg_scorable'):
        return dict(state='INELIGIBLE', reason=unit.get('eligibility_note') or 'source not scorable')
    if not unit.get('crop') or sha(unit['crop']) != unit['crop_sha256']:
        raise ValueError('SOURCE_CROP_CHANGED')
    if unit.get('source_image') and sha(unit['source_image']) != unit['source_sha256']:
        raise ValueError('SOURCE_IMAGE_CHANGED')
    members = unit.get('baseline_members')
    if members:
        # Literal Unicode and spaces: punctuation/hyphenation rules must already
        # be frozen in source transcription. No normalization to inflate recall.
        required = Counter(unit['source_visual_transcription'].split())
        full = Counter(w['text'] for w in members.get('independent_reader', []))
        final = Counter(w['text'] for w in members.get('words', []))
        missing_final, missing_raw = required-final, required-full
        if not missing_final:
            return dict(state='BASELINE_COMPLETE', missing=[])
        if not missing_raw:
            return dict(state='PROJECTION_ONLY', missing=list(missing_final.elements()))
        # Lexical discrepancy alone may be an incorrect word or punctuation,
        # rather than a missing complete source unit. Require source audit label.
        if unit.get('genuine_lexical_gap') is True:
            return dict(state='SOURCE_AUDITED_GAP', missing=list(missing_raw.elements()),
                        human_checked=unit.get('human_checked',False))
        return dict(state='UNKNOWN',reason='source review does not establish a complete gap')
    return dict(state='UNKNOWN', reason='missing full-reader and final member ledger; old audit label alone is insufficient')


def qualify(units, *, max_pages=24, max_opportunities=120, subset=None, reuse_freeze=None):
    ids = [u['unit_id'] for u in units]
    if len(ids) != len(set(ids)):
        raise ValueError('DUPLICATE_SOURCE_UNIT')
    physical=[(u['book'],u.get('source_id') or u.get('source_page') or u['page'],tuple(u['bbox'])) for u in units if 'bbox' in u]
    if len(physical)!=len(set(physical)):raise ValueError('DUPLICATE_PHYSICAL_SOURCE_UNIT')
    pages = {(u['book'],u.get('source_id') or u.get('source_page') or u['page']) for u in units}
    if len(units)>max_opportunities or len(pages)>max_pages:
        raise ValueError('DEVELOPMENT_SCOPE_EXCEEDED')
    rows = [dict(unit_id=u['unit_id'],book=u['book'],structure=u['structure'],**classify(u)) for u in units]
    if subset is not None:
        if len(subset) != len(set(subset)) or not set(subset)<=set(ids):
            raise ValueError('INVALID_FROZEN_SUBSET')
        selected = [u for u in units if u['unit_id'] in subset]
        if len({(u['book'],u.get('source_id') or u.get('source_page') or u['page']) for u in selected})>12:
            raise ValueError('P3_SUBSET_PAGE_LIMIT')
        checked = [r for r in rows if r['unit_id'] in subset]
    else:
        selected = units
        checked = rows
    gaps = [r for r in checked if r['state']=='SOURCE_AUDITED_GAP']
    books, structures = sorted({r['book'] for r in gaps}), sorted({r['structure'] for r in gaps})
    reasons = []
    if len(gaps)<12:
        reasons.append(f'complete gaps {len(gaps)} < 12')
    if len(books)<2:
        reasons.append(f'gap books {len(books)} < 2')
    if len(structures)<2:
        reasons.append(f'gap structures {len(structures)} < 2')
    if subset is None:
        reasons.append('P3 execution subset not frozen')
    greek_ids = greek_source_word_ids(selected, checked)
    if len(greek_ids) < 100:
        reasons.append(f'complete Greek source words {len(greek_ids)} < 100')
    data_eligible=not reasons
    reuse=verify_reuse(units,subset,reuse_freeze)
    execution_reasons=reasons+reuse['reasons']
    return dict(schema='development-panel/2',rules=RULES,state='INSUFFICIENT' if execution_reasons else 'ELIGIBLE',
                data_eligible=data_eligible,reuse=reuse,residual_on_allowed=data_eligible and reuse['state']=='VERIFIED',total_opportunities=len(units),source_pages=len(pages),
                qualified_gaps=len(gaps),books=books,structures=structures,rows=rows,reasons=execution_reasons,
                Greek_complete_word_count=len(greek_ids),Greek_complete_word_minimum=100,
                Greek_complete_word_unit_ids=greek_ids,
                Greek_word_count_source='Individual source-confirmed complete words in the frozen subset; legacy compound and UNKNOWN references excluded',
                new_ocr_calls=0,reference_fed_runtime=False,exposed_regression=True)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    args=p.parse_args();data=json.loads(args.input.read_text());result=qualify(data['units'],subset=data.get('p3_subset'),reuse_freeze=data.get('reuse_freeze'))
    result['input_sha256']=sha(args.input)
    with args.output.open('x') as out:json.dump(result,out,ensure_ascii=False,indent=2)
    print(json.dumps({k:result[k] for k in ('state','qualified_gaps','reasons')}))


if __name__=='__main__':main()
