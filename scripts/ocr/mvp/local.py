"""Fresh input -> Apple -> pixel residual crops -> optional independent reader -> PDF."""
import argparse
from contextlib import contextmanager
import fcntl
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import time
import fitz
from PIL import Image
from scripts.ocr.free_local import pipeline as old
from .core import CONFIG_VERSION, CONSUMER_POLICY, discover_residual, complete_residual_support, residual_member_in_target, compose, digest, sha, ownership, invariant
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

class PageWorkerCleanupError(RuntimeError):
    """Do not advance to another page if the owned worker cannot be reaped."""

def reap_page_worker(proc):
    """Only for this parent's child, started in its own process session."""
    try:
        try:
            # An unreaped child retains its PID even if it has just exited.
            # Do not poll/reap before signalling its isolated worker group.
            if proc.returncode is None:
                try:os.killpg(proc.pid,signal.SIGKILL)
                except ProcessLookupError:pass
            proc.wait(timeout=5)
        finally:
            for stream in (proc.stdout,proc.stderr):
                if stream is not None:stream.close()
    except Exception as exc:
        raise PageWorkerCleanupError('PAGE_WORKER_CLEANUP_FAILED') from exc

def parent_page_result(out,resultpath,page):
    """Keep worker-written bytes, including malformed or partial results."""
    resultpath.parent.mkdir(parents=True,exist_ok=True)
    receipt=resultpath.with_name('parent-result.json') if resultpath.exists() else resultpath
    page['raw_files']={str(f.relative_to(out)):sha(f) for f in resultpath.parent.rglob('*')
                       if f.is_file() and f!=receipt}
    write(receipt,page)

def pipe_text(value):
    return value.decode('utf-8',errors='replace') if isinstance(value,bytes) else (value or '')

@contextmanager
def worker_parent_lifeline(fd):
    """A parent-owned pipe, never an unrelated PID or process-group target."""
    import fcntl
    import select
    import stat
    import threading
    if type(fd) is not int or fd<3:raise ValueError('WORKER_PARENT_LIFELINE_REQUIRED')
    if not stat.S_ISFIFO(os.fstat(fd).st_mode):raise ValueError('WORKER_LIFELINE_PIPE_REQUIRED')
    if fcntl.fcntl(fd,fcntl.F_GETFL)&os.O_ACCMODE!=os.O_RDONLY:raise ValueError('WORKER_LIFELINE_READ_END_REQUIRED')
    if os.getpgrp()!=os.getpid():raise ValueError('WORKER_ISOLATED_GROUP_REQUIRED')
    finished=threading.Event()
    def stop_owned_group():
        if os.getpgrp()!=os.getpid():os._exit(125)
        os.killpg(os.getpid(),signal.SIGKILL)
        # Do not resume the body or race the requested signal with exit(125).
        while True:signal.pause()
    def monitor():
        while True:
            try:os.read(fd,1);break
            except InterruptedError:continue
            except OSError:break
        if not finished.is_set():stop_owned_group()
    try:
        # Reader subprocesses must not inherit this private descriptor.
        os.set_inheritable(fd,False)
        if select.select([fd],[],[],0)[0]:stop_owned_group()
        threading.Thread(target=monitor,name='worker-parent-lifeline',daemon=True).start()
        yield
    finally:
        finished.set();os.close(fd)

