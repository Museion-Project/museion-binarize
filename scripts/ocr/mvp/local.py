"""Fresh input -> Apple -> pixel residual crops -> optional independent reader -> PDF."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import fitz
from PIL import Image
from scripts.ocr.free_local import pipeline as old
from .core import CONFIG_VERSION, CONSUMER_POLICY, discover_residual, complete_residual_support, residual_member_in_target, compose, digest, sha, ownership
from .store import write, read, publish, load_snapshot
ROOT=Path(__file__).resolve().parents[3]

def runtime_config(config=None):
    config=dict(config or {})
    config.setdefault('apple_helper',os.environ.get('MUSEION_APPLE_HELPER',str(ROOT/'research-sessions/free-ocr-mvp-2026-09-29/apple-accurate')))
    config.setdefault('tesseract',os.environ.get('MUSEION_TESSERACT',shutil.which('tesseract') or ''))
    config.setdefault('font',str(old.FONT))
    config.setdefault('fallback_font_paths',['/System/Library/Fonts/Times.ttc'])
    config.setdefault('page_timeout_seconds',120)
    config.setdefault('max_residual_regions',32)
    config.setdefault('text_layer_policy','preserve')
    config.setdefault('residual_enabled',True)
    return config

def model_dependencies(config):
    import re
    try:
        result=subprocess.run([config['tesseract'],'--list-langs'],capture_output=True,text=True,timeout=10)
        match=re.search(r'"([^"\n]+)"',result.stdout)
        directory=Path(os.environ.get('TESSDATA_PREFIX',match[1] if match else ''))
        return [dict(name=n,path=str(directory/(n+'.traineddata')),sha256=sha(directory/(n+'.traineddata'))) for n in ['grc','eng']]
    except (OSError,subprocess.TimeoutExpired):return []

def loaded_source_identity():
    """Actual imported recognition/export modules, never evaluation references."""
    names=('scripts.ocr.mvp.core','scripts.ocr.mvp.local','scripts.ocr.mvp.store','scripts.ocr.free_local.pipeline')
    result={}
    for name in names:
        module=sys.modules.get(name);path=getattr(module,'__file__',None)
        if not path:raise RuntimeError('LOADED_SOURCE_IDENTITY_MISSING: '+name)
        path=Path(path).resolve();result[name]=dict(path=str(path),sha256=sha(path))
    return result

def worker_runtime_identity(config,tessdata=None):
    return dict(code_sha256={p.name:sha(p) for p in Path(__file__).parent.glob('*.py')},
                loaded_modules=loaded_source_identity(),python=dict(path=str(Path(sys.executable).resolve()),sha256=sha(Path(sys.executable).resolve())),
                apple_helper_sha256=sha(config['apple_helper']),tesseract_executable_sha256=sha(config['tesseract']),
                font_sha256=sha(config['font']),tessdata=model_dependencies(config) if tessdata is None else
                [dict(t,path=t['path'],sha256=sha(t['path'])) for t in tessdata],config=config)

def readiness(mode='local',config=None):
    if mode not in ('local','apple-residual'):raise ValueError('EXPLICIT_MODE_REQUIRED')
    config=runtime_config(config);deps=[]
    for name,key in [('Apple Vision helper','apple_helper'),('Tesseract','tesseract'),('Unicode font','font')]:
        p=Path(config[key]);deps.append(dict(name=name,path=str(p),available=p.is_file(),version=sha(p) if p.is_file() else None))
    deps.extend(dict(name=n,path=str(importlib.util.find_spec(n).origin) if importlib.util.find_spec(n) else None,available=importlib.util.find_spec(n) is not None,version=None) for n in ('fitz','cv2','PIL'))
    tess=config['tesseract'];langs=[]
    if tess and Path(tess).is_file():
        try:langs=subprocess.run([tess,'--list-langs'],capture_output=True,text=True,timeout=10).stdout.splitlines()
        except (OSError,subprocess.TimeoutExpired):pass
    deps.append(dict(name='Tesseract grc+eng',available='grc' in langs and 'eng' in langs,path=None,version=None))
    runtime=all(d['available'] for d in deps)
    deps.extend(dict(name='Optional export fallback font',path=path,available=Path(path).is_file(),version=sha(path) if Path(path).is_file() else None,required=False,distribution_closed=False,origin='macOS installed system font' if path.startswith('/System/') else 'configured font') for path in config['fallback_font_paths'])
    blockers=[]
    if not runtime:blockers.append(dict(code='LOCAL_DEPENDENCY_MISSING',message='Existing local runtime dependencies incomplete',evidence=deps))
    blockers.extend([dict(code='QUALITY_NOT_ACCEPTED',message='Quality acceptance is incomplete; consult verified version-specific blockers and final report (confirmation must not be repeated after its budget is consumed)',evidence=None),dict(code='APP_NOT_VERIFIED',message='App input/review/save consumption and relocatable packaging are not yet verified',evidence=None)])
    evidence=[];live=False
    if config.get('readiness_evidence'):
        try:
            proof=read(config['readiness_evidence'])
            verified_code=all(sha(Path(__file__).parent/name)==h for name,h in proof['code_hashes'].items())
            verified_runtime=proof['apple_helper_sha256']==sha(config['apple_helper']) and proof['tesseract_sha256']==sha(config['tesseract'])
            verified_artifacts=all(Path(p).is_file() and sha(p)==h for p,h in proof['artifact_hashes'].items())
            live=bool(proof.get('live_verified')) and mode in proof.get('live_modes',[]) and verified_code and verified_runtime and verified_artifacts
            if live:
                evidence=[str(Path(config['readiness_evidence']).resolve())]
                blockers.extend(dict(code=code,message='Verified acceptance blocker; see evidence manifest/report',evidence=evidence[0]) for code in proof.get('quality_blockers',[]))
        except (OSError,ValueError,KeyError):pass
    return dict(schema_version=1,mode=mode,component_ready=runtime,local_runtime_ready=runtime,app_ready=False,distribution_ready=False,quality_ready=False,live_verified=live,ready=False,blockers=blockers,dependencies=deps,evidence=evidence,config_version=CONFIG_VERSION)

def invoke(command,timeout,stdout,stderr):
    started=time.monotonic();record=dict(command=command,timeout_seconds=max(.1,timeout),process_start='new subprocess; model load included',status='FAILED')
    try:
        result=subprocess.run(command,capture_output=True,text=True,timeout=max(.1,timeout),env=dict(os.environ,OMP_THREAD_LIMIT='4'))
        Path(stdout).write_text(result.stdout);Path(stderr).write_text(result.stderr)
        record.update(returncode=result.returncode,status='OK' if result.returncode==0 else 'FAILED')
        if result.returncode:raise RuntimeError('LOCAL_READER_FAILED: '+str(command[0]))
        return result.stdout
    except Exception as exc:
        record['error']=repr(exc)
        raise
    finally:
        record['wall_seconds']=time.monotonic()-started
        write(str(stdout)+'.call.json',record)

def worker(request):
    source=Path(request['input_pdf']);page_no=request['page'];out=Path(request['output']);config=runtime_config(request['config'])
    started=time.monotonic();deadline=started+config['page_timeout_seconds']-2
    stages={};rawdir=out/'raw'/f'page-{page_no:04}';rawdir.mkdir(parents=True,exist_ok=True)
    p=dict(page=page_no,status='FAILED',route='ocr',words=[],review_reasons=[],source_sha256=request['input_sha256'],revision=0)
    try:
        if sha(source)!=request['input_sha256']:raise RuntimeError('WORKER_SOURCE_CHANGED')
        before=worker_runtime_identity(config)
        write(rawdir/'runtime-version.json',before)
        doc=fitz.open(source);page=doc[page_no-1]
        stage=time.monotonic();pix=page.get_pixmap(dpi=300,alpha=False);image=rawdir/'source.png';pix.save(image)
        p.update(width=pix.width,height=pix.height,image_path=str(image),image_sha256=sha(image),rotation=page.rotation)
        stages['render']=time.monotonic()-stage
        native=old.native_text(page)
        if native is not None and config['text_layer_policy']=='preserve':
            p.update(route='native',native_text=native,status='NATIVE_PRESERVED')
        elif page.get_text().strip() and config['text_layer_policy']=='replace' and any(t['type']!=3 for t in page.get_texttrace()):
            p.update(route='existing',native_text=page.get_text(),status='VISIBLE_TEXT_REPLACE_REVIEW',review_reasons=['Visible/native text replacement requires further source-preserving consumer implementation; no duplicate layer inserted'])
        elif page.get_text().strip() and config['text_layer_policy']=='preserve':
            p.update(route='existing',native_text=page.get_text(),status='EXISTING_TEXT_REVIEW',review_reasons=['Explicit replace required for an unreliable existing layer'])
        else:
            p['text_layer_policy']='replace' if page.get_text().strip() or page.rotation else 'preserve'
            inp=rawdir/'apple-input.json';write(inp,[dict(id='page',image_path=str(image))]);apple_dir=rawdir/'apple'
            stage=time.monotonic();invoke([config['apple_helper'],str(inp),str(apple_dir)],deadline-time.monotonic(),rawdir/'apple.stdout',rawdir/'apple.stderr')
            raw=read(apple_dir/'page.json')
            if raw.get('state')!='locally_recognized':raise RuntimeError('APPLE_INFERENCE_FAILED')
            original=old.apple_words(raw,pix.width,pix.height);p['original_apple']=original
            stages['apple']=time.monotonic()-stage
            stage=time.monotonic();regions,skipped=discover_residual(image,original,config['max_residual_regions']) if config['residual_enabled'] else ([],[]);p['residual_regions']=regions;p['residual_skipped']=skipped
            regions=complete_residual_support(image,regions);p['residual_regions']=regions
            stages['residual_detection']=time.monotonic()-stage
            residual=[];stage=time.monotonic()
            im=Image.open(image)
            for region in regions:
                if time.monotonic()>=deadline:raise TimeoutError('PAGE_DEADLINE')
                rid=region['member_id'];crop=rawdir/(rid+'.png');im.crop(region['canvas_bbox']).save(crop)
                tsv=invoke([config['tesseract'],str(crop),'stdout','-l','eng+grc','--oem','1','--psm','7','tsv'],deadline-time.monotonic(),rawdir/(rid+'.tsv'),rawdir/(rid+'.stderr'))
                x,y,_,_=region['canvas_bbox'];targets=[];unknown=[]
                for t in old.tess_words(tsv):
                    t['bbox']=[t['bbox'][0]+x,t['bbox'][1]+y,t['bbox'][2]+x,t['bbox'][3]+y]
                    if old.intersection(t['bbox'],region['target_bbox'])<=0:continue
                    if not residual_member_in_target(t['bbox'],region['target_bbox']):
                        unknown.append(dict(t,reason='incomplete_target_membership'));continue
                    t.update(id=rid+':'+t['id'],line_id=rid+':'+t['line_id'],engine='residual-tesseract',residual_member=rid,canvas_sha256=sha(crop),target_bbox=region['target_bbox'])
                    residual.append(t);targets.append(t['id'])
                region.update(canvas_path=str(crop),canvas_sha256=sha(crop),reader_members=targets,unknown_members=unknown,state='recognized' if targets else 'UNKNOWN')
            stages['residual_read']=time.monotonic()-stage
            p['residual_reader']=residual
            independent=[];stage=time.monotonic()
            if request['mode']=='local':
                tsv=invoke([config['tesseract'],str(image),'stdout','-l','grc+eng','--oem','1','--psm','3','tsv'],deadline-time.monotonic(),rawdir/'tesseract.tsv',rawdir/'tesseract.stderr')
                independent=old.tess_words(tsv)
            p['independent_reader']=independent;stages['independent_read']=time.monotonic()-stage
            stage=time.monotonic();words,decisions=compose(original,independent,residual);stages['merge']=time.monotonic()-stage
            p.update(words=words,contributions=ownership(words),decisions=decisions,status='OCR_DRAFT' if words else 'EMPTY',review_reasons=['Machine Greek/residual drafts; complex reading order unverified'])
        after=worker_runtime_identity(config,before['tessdata'])
        write(rawdir/'runtime-after.json',dict(before_sha256=sha(rawdir/'runtime-version.json'),identity=after,unchanged=after==before))
        if after!=before:raise RuntimeError('WORKER_RUNTIME_CHANGED')
        if sha(source)!=request['input_sha256']:raise RuntimeError('WORKER_SOURCE_CHANGED')
    except Exception as exc:
        error='PAGE_TIMEOUT' if isinstance(exc,(TimeoutError,subprocess.TimeoutExpired)) else repr(exc)
        p.update(status='FAILED',route='failed',words=[],error=error,review_reasons=['Failed page retained; raw partial evidence preserved'])
    p['raw_files']={str(f.relative_to(out)):sha(f) for f in rawdir.rglob('*') if f.is_file()}
    p['timings']=stages;p['wall_seconds']=time.monotonic()-started;write(rawdir/'page-result.json',p)
    return p

def run_task(task,config=None):
    required=('operation_id','input_pdf','input_sha256','page_numbers','mode','output_directory','config_version')
    if any(k not in task for k in required):raise ValueError('MISSING_TASK_FIELDS')
    if task['mode'] not in ('local','apple-residual'):raise ValueError('EXPLICIT_MODE_REQUIRED')
    if task['config_version']!=CONFIG_VERSION:
        # Completed legacy jobs remain resumable as their original receipt. They
        # never get relabelled as new-policy quality or rerun with changed code.
        legacy=Path(task['output_directory']).resolve()
        if task['config_version']=='mvp-local-v1' and (legacy/'CURRENT.json').is_file() and (legacy/'completion.json').is_file():
            job=read(legacy/'job.json')
            if job['task']==task and all(job['config'].get(k)==v for k,v in (config or {}).items()):
                if sha(Path(task['input_pdf']).resolve())!=task['input_sha256']:raise ValueError('SOURCE_HASH_MISMATCH')
                load_snapshot(legacy)
                return read(legacy/'completion.json')
        raise ValueError('CONFIG_VERSION_MISMATCH')
    source=Path(task['input_pdf']).resolve();out=Path(task['output_directory']).resolve();config=runtime_config(config)
    if config['text_layer_policy'] not in ('preserve','replace'):raise ValueError('EXPLICIT_TEXT_LAYER_POLICY')
    if type(config['residual_enabled']) is not bool:raise ValueError('EXPLICIT_RESIDUAL_POLICY')
    if not 1<=config['max_residual_regions']<=256:raise ValueError('RESIDUAL_REGION_LIMIT')
    if not 1<=config['page_timeout_seconds']<=120:raise ValueError('PAGE_TIMEOUT_LIMIT')
    if source.is_relative_to(out):raise ValueError('OUTPUT_CONTAINS_SOURCE')
    if sha(source)!=task['input_sha256']:raise ValueError('SOURCE_HASH_MISMATCH')
    doc=fitz.open(source)
    if doc.is_encrypted:raise ValueError('ENCRYPTED_INPUT')
    pages=task['page_numbers']
    if not pages or len(pages)!=len(set(pages)) or any(type(p) is not int or not 1<=p<=len(doc) for p in pages):raise ValueError('INVALID_PAGE_RANGE')
    doc.close();identity=digest(dict(task=task,config=config))
    if out.exists():
        if not (out/'job.json').exists() or read(out/'job.json')['identity']!=identity:raise FileExistsError('Use a new output directory or identical resumable task')
        if (out/'CURRENT.json').exists():return read(out/'completion.json')
    else:out.mkdir(parents=True)
    write(out/'job.json',dict(schema_version=1,identity=identity,task=task,config=config));(out/'raw').mkdir(exist_ok=True)
    results=[];started=time.monotonic();cancelled=False
    for number in pages:
        if (out/'CANCEL').exists():cancelled=True;break
        resultpath=out/'raw'/f'page-{number:04}'/'page-result.json'
        if resultpath.exists():p=read(resultpath)
        else:
            request=dict(input_pdf=str(source),input_sha256=task['input_sha256'],page=number,mode=task['mode'],output=str(out),config=config)
            requestpath=out/'raw'/f'worker-{number}.json';write(requestpath,request)
            proc=subprocess.Popen([sys.executable,'-m','scripts.ocr.mvp.local','--worker',str(requestpath)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=ROOT,start_new_session=True)
            deadline=time.monotonic()+config['page_timeout_seconds']
            try:
                while True:
                    if (out/'CANCEL').exists():
                        cancelled=True;os.killpg(proc.pid,signal.SIGKILL);stdout,stderr=proc.communicate()
                        p=dict(page=number,status='CANCELLED',route='failed',error='USER_CANCELLED',words=[],source_sha256=task['input_sha256'])
                        resultpath.parent.mkdir(parents=True,exist_ok=True);write(resultpath,p);break
                    remaining=deadline-time.monotonic()
                    if remaining<=0:raise subprocess.TimeoutExpired(proc.args,config['page_timeout_seconds'])
                    try:
                        stdout,stderr=proc.communicate(timeout=min(.25,remaining));break
                    except subprocess.TimeoutExpired:continue
                (out/'raw'/f'worker-{number}.stdout').write_text(stdout);(out/'raw'/f'worker-{number}.stderr').write_text(stderr)
                if not resultpath.exists():raise RuntimeError('PAGE_WORKER_FAILED')
                p=read(resultpath)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid,signal.SIGKILL);stdout,stderr=proc.communicate()
                p=dict(page=number,status='FAILED',route='failed',error='PAGE_TIMEOUT',words=[],wall_seconds=config['page_timeout_seconds'],source_sha256=task['input_sha256'])
                resultpath.parent.mkdir(parents=True,exist_ok=True);write(resultpath,p)
            except Exception as exc:
                p=dict(page=number,status='FAILED',route='failed',error=repr(exc),words=[],source_sha256=task['input_sha256'])
                resultpath.parent.mkdir(parents=True,exist_ok=True);write(resultpath,p)
        results.append(p);write(out/'progress.json',dict(schema_version=1,completed=len(results),total=len(pages),page_results=results));print(f'page {number}: {p["status"]}',file=sys.stderr,flush=True)
        if cancelled:break
    # Cancellation preserves unprocessed source pages in output and records their state.
    for number in pages[len(results):]:results.append(dict(page=number,status='CANCELLED',route='failed',words=[]))
    snapshot=dict(schema_version=1,operation_id=task['operation_id'],mode=task['mode'],source_pdf=str(source),input_sha256=task['input_sha256'],revision=0,pages=results,receipts=[],config_version=CONFIG_VERSION,consumer_policy=CONSUMER_POLICY,font_path=config['font'],fallback_font_paths=config['fallback_font_paths'])
    old.FONT=Path(config['font']);save_start=time.monotonic();folder=publish(out,snapshot)
    artifacts={k:str(folder/n) for k,n in [('searchable_pdf','searchable.pdf'),('text','text.txt'),('pages_json','pages.json'),('review_html','review.html')]};artifacts['raw_directory']=str(out/'raw');artifacts['page_mapping_json']=str(folder/'page-map.json')
    status='cancelled' if cancelled else ('failed' if all(p['status']=='FAILED' for p in results) else 'review_required')
    envelope=dict(schema_version=1,operation_id=task['operation_id'],mode=task['mode'],status=status,input_sha256=task['input_sha256'],output_directory=str(out),artifacts=artifacts,page_results=results,review_required=True,timings=dict(total_seconds=time.monotonic()-started,save_seconds=time.monotonic()-save_start),usage=dict(cloud_requests=0,downloads=0),evidence=[str(out/'job.json')],revision=0,export_review=snapshot.get('export_review',[]),**{key:snapshot[key] for key in ('source_page_count','exported_page_count','untouched_page_numbers','pdf_page_mapping')})
    write(out/'completion.json',envelope)
    if sha(source)!=task['input_sha256']:raise ValueError('SOURCE_CHANGED')
    return envelope

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--worker');args=parser.parse_args()
    if args.worker:worker(read(args.worker))
if __name__=='__main__':main()

def cancel_task(output_directory):
    """Cooperative cancel signal, checked at <=250ms while a worker is active."""
    out=Path(output_directory).resolve()
    if not (out/'job.json').is_file():raise ValueError('UNKNOWN_OPERATION')
    (out/'CANCEL').touch()
    return dict(schema_version=1,status='cancellation_requested',output_directory=str(out))