def load_task_completion(out,snapshot=None,folder=None):
    """Read a bound completion, or expose a verified draft with no success claim.

    Publishing CURRENT and writing completion are separate durable operations.
    A missing or partially written completion does not authorize worker replay
    or fabrication of a successful receipt. Existing bytes remain untouched.
    """
    out=Path(out).resolve()
    if snapshot is None or folder is None:snapshot,folder=load_snapshot(out)
    path=out/'completion.json';state='missing'
    bound_identity=bool(type(snapshot.get('operation_id')) is str and snapshot['operation_id'] and
                        type(snapshot.get('mode')) is str and snapshot['mode'])
    if path.is_symlink():raise ValueError('COMPLETION_PATH_ESCAPE')
    if bound_identity:
        jobpath=out/'job.json'
        if jobpath.is_symlink():raise ValueError('TASK_JOB_PATH_SYMLINK')
        job=read(jobpath) if jobpath.is_file() else {}
        if job.get('reuse_manifest_sha256') or (out/'reused-pages.json').exists() or (out/'reused-pages.json').is_symlink():
            reused,inherited=_reused_page_state(out,job['task'],job['config'],job,validate_results=False)
            if (job['task'].get('operation_id')!=snapshot['operation_id'] or
                job['task'].get('input_sha256')!=snapshot['input_sha256'] or snapshot.get('inherited_review')!=inherited):
                raise ValueError('REUSED_REVIEW_BINDING_MISMATCH')
    if not bound_identity:state='identity_unverified'
    elif path.exists():
        try:completion=read(path)
        except (OSError,ValueError,UnicodeError):state='unreadable'
        else:
            if not isinstance(completion,dict):state='unreadable'
            else:
                if (completion.get('input_sha256')!=snapshot['input_sha256'] or
                    completion.get('operation_id')!=snapshot['operation_id'] or
                    completion.get('mode')!=snapshot['mode'] or
                    type(completion.get('revision')) is not int or completion.get('revision')!=snapshot['revision'] or
                    digest(completion.get('page_results'))!=snapshot['pages_hash'] or
                    completion.get('status') not in ('review_required','failed','cancelled') or
                    not isinstance(completion.get('artifacts'),dict)):
                    raise ValueError('COMPLETION_BINDING_MISMATCH')
                return completion,'recorded'
    artifacts={key:str(folder/name) for key,name in
               [('searchable_pdf','searchable.pdf'),('text','text.txt'),('pages_json','pages.json'),('review_html','review.html')]}
    artifacts.update(raw_directory=str(out/'raw'),page_mapping_json=str(folder/'page-map.json'))
    result=dict(schema_version=1,operation_id=snapshot.get('operation_id'),mode=snapshot.get('mode'),
                status='completion_unverified',input_sha256=snapshot['input_sha256'],output_directory=str(out),
                artifacts=artifacts,page_results=snapshot['pages'],review_required=True,revision=snapshot['revision'],
                export_review=snapshot.get('export_review',[]),completion_record_state=state,
                evidence=[str(out/'CURRENT.json'),str(folder/'manifest.json')],
                **{key:snapshot[key] for key in ('source_page_count','exported_page_count','untouched_page_numbers','pdf_page_mapping')})
    return result,state

def saved_page_result(out,number,task,config):
    """Admit an unchanged cached page before work, never repair or overwrite it.

    A parent failure takes precedence over worker bytes. Success requires the
    actual saved source image, complete raw inventory and current runtime;
    unresolved partial pages require an explicit recovery decision.
    """
    out=Path(out)
    def check(ok,reason):
        if not ok:raise ValueError('SAVED_PAGE_'+reason)
    def confined(path):
        check(path.is_relative_to(out),'PATH_ESCAPE')
        check(not any(p.is_symlink() for p in (path,*path.parents) if p.is_relative_to(out)),'PATH_ESCAPE')
        check(path.resolve().is_relative_to(out.resolve()),'PATH_ESCAPE')
    directory=out/'raw'/f'page-{number:04}'
    confined(directory)
    if not directory.exists():return None
    check(directory.is_dir(),'PARTIAL_UNVERIFIED')
    parent=directory/'parent-result.json';worker=directory/'page-result.json'
    selected=parent if parent.exists() or parent.is_symlink() else worker
    confined(selected);check(selected.is_file(),'PARTIAL_UNVERIFIED')
    original_hash=sha(selected)
    try:p=read(selected)
    except (ValueError,OSError) as exc:raise ValueError('SAVED_PAGE_RESULT_UNREADABLE') from exc
    check(type(p) is dict and type(p.get('page')) is int and p['page']==number and p.get('source_sha256')==task['input_sha256'],'BINDING_MISMATCH')
    routes=dict(NATIVE_PRESERVED='native',VISIBLE_TEXT_REPLACE_REVIEW='existing',EXISTING_TEXT_REVIEW='existing',OCR_DRAFT='ocr',EMPTY='ocr',FAILED='failed',TIMEOUT='failed',CANCELLED='failed')
    check(type(p.get('status')) is str and p['status'] in routes and p.get('route')==routes[p['status']],'STATUS_INVALID')
    words=p.get('words');raw=p.get('raw_files')
    check(type(words) is list and type(raw) is dict,'RESULT_INVALID')
    check(type(p.get('review_reasons',[])) is list and all(type(v) is str for v in p.get('review_reasons',[])) and type(p.get('native_text','')) is str,'RESULT_INVALID')
    for w in words:
        check(type(w) is dict and type(w.get('id')) is str and type(w.get('text')) is str and type(w.get('bbox')) is list and len(w['bbox'])==4 and all(type(v) in (int,float) and math.isfinite(v) for v in w['bbox']),'WORDS_INVALID')
    check(len({w['id'] for w in words})==len(words),'WORDS_INVALID')
    check(p['route']!='failed' or not words,'FAILED_PAGE_WORDS')
    registered=set()
    for name,h in raw.items():
        check(type(name) is str and type(h) is str,'RAW_INVALID')
        rel=Path(name);check(not rel.is_absolute() and '..' not in rel.parts,'PATH_ESCAPE')
        path=out/rel;confined(path)
        check(path.is_relative_to(directory) and path!=selected and path.is_file(),'PATH_ESCAPE')
        check(sha(path)==h,'RAW_CHANGED');registered.add(path)
    actual=set()
    for path in directory.rglob('*'):
        confined(path)
        check(path.is_file() or path.is_dir(),'PATH_ESCAPE')
        if path.is_file() and path!=selected:actual.add(path)
    check(actual==registered,'RAW_INVENTORY_MISMATCH')
    if p['route']!='failed':
        image=p.get('image_path');check(type(image) is str,'IMAGE_BINDING')
        image=Path(image);confined(image)
        check(image.is_relative_to(directory) and image.is_file() and raw.get(str(image.relative_to(out)))==p.get('image_sha256')==sha(image),'IMAGE_BINDING')
        beforepath=directory/'runtime-version.json';afterpath=directory/'runtime-after.json'
        check(beforepath in registered and afterpath in registered,'RUNTIME_UNVERIFIED')
        try:before=read(beforepath);after=read(afterpath)
        except (ValueError,OSError) as exc:raise ValueError('SAVED_PAGE_RUNTIME_UNVERIFIED') from exc
        check(type(before) is dict and type(after) is dict and type(before.get('tessdata')) is list,'RUNTIME_UNVERIFIED')
        check(after.get('unchanged') is True and after.get('before_sha256')==sha(beforepath) and after.get('identity')==before,'RUNTIME_UNVERIFIED')
        check(before.get('config')==config,'CONFIG_MISMATCH')
        try:current=worker_runtime_identity(config,before['tessdata'])
        except (OSError,ValueError,KeyError,TypeError) as exc:raise ValueError('SAVED_PAGE_RUNTIME_UNVERIFIED') from exc
        check(before==current,'RUNTIME_MISMATCH')
    try:invariant(dict(schema_version=1,pages=[p],pages_hash=digest([p])))
    except (ValueError,KeyError,TypeError) as exc:raise ValueError('SAVED_PAGE_MEMBERS_INVALID') from exc
    check(sha(selected)==original_hash,'RESULT_CHANGED')
    return p

@contextmanager
def task_producer(out):
    """One producer for the entire task, including validation and publication.

    Lock the immutable job inode without changing saved bytes or creating a
    stale PID marker. The OS releases ownership on exit. Workers do not inherit
    the descriptor; a second producer must fail before touching progress/raw.
    """
    path=Path(out)/'job.json'
    if path.is_symlink():raise ValueError('TASK_JOB_PATH_SYMLINK')
    if not path.exists():raise FileExistsError('Use a new output directory or identical resumable task')
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        info=os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or info.st_uid!=os.getuid():raise ValueError('TASK_JOB_FILE_UNSAFE')
        try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError as exc:raise ValueError('TASK_PRODUCER_BUSY') from exc
        current=path.lstat()
        if (current.st_dev,current.st_ino)!=(info.st_dev,info.st_ino) or current.st_nlink!=1:raise ValueError('TASK_JOB_FILE_CHANGED')
        yield
    finally:os.close(fd)

def _reused_page_state(out,task,config,job,*,validate_results=True):
    """Bound derivative cache and inherited review history, never old raw edits."""
    path=Path(out)/'reused-pages.json';expected=job.get('reuse_manifest_sha256')
    if expected is None:
        if path.exists() or path.is_symlink():raise ValueError('REUSED_PAGE_MANIFEST_UNBOUND')
        return {},None
    if path.is_symlink() or not path.is_file() or sha(path)!=expected:raise ValueError('REUSED_PAGE_MANIFEST_CHANGED')
    value=read(path)
    if (type(value) is not dict or value.get('schema')!='local-completed-page-reuse/1' or
        value.get('task_identity')!=digest(dict(task=task,config=config)) or
        value.get('source_sha256')!=task['input_sha256'] or type(value.get('page_results')) is not dict or
        type(value.get('inherited_review')) is not dict):raise ValueError('REUSED_PAGE_BINDING_MISMATCH')
    hashes={}
    for name,h in value['page_results'].items():
        if not name.isdecimal() or str(int(name))!=name or int(name) not in task['page_numbers'] or type(h) is not str:
            raise ValueError('REUSED_PAGE_BINDING_MISMATCH')
        number=int(name)
        if validate_results:p=saved_page_result(out,number,task,config)
        else:
            # Historical viewing verifies the pinned derivative record; it
            # never rebinds an old draft to the currently installed runtime.
            resultpath=Path(out)/'raw'/f'page-{number:04d}'/'page-result.json'
            if resultpath.is_symlink() or not resultpath.is_file():raise ValueError('REUSED_PAGE_RESULT_CHANGED')
            p=read(resultpath)
        if p is None or p['route']=='failed' or digest(p)!=h:raise ValueError('REUSED_PAGE_RESULT_CHANGED')
        hashes[number]=h
    return hashes,value['inherited_review']

def _validate_saved_checkpoint(out,pages,cached,reused):
    if not cached:return
    path=Path(out)/'progress.json'
    if path.is_symlink() or not path.is_file():raise ValueError('SAVED_PAGE_CHECKPOINT_UNVERIFIED')
    try:progress=read(path)
    except (ValueError,OSError) as exc:raise ValueError('SAVED_PAGE_CHECKPOINT_UNVERIFIED') from exc
    if (type(progress) is not dict or progress.get('schema_version')!=1 or
        type(progress.get('completed')) is not int or type(progress.get('page_results')) is not list or
        progress.get('total')!=len(pages) or progress['completed']!=len(progress['page_results']) or
        not (0 if reused else 1)<=progress['completed']<=len(pages)):
        raise ValueError('SAVED_PAGE_CHECKPOINT_UNVERIFIED')
    rows=progress['page_results']
    if any(type(p) is not dict or type(p.get('page')) is not int for p in rows) or [p['page'] for p in rows]!=pages[:len(rows)]:
        raise ValueError('SAVED_PAGE_CHECKPOINT_UNVERIFIED')
    recorded={p['page']:p for p in rows}
    if (set(cached)!=(set(recorded)|set(reused)) or
        any(digest(cached[n])!=digest(p) for n,p in recorded.items()) or
        any(digest(cached[n])!=h for n,h in reused.items())):
        raise ValueError('SAVED_PAGE_CHECKPOINT_MISMATCH')

def run_task(task,config=None,*,before_processing=None):
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
    if Path(task['output_directory']).is_symlink():raise ValueError('TASK_OUTPUT_PATH_SYMLINK')
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
    existing=out.exists()
    if not existing:
        out.mkdir(parents=True)
        write(out/'job.json',dict(schema_version=1,identity=identity,task=task,config=config))
    with task_producer(out):
        return _run_task_owned(task,config,source,out,identity,existing,before_processing=before_processing)

def _run_task_owned(task,config,source,out,identity,existing,*,before_processing=None):
    """Internal processing path; run_task holds exclusive job ownership."""
    pages=task['page_numbers'];cached={};reused={};inherited=None
    if existing:
        if (out/'job.json').is_symlink():raise ValueError('TASK_JOB_PATH_SYMLINK')
        if not (out/'job.json').exists():raise FileExistsError('Use a new output directory or identical resumable task')
        job=read(out/'job.json')
        if type(job) is not dict or job.get('identity')!=identity:raise FileExistsError('Use a new output directory or identical resumable task')
        if job.get('schema_version')!=1 or job.get('task')!=task or job.get('config')!=config:raise ValueError('TASK_JOB_BINDING_MISMATCH')
        reused,inherited=_reused_page_state(out,task,config,job)
        if (out/'worker-cleanup-failure.json').exists():raise PageWorkerCleanupError('PAGE_WORKER_CLEANUP_UNVERIFIED')
        if (out/'CURRENT.json').is_symlink():raise ValueError('TASK_CURRENT_PATH_SYMLINK')
        if (out/'CURRENT.json').exists():
            if before_processing is not None:raise ValueError('TASK_ALREADY_PUBLISHED')
            snapshot,folder=load_snapshot(out)
            if (snapshot['input_sha256']!=task['input_sha256'] or Path(snapshot['source_pdf']).resolve()!=source or
                snapshot.get('operation_id')!=task['operation_id'] or snapshot.get('mode')!=task['mode'] or
                snapshot.get('config_version')!=task['config_version'] or [p['page'] for p in snapshot['pages']]!=pages):
                raise ValueError('CURRENT_TASK_BINDING_MISMATCH')
            return load_task_completion(out,snapshot,folder)[0]
        # Validate every existing selected page before mutating job/progress or
        # starting even an earlier missing page. A partial raw directory is not
        # permission to overwrite observations or silently retry recognition.
        for number in pages:
            p=saved_page_result(out,number,task,config)
            if p is not None:cached[number]=p
        _validate_saved_checkpoint(out,pages,cached,reused)
    # An explicit desktop continuation may register a new client attempt only
    # after all saved evidence checks, while this process owns the job lease.
    if before_processing is not None:before_processing(dict(cached))
    (out/'raw').mkdir(exist_ok=True)
    results=[];started=time.monotonic();cancelled=False
    for number in pages:
        if (out/'CANCEL').exists():cancelled=True;break
        resultpath=out/'raw'/f'page-{number:04}'/'page-result.json'
        parentpath=resultpath.with_name('parent-result.json')
        if number in cached:p=cached[number]
        else:
            request=dict(input_pdf=str(source),input_sha256=task['input_sha256'],page=number,mode=task['mode'],output=str(out),config=config)
            requestpath=out/'raw'/f'worker-{number}.json';write(requestpath,request)
            proc=None;stdout=stderr='';lifeline_read=lifeline_write=None
            try:
                try:
                    lifeline_read,lifeline_write=os.pipe()
                    proc=subprocess.Popen([sys.executable,'-m','scripts.ocr.mvp.local','--parent-lifeline-fd',str(lifeline_read),'--worker',str(requestpath)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=ROOT,start_new_session=True,pass_fds=(lifeline_read,))
                    os.close(lifeline_read);lifeline_read=None
                    deadline=time.monotonic()+config['page_timeout_seconds']
                    while True:
                        if (out/'CANCEL').exists():
                            cancelled=True
                            if proc.returncode is None:
                                try:os.killpg(proc.pid,signal.SIGKILL)
                                except ProcessLookupError:pass
                            stdout,stderr=proc.communicate(timeout=5);break
                        remaining=deadline-time.monotonic()
                        if remaining<=0:raise subprocess.TimeoutExpired(proc.args,config['page_timeout_seconds'],output=stdout,stderr=stderr)
                        try:
                            stdout,stderr=proc.communicate(timeout=min(.25,remaining));break
                        except subprocess.TimeoutExpired as exc:
                            stdout=pipe_text(exc.output);stderr=pipe_text(exc.stderr)
                finally:
                    try:
                        if proc is not None:reap_page_worker(proc)
                    finally:
                        for fd in (lifeline_read,lifeline_write):
                            if fd is not None:os.close(fd)
                (out/'raw'/f'worker-{number}.stdout').write_text(stdout);(out/'raw'/f'worker-{number}.stderr').write_text(stderr)
                if cancelled:
                    p=dict(page=number,status='CANCELLED',route='failed',error='USER_CANCELLED',words=[],source_sha256=task['input_sha256'])
                    parent_page_result(out,resultpath,p)
                else:
                    if not resultpath.exists():raise RuntimeError('PAGE_WORKER_FAILED')
                    p=read(resultpath)
            except PageWorkerCleanupError as exc:
                p=dict(page=number,status='FAILED',route='failed',error=repr(exc),words=[],source_sha256=task['input_sha256'])
                write(out/'worker-cleanup-failure.json',dict(operation_id=task['operation_id'],page=number,
                      worker_pid=proc.pid if proc is not None else None,source_sha256=task['input_sha256'],
                      error=repr(exc),cause=repr(exc.__cause__),resume_blocked=True))
                parent_page_result(out,resultpath,p)
                raise
            except subprocess.TimeoutExpired as exc:
                (out/'raw'/f'worker-{number}.stdout').write_text(pipe_text(exc.output));(out/'raw'/f'worker-{number}.stderr').write_text(pipe_text(exc.stderr))
                p=dict(page=number,status='FAILED',route='failed',error='PAGE_TIMEOUT',words=[],wall_seconds=config['page_timeout_seconds'],source_sha256=task['input_sha256'])
                parent_page_result(out,resultpath,p)
            except Exception as exc:
                p=dict(page=number,status='FAILED',route='failed',error=repr(exc),words=[],source_sha256=task['input_sha256'])
                parent_page_result(out,resultpath,p)
        results.append(p);write(out/'progress.json',dict(schema_version=1,completed=len(results),total=len(pages),page_results=results));print(f'page {number}: {p["status"]}',file=sys.stderr,flush=True)
        if cancelled:break
    # Cancellation preserves unprocessed source pages in output and records their state.
    for number in pages[len(results):]:results.append(cached.get(number,dict(page=number,status='CANCELLED',route='failed',words=[])))
    # A remaining page may take time. Revalidate cached bytes before producing
    # a draft, without refreshing manifests or substituting a changed result.
    for number,p in cached.items():
        if digest(saved_page_result(out,number,task,config))!=digest(p):raise ValueError('SAVED_PAGE_RESULT_CHANGED')
    snapshot=dict(schema_version=1,operation_id=task['operation_id'],mode=task['mode'],source_pdf=str(source),input_sha256=task['input_sha256'],revision=0,pages=results,receipts=[],config_version=CONFIG_VERSION,consumer_policy=CONSUMER_POLICY,font_path=config['font'],fallback_font_paths=config['fallback_font_paths'])
    if inherited is not None:snapshot['inherited_review']=inherited
    old.FONT=Path(config['font']);save_start=time.monotonic();folder=publish(out,snapshot)
    artifacts={k:str(folder/n) for k,n in [('searchable_pdf','searchable.pdf'),('text','text.txt'),('pages_json','pages.json'),('review_html','review.html')]};artifacts['raw_directory']=str(out/'raw');artifacts['page_mapping_json']=str(folder/'page-map.json')
    status='cancelled' if cancelled else ('failed' if all(p['status']=='FAILED' for p in results) else 'review_required')
    envelope=dict(schema_version=1,operation_id=task['operation_id'],mode=task['mode'],status=status,input_sha256=task['input_sha256'],output_directory=str(out),artifacts=artifacts,page_results=results,review_required=True,timings=dict(total_seconds=time.monotonic()-started,save_seconds=time.monotonic()-save_start),usage=dict(cloud_requests=0,downloads=0),evidence=[str(out/'job.json')],revision=0,export_review=snapshot.get('export_review',[]),**{key:snapshot[key] for key in ('source_page_count','exported_page_count','untouched_page_numbers','pdf_page_mapping')})
    write(out/'completion.json',envelope)
    if sha(source)!=task['input_sha256']:raise ValueError('SOURCE_CHANGED')
    return envelope

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--worker');parser.add_argument('--parent-lifeline-fd',type=int);args=parser.parse_args()
    if args.worker:
        with worker_parent_lifeline(args.parent_lifeline_fd):worker(read(args.worker))
if __name__=='__main__':main()

def cancel_task(output_directory):
    """Cooperative cancel signal, checked at <=250ms while a worker is active."""
    out=Path(output_directory).resolve()
    if not (out/'job.json').is_file():raise ValueError('UNKNOWN_OPERATION')
    (out/'CANCEL').touch()
    return dict(schema_version=1,status='cancellation_requested',output_directory=str(out))
