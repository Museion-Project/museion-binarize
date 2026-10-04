"""Fixed, structured desktop bridge. Every invocation is local and network denied."""
import hashlib,json,os,shutil,socket,sqlite3,sys,uuid,unicodedata,tempfile
from pathlib import Path

def denied(*a,**k):raise OSError('APP_MVP_NETWORK_DISABLED: exact future upload approval required')
socket.create_connection=denied
socket.socket.connect=denied
socket.getaddrinfo=denied

def read(p):return json.loads(Path(p).read_text())
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,x):Path(p).write_text(json.dumps(x,ensure_ascii=False,indent=2))
def check(ok,msg):
 if not ok:raise ValueError(msg)
def atomic_save_copy(src,dest,*,validate,receipt_path,result,expected_sha256=None):
 """Publish a complete checked copy with exclusive hard links, never replace.

 Both staging files are on their destination volumes. On an ordinary failure
 remove only links still identifying our own unchanged file. A process crash
 after PDF commit can leave a complete PDF before its receipt, never a partial
 PDF; the durable prepared journal keeps that outcome locatable.
 """
 src,dest,receipt_path=Path(src),Path(dest),Path(receipt_path)
 expected=expected_sha256 or sha(src);owned=[];temporaries=[];journal=None
 def stage(parent,prefix):
  fd,name=tempfile.mkstemp(prefix=prefix,dir=parent);temporaries.append(Path(name));return fd,Path(name)
 def sync_json(path,value):
  with path.open('x',encoding='utf-8') as out:
   json.dump(value,out,ensure_ascii=False,indent=2);out.flush();os.fsync(out.fileno())
 def sync_dir(path):
  fd=os.open(path,os.O_RDONLY)
  try:os.fsync(fd)
  finally:os.close(fd)
 try:
  fd,tmp=stage(dest.parent,'.museion-save-')
  with os.fdopen(fd,'wb') as out,src.open('rb') as inp:
   shutil.copyfileobj(inp,out);out.flush();os.fsync(out.fileno())
  check(sha(tmp)==expected and sha(src)==expected,'DERIVED_OUTPUT_CHANGED')
  validate()
  result=dict(result,output_pdf=str(dest),sha256=expected,save_policy='atomic-exclusive-copy-v1')
  fd,staged_receipt=stage(receipt_path.parent,'.museion-receipt-');os.close(fd)
  with staged_receipt.open('w',encoding='utf-8') as out:
   json.dump(result,out,ensure_ascii=False,indent=2);out.flush();os.fsync(out.fileno())
  journal=receipt_path.with_name('save-pending-'+uuid.uuid4().hex+'.json')
  stat=tmp.stat()
  sync_json(journal,dict(schema='save-pending/2',state='PREPARED',result=result,receipt_path=str(receipt_path),
                         source_pdf=str(src),target_identity=dict(device=stat.st_dev,inode=stat.st_ino,size=stat.st_size),note='recovery evidence only; not a successful save receipt'))
  sync_dir(journal.parent)
  # Revalidate immediately before the exclusive commit. No overwrite race.
  validate();check(sha(src)==expected,'DERIVED_OUTPUT_CHANGED')
  os.link(tmp,dest);owned.append((dest,tmp,expected))
  os.link(staged_receipt,receipt_path);owned.append((receipt_path,staged_receipt,sha(staged_receipt)))
  sync_dir(dest.parent);sync_dir(receipt_path.parent)
  journal.unlink();journal=None
  return result
 except BaseException:
  for final,staged,h in reversed(owned):
   try:
    # Do not delete an output another process replaced or modified.
    if final.samefile(staged) and sha(final)==h:final.unlink()
   except FileNotFoundError:pass
  raise
 finally:
  for tmp in temporaries:tmp.unlink(missing_ok=True)
  # Failed journals are deliberately retained: cleanup/recovery must inspect
  # final hash and receipt identity, not silently retry or overwrite.
def source_bound(source,h,request):
 check(sha(source)==h==request['input_sha256'],'SOURCE_HASH_MISMATCH')
 check(Path(source).resolve()==Path(request['input_pdf']).resolve(),'SOURCE_PATH_MISMATCH')
def packaged_config(root,session_root,legacy_session_root=None):
 root=Path(root).resolve();manifest=read(root/'runtime-manifest.json')
 check(manifest.get('schema')=='museion-local-runtime/1','RESOURCE_MANIFEST_SCHEMA')
 from .runtime_contract import verify_inventory
 inventory=verify_inventory(root,manifest)
 import ast
 def constant(name,key):
  tree=ast.parse((root/name).read_text())
  return next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id==key for t in n.targets))
 check(manifest.get('config_version')==constant('scripts/ocr/mvp/core.py','CONFIG_VERSION'),'RESOURCE_CONFIG_VERSION_MISMATCH')
 check(manifest.get('exporter_version')==constant('scripts/ocr/mvp/store.py','EXPORTER_VERSION'),'RESOURCE_EXPORTER_VERSION_MISMATCH')
 def resource(name):
  relative=Path(manifest[name]);check(not relative.is_absolute() and '..' not in relative.parts,'RESOURCE_PATH_ESCAPE')
  path=(root/relative).resolve();check(path.is_relative_to(root) and path.is_file(),'RESOURCE_MISSING: '+name)
  return str(path)
 local={k:resource(k) for k in ('apple_helper','tesseract','font')}
 local.update(page_timeout_seconds=120,max_residual_regions=32,text_layer_policy='preserve',fallback_font_paths=['/System/Library/Fonts/Times.ttc'])
 tess=root/manifest['tessdata'];check(tess.resolve().is_relative_to(root) and tess.is_dir(),'TESSDATA_MISSING')
 return dict(python=resource('python'),package_root=str(root),session_root=str(Path(session_root).resolve()),tessdata=str(tess),local=local,external_development_runtime=False,local_only=True,runtime_inventory_verified=True,resource_inventory_proof=inventory,resource_manifest_sha256=sha(root/'runtime-manifest.json'),persistent_sessions=True,require_runtime_binding=True,require_document_binding=True,legacy_session_roots=[str(Path(legacy_session_root).resolve())] if legacy_session_root else [])

def context(request,config):
 from .sessions import roots,folder_for,mutation_guard
 sid=request.get('session_id')
 if sid:
  folder=folder_for(sid,config);check(not (folder/'desktop-session.json').is_symlink(),'SESSION_METADATA_SYMLINK');meta=read(folder/'desktop-session.json')
  check(meta['mode']==request['mode'],'SESSION_MODE_MISMATCH')
  source_bound(meta['source'],meta['source_sha256'],request)
  if meta['mode']=='local' and request.get('action') in ('review','save','recover-save','continue'):mutation_guard(meta,config)
  return folder,meta
 return roots(config,create=True)[0],None

def paid_receipt(folder):
 db=sqlite3.connect(f'file:{folder / "state.sqlite"}?mode=ro',uri=True)
 try:
  row=db.execute('SELECT revision,data FROM reviews ORDER BY revision DESC LIMIT 1').fetchone()
  return json.loads(row[1]) if row else dict(revision=0,adopted={})
 finally:db.close()

def incomplete_local_view(folder,meta,result,config):
 """Inspect durable pages before publication. Never infer worker liveness or resume work."""
 import fitz
 import math
 from scripts.ocr.mvp.core import digest
 from .sessions import compatibility
 op=folder/'operation'
 def confined(path):
  check(path.is_relative_to(op) and not any(p.is_symlink() for p in (path,*path.parents) if p.is_relative_to(op)), 'INCOMPLETE_PATH_ESCAPE')
  check(path.resolve().is_relative_to(op.resolve()),'INCOMPLETE_PATH_ESCAPE')
 confined(op);confined(op/'job.json')
 job=read(op/'job.json');check(type(job) is dict,'INCOMPLETE_JOB_INVALID')
 task=job.get('task');stored=job.get('config')
 check(type(task) is dict and type(stored) is dict and job.get('schema_version')==1 and job.get('identity')==digest(dict(task=task,config=stored)), 'INCOMPLETE_JOB_IDENTITY')
 check(task.get('mode')=='local' and task.get('operation_id')==folder.name and Path(task.get('output_directory','')).resolve()==op.resolve(), 'INCOMPLETE_TASK_SCOPE')
 check(task.get('input_sha256')==meta['source_sha256'] and Path(task.get('input_pdf','')).resolve()==Path(meta['source']).resolve(), 'INCOMPLETE_SOURCE_MISMATCH')
 check(sha(meta['source'])==meta['source_sha256'],'SOURCE_HASH_MISMATCH')
 pages=task.get('page_numbers')
 check(type(pages) is list and 1<=len(pages)<=40 and all(type(n) is int for n in pages) and len(set(pages))==len(pages),'INCOMPLETE_PHYSICAL_PAGES')
 with fitz.open(meta['source']) as doc:
  check(not doc.needs_pass and all(1<=n<=len(doc) for n in pages),'INCOMPLETE_PHYSICAL_PAGES')
  binding=meta.get('document_binding')
  if binding:
   check(binding.get('source_sha256')==meta['source_sha256'] and Path(binding.get('source_pdf','')).resolve()==Path(meta['source']).resolve() and binding.get('source_page_count')==len(doc) and binding.get('physical_pages')==pages,'INCOMPLETE_DOCUMENT_BINDING')
 runtime=meta.get('runtime_binding')
 if runtime:check(runtime.get('local_config')==stored and runtime.get('config_version')==task.get('config_version'),'INCOMPLETE_CONFIG_MISMATCH')
 observed=[]
 for number in pages:
  directory=op/'raw'/f'page-{number:04d}'
  placeholder=dict(page=number,status='NOT_PROCESSED',source_sha256=meta['source_sha256'],words=[],review_reasons=['No saved page result; processing state is unverified'])
  try:
   confined(directory)
   parent=directory/'parent-result.json';worker=directory/'page-result.json'
   # An incomplete/malformed parent failure is not replaced by worker success.
   selected=parent if parent.exists() or parent.is_symlink() else worker
   confined(selected)
   if selected.exists():
    page=read(selected)
    check(type(page) is dict and type(page.get('page')) is int and page['page']==number and page.get('source_sha256')==meta['source_sha256'],'INCOMPLETE_PAGE_BINDING')
    check(page.get('status') in ('NATIVE_PRESERVED','VISIBLE_TEXT_REPLACE_REVIEW','EXISTING_TEXT_REVIEW','OCR_DRAFT','EMPTY','FAILED','TIMEOUT','CANCELLED'),'INCOMPLETE_PAGE_STATUS')
    words=page.get('words');raw=page.get('raw_files')
    check(type(words) is list and type(raw) is dict,'INCOMPLETE_PAGE_INVALID')
    check(type(page.get('review_reasons',[])) is list and all(type(v) is str for v in page.get('review_reasons',[])) and type(page.get('native_text','')) is str,'INCOMPLETE_PAGE_INVALID')
    for word in words:
     check(type(word) is dict and type(word.get('id')) is str and type(word.get('text')) is str and type(word.get('bbox')) is list and len(word['bbox'])==4 and all(type(v) in (int,float) and math.isfinite(v) for v in word['bbox']),'INCOMPLETE_WORD_INVALID')
    check(len({w['id'] for w in words})==len(words),'INCOMPLETE_WORD_INVALID')
    for name,h in raw.items():
     check(type(name) is str and type(h) is str,'INCOMPLETE_RAW_INVALID')
     relative=Path(name);check(not relative.is_absolute() and '..' not in relative.parts,'INCOMPLETE_RAW_SCOPE')
     path=op/relative;confined(path)
     check(path.is_relative_to(directory) and path!=selected and path.is_file() and sha(path)==h,'INCOMPLETE_RAW_CHANGED')
    if page.get('image_path'):
     image=Path(page['image_path']);confined(image)
     check(image.is_relative_to(directory) and type(page.get('image_sha256')) is str and image.is_file() and raw.get(str(image.relative_to(op)))==page['image_sha256']==sha(image),'INCOMPLETE_IMAGE_BINDING')
    elif page['status'] not in ('FAILED','TIMEOUT','CANCELLED'):
     raise ValueError('INCOMPLETE_IMAGE_MISSING')
    check(not words or bool(raw),'INCOMPLETE_RAW_MISSING')
    placeholder=page
   elif directory.exists():
    placeholder.update(status='UNVERIFIED',review_reasons=['Page files exist without a validated result'])
  except (ValueError,OSError,KeyError,TypeError) as exc:
   placeholder.update(status='UNVERIFIED',review_reasons=['Saved page result is unverified: '+str(exc)])
  observed.append(placeholder)
 check(sha(meta['source'])==meta['source_sha256'],'SOURCE_HASH_MISMATCH')
 state=compatibility(meta,config or {})
 result.update(status='processing_unverified',draft_published=False,revision=None,pages=observed,
               completion_record_state='not_published',export_review=[],receipts=[],pending_saves=[],
               document_binding=meta.get('document_binding'),runtime_compatible=state=='verified',
               runtime_binding_status=state,session_storage=meta.get('session_storage','legacy-temporary'))
 from .sessions import continuation_state,current_attempt
 available=(state=='verified' and any(p['status']=='NOT_PROCESSED' for p in observed) and
            all(p['status'] not in ('UNVERIFIED','FAILED','TIMEOUT','CANCELLED') for p in observed) and
            not any((op/name).exists() or (op/name).is_symlink() for name in ('CANCEL','cancel','worker-cleanup-failure.json','completion.json')))
 result['continuation_available']=False
 if available:
  try:
   current_attempt(folder,meta)
   result.update(continuation_available=True,continuation_state_sha256=continuation_state(folder))
  except (ValueError,OSError,TypeError,KeyError):pass
 return result

def view(folder,meta,config=None):
 mode=meta['mode'];result=dict(mode=mode,session_id=folder.name,provenance=meta['provenance'],source_sha256=meta['source_sha256'],ready=False,quality_ready=False,review_required=True,network_requests=0)
 if mode in ('local','critical-edition'):
  current=folder/'operation/CURRENT.json'
  if mode=='local':check(not current.is_symlink() and not (folder/'operation').is_symlink(),'LOCAL_DRAFT_PATH_SYMLINK')
  if mode=='local' and not current.exists() and not current.is_symlink():
   return incomplete_local_view(folder,meta,result,config)
  from scripts.ocr.mvp.store import load_snapshot
  snap,revision=load_snapshot(folder/'operation')
  result.update(revision=snap['revision'],pages=snap['pages'],export_review=snap.get('export_review',[]),output_pdf=str(revision/'searchable.pdf'),receipts=snap.get('receipts',[]),document_binding=meta.get('document_binding'))
  result['status']=read(folder/'operation/completion.json').get('status','review_required') if mode!='local' and (folder/'operation/completion.json').is_file() else 'review_required'
  if mode=='local':
   from scripts.ocr.mvp.store import reader_alternatives
   check(snap['input_sha256']==meta['source_sha256'] and Path(snap['source_pdf']).resolve()==Path(meta['source']).resolve() and all(page.get('source_sha256',snap['input_sha256'])==snap['input_sha256'] for page in snap['pages']),'SNAPSHOT_SOURCE_MISMATCH')
   from scripts.ocr.mvp.local import load_task_completion
   completion,completion_state=load_task_completion(folder/'operation',snap,revision)
   result.update(status=completion['status'],completion_record_state=completion_state,draft_published=True)
   result['reader_alternatives']=reader_alternatives(snap)
   from .sessions import compatibility
   from .save_recovery import inspect
   binding_status=compatibility(meta,config or {})
   result.update(runtime_compatible=binding_status=='verified',runtime_binding_status=binding_status,session_storage=meta.get('session_storage','legacy-temporary'))
   result['pending_saves']=inspect(folder,meta,result)
 elif mode=='paid':
  op=folder/'operation';completion=read(op/'result.json');task=read(op/'task.json');receipt=paid_receipt(op);pages=read(op/'pages-v0.json') if (op/'pages-v0.json').exists() else []
  errors=[]
  for f in (op/'raw').glob('*/receipt.json'):
   raw=read(f)
   if raw.get('error') or (raw.get('http_status') or 0)>=400:
    response=raw.get('response') if isinstance(raw.get('response'),dict) else {}
    errors.append(dict(request_id=f.parent.name,http_status=raw.get('http_status'),error=raw.get('error'),message=response.get('message'),code=response.get('code'),unsettled_reserve_usd=raw.get('unsettled_reserve_usd'),no_retry=True))
  result.update(revision=receipt['revision'],pages=pages,adopted=receipt.get('adopted',{}),completion=completion,status=completion['status'],output_pdf=completion.get('artifacts',{}).get('searchable_pdf'),error=completion.get('message') or completion.get('error'),provider_errors=errors)
 else:
  table=read(folder/meta['table']) if meta.get('table') else None
  completion=read(folder/'import-completion.json') if (folder/'import-completion.json').exists() else {}
  result.update(revision=table['revision'] if table else 0,table=table,completion=completion,status=completion.get('status','review_required'),output_pdf=completion.get('artifacts',{}).get('bookmarks_pdf'),error=completion.get('error') or completion.get('message') or completion.get('error_code'),diagnostic_only=meta.get('diagnostic_only',not bool(table)),directory_blocker=meta.get('directory_blocker'),decision=(table or {}).get('admission') or completion.get('decision'),export_locked=meta.get('diagnostic_only',not bool(table)) or ((table or {}).get('admission') or completion.get('decision') or {}).get('export_locked',False))
 return result

def register_directory_table(path,folder,table,meta):
 # Legacy/missing ledger results remain inspectable diagnostics, never saveable.
 meta['diagnostic_only']=True
 meta['directory_blocker']='LEGACY_OR_MISSING_GUARD: diagnostic only; review/save disabled'
 if table.get('admission'):
  from scripts.bookmarks.paid_mvp.engine import _verify_admission_path
  try:
   _verify_admission_path(path,table)
   for revision in range(table['revision']+1):
    ledger=Path(path).parent/f'admission-r{revision:03d}.json'
    shutil.copy2(ledger,folder/ledger.name)
   for name in ('source-coverage.json','raw-model.json'):shutil.copy2(Path(path).parent/name,folder/name)
   meta['diagnostic_only']=False;meta.pop('directory_blocker',None)
  except ValueError as error:meta['directory_blocker']=str(error)
 meta['table']=f'draft-r{table["revision"]:03d}.json';write(folder/meta['table'],table)

def register_local_operation(folder,task,config):
 # Only bridge-owned NEW session directories: never mkdir from cancel or overwrite artifacts.
 from scripts.ocr.mvp.local import runtime_config
 from scripts.ocr.mvp.core import digest
 op=folder/'operation';op.mkdir(exist_ok=False)
 normalized=runtime_config(config)
 write(op/'job.json',dict(schema_version=1,identity=digest(dict(task=task,config=normalized)),task=task,config=normalized))
 # A.run_task independently repeats source/page/config/identity validation.
 return op

def main(request,config):
 mode=request.get('mode');action=request.get('action')
 check(mode in ('local','critical-edition','paid','paid-contents'),'EXPLICIT_MODE_REQUIRED')
 if config.get('local_only'):check(mode=='local' and action not in ('import','preflight'),'LOCAL_CANDIDATE_ONLY')
 check(action in ('readiness','start','cancel','continue','import','resume','reload','review','save','search','preflight','recover-save'),'ACTION_NOT_ALLOWED')
 if action=='continue':
  check(mode=='local' and type(request.get('session_id')) is str,'LOCAL_CONTINUATION_SESSION_REQUIRED')
  token=request.get('client_operation_id');challenge=request.get('continuation_state_sha256')
  check(type(token) is str and len(token)==32 and all(c in '0123456789abcdef' for c in token),'CONTINUATION_OPERATION_REQUIRED')
  check(type(challenge) is str and len(challenge)==64 and all(c in '0123456789abcdef' for c in challenge),'CONTINUATION_STATE_REQUIRED')
  check('pages' not in request,'CONTINUATION_PAGES_FIXED')
 if action=='start' and mode=='local':
  # Validate the actual PDF before creating session/task state or any worker.
  # Rust holds the shared document lease; hash revalidation also catches edits
  # to the source while the packaged runtime was being checked.
  import fitz
  source=Path(request['input_pdf'])
  source_bound(source,request['input_sha256'],request)
  pages=request.get('pages')
  check(type(pages) is list and 1<=len(pages)<=40 and all(type(n) is int for n in pages),'INVALID_PHYSICAL_PAGES')
  check(len(set(pages))==len(pages),'DUPLICATE_PHYSICAL_PAGES')
  with fitz.open(source) as doc:
   count=len(doc);check(not doc.needs_pass,'ENCRYPTED_SOURCE_UNSUPPORTED')
   check(all(1<=n<=count for n in pages),'INVALID_PHYSICAL_PAGES')
  source_bound(source,request['input_sha256'],request)
  if config.get('require_document_binding'):
   check(type(request.get('document_id')) is str and bool(request['document_id']),'DOCUMENT_BINDING_REQUIRED')
   check(type(request.get('document_page_count')) is int and request['document_page_count']==count,'DOCUMENT_PAGE_COUNT_CHANGED')
 if action=='readiness':
  if mode=='local':
   from scripts.ocr.mvp.local import readiness
   result=readiness('local',config['local'])
  elif mode=='critical-edition':
   from scripts.ocr.edition_mvp.local import readiness
   result=readiness('critical-edition',config['critical-edition'])
  elif mode=='paid':
   from scripts.ocr.paid_mvp.pipeline import readiness
   result=readiness()
  else:
   from scripts.bookmarks.paid_mvp.engine import readiness
   result=readiness()
  result.update(desktop_entry_enabled=True,distribution_ready=False,quality_ready=False,ready=False,network_send_enabled=False,external_development_runtime=config.get('external_development_runtime',True))
  return result
 if action=='resume':
  check(mode=='local','LOCAL_RESUME_ONLY')
  from .sessions import roots
  matches=[]
  for root in roots(config):
   if root.exists():
    for candidate in root.glob('*/desktop-session.json'):
     try:
      if candidate.parent.is_symlink() or candidate.is_symlink():continue
      existing=read(candidate)
      op=candidate.parent/'operation'
      if existing.get('mode')=='local' and existing.get('source_sha256')==request['input_sha256'] and Path(existing['source']).resolve()==Path(request['input_pdf']).resolve() and not op.is_symlink() and ((op/'CURRENT.json').is_file() or (op/'job.json').is_file()):matches.append(candidate)
     except (ValueError,OSError,KeyError):continue
  check(bool(matches),'NO_LOCAL_DRAFT: no matching previous draft for this original PDF')
  latest=max(matches,key=lambda p:p.stat().st_mtime_ns);request=dict(request,session_id=latest.parent.name)
  folder,meta=context(request,config);return view(folder,meta,config)
 root,meta=context(request,config)
 if action=='start':check(mode in ('local','critical-edition'),'CLOUD_SEND_DISABLED: exact future upload approval required')
 if action in ('start','import'):
  check(not meta,'NEW_SESSION_REQUIRED');folder=root/uuid.uuid4().hex;folder.mkdir(mode=0o700)
  meta=dict(client_operation_id=request.get('client_operation_id'),mode=mode,source=request['input_pdf'],source_sha256=request['input_sha256'],provenance='fresh-local-app' if action=='start' else 'imported-existing-result')
  if mode=='local' and action=='start':
   from .sessions import runtime_binding
   meta.update(schema='desktop-local-session/2',runtime_binding=runtime_binding(config),session_storage='persistent-private' if config.get('persistent_sessions') else 'explicit-development-root')
   meta['document_binding']=dict(schema='local-document-request/1',document_id=request.get('document_id'),source_sha256=request['input_sha256'],source_pdf=str(Path(request['input_pdf']).resolve()),source_page_count=count,physical_pages=list(pages),binding_required=bool(config.get('require_document_binding')),source_checked_before_session=True)
  write(folder/'desktop-session.json',meta)
  if action=='start':
   check(mode in ('local','critical-edition'),'CLOUD_SEND_DISABLED: import an existing result; future exact manifest/endpoint/model/budget confirmation required')
   write(folder/'task-request.json',request)
   if mode=='local':
    from scripts.ocr.mvp.core import CONFIG_VERSION as task_config_version
   else:
    task_config_version='mvp-edition-v1'
   task=dict(operation_id=folder.name,input_pdf=request['input_pdf'],input_sha256=request['input_sha256'],page_numbers=request['pages'],mode=mode,output_directory=str(folder/'operation'),config_version=task_config_version)
   # Register identity before publishing sid: early cancellation cannot create a conflicting output tree.
   if mode=='local':register_local_operation(folder,task,config['local'])
   print(json.dumps(dict(event='session',session_id=folder.name,client_operation_id=request.get('client_operation_id'))),file=sys.stderr,flush=True)
   if mode=='local':
    from scripts.ocr.mvp.local import run_task
    run_task(task,config['local'])
   else:
    from scripts.ocr.edition_mvp.local import run_task
    run_task(task,config['critical-edition'])
  else:
   path=Path(request['import_path']).resolve();check(path.is_file(),'RESULT_FILE_MISSING');completion=read(path)
   if mode in ('local','critical-edition'):
    from scripts.ocr.mvp.store import load_snapshot
    snap,_=load_snapshot(path.parent);check(snap['mode']==mode or mode=='local' and snap['mode']=='apple-residual','IMPORTED_MODE_MISMATCH');meta['provenance']+=': observed '+snap['mode']+'; imported diagnostic only';source_bound(snap['source_pdf'],snap['input_sha256'],request)
    shutil.copytree(path.parent,folder/'operation')
    # Absolute raw/image paths remain pinned original observations; no recognition rerun.
   elif mode=='paid':
    check(completion.get('mode')=='paid','IMPORTED_MODE_MISMATCH');task=read(path.parent/'task.json');source_bound(task['input_pdf'],task['input_sha256'],request)
    shutil.copytree(path.parent,folder/'operation')
    # Imported result stays provenance; exports/reviews operate only in the session copy.
   elif completion.get('schema')=='mpdf-bookmark-table/1' or 'entries' in completion and 'source_sha256' in completion:
    source_bound(completion['source'],completion['source_sha256'],request)
    check(completion.get('config_version')=='paid-contents-v1','IMPORTED_MODE_MISMATCH');table=dict(completion);table['source']=request['input_pdf'];write(folder/f'draft-r{table["revision"]:03d}.json',table)
    register_directory_table(path,folder,table,meta)
   else:
    check(completion.get('mode')=='paid-contents','IMPORTED_MODE_MISMATCH');source_bound(request['input_pdf'],completion['input_sha256'],request)
    draft=completion.get('artifacts',{}).get('draft')
    if draft:
     table=read(draft);source_bound(table['source'],table['source_sha256'],request);register_directory_table(draft,folder,table,meta)
    write(folder/'import-completion.json',completion)
  if not (mode=='local' and action=='start'):write(folder/'desktop-session.json',meta)
  return view(folder,meta,config)
 folder=root
 if action=='continue':
  from .sessions import compatibility,publish_attempt
  from scripts.ocr.mvp.local import run_task
  check(compatibility(meta,config)=='verified','SESSION_RUNTIME_UNVERIFIED_OR_CHANGED')
  op=folder/'operation'
  check(not any((op/name).exists() or (op/name).is_symlink() for name in ('CURRENT.json','completion.json')),'TASK_ALREADY_PUBLISHED')
  observed=view(folder,meta,config)
  check(observed.get('continuation_available') is True,'CONTINUATION_NOT_AVAILABLE')
  task=read(op/'job.json')['task']
  if config.get('require_document_binding'):
   check(type(request.get('document_id')) is str and bool(request['document_id']),'DOCUMENT_BINDING_REQUIRED')
   check(type(request.get('document_page_count')) is int and request['document_page_count']==meta['document_binding']['source_page_count'],'DOCUMENT_PAGE_COUNT_CHANGED')
  def admitted(cached):
   check(all(p['status'] not in ('FAILED','TIMEOUT','CANCELLED') for p in cached.values()),'CONTINUATION_SAVED_FAILURE')
   remaining=[n for n in task['page_numbers'] if n not in cached]
   check(bool(remaining),'CONTINUATION_NO_REMAINING_PAGES')
   # A worker request/log without a result is still prior attempt evidence.
   # Preserve it and refuse, rather than silently overwrite or rerun the page.
   check(not any((op/'raw'/f'worker-{n}{suffix}').exists() or (op/'raw'/f'worker-{n}{suffix}').is_symlink()
                 for n in remaining for suffix in ('.json','.stdout','.stderr')),'CONTINUATION_PARTIAL_WORKER_EVIDENCE')
   check(compatibility(meta,config)=='verified','SESSION_RUNTIME_UNVERIFIED_OR_CHANGED')
   publish_attempt(folder,meta,task,request,cached)
   print(json.dumps(dict(event='session',session_id=folder.name,client_operation_id=request['client_operation_id'])),file=sys.stderr,flush=True)
  run_task(task,config['local'],before_processing=admitted)
  return view(folder,meta,config)
 if action=='cancel':
  if mode=='local':
   from .sessions import session_control,current_attempt,safe_record
   from scripts.ocr.mvp.local import cancel_task
   with session_control(folder):
    check(safe_record(folder/'desktop-session.json')==meta,'SESSION_METADATA_CHANGED')
    token=current_attempt(folder,meta)
    if request.get('client_operation_id') or (folder/'active-processing.json').exists():check(request.get('client_operation_id')==token,'CANCEL_OPERATION_MISMATCH')
    cancel_task(folder/'operation')
  else:
   if request.get('client_operation_id'):check(request['client_operation_id']==meta.get('client_operation_id'),'CANCEL_OPERATION_MISMATCH')
   check((folder/'operation').is_dir(),'UNKNOWN_OPERATION');(folder/'operation/cancel').touch();(folder/'operation/CANCEL').touch()
  return dict(status='cancel_requested',session_id=folder.name)
 if action=='preflight':
  check(mode in ('paid','paid-contents'),'PAID_PREFLIGHT_ONLY')
  if mode=='paid':
   from scripts.ocr.paid_mvp.pipeline import preflight
   task=read(folder/'operation/task.json');manifest=preflight(task)
  else:
   from scripts.bookmarks.paid_mvp.engine import preflight
   check(meta.get('table'),'FAILED_RESULT_HAS_NO_SENDABLE_DRAFT')
   table=read(folder/meta['table']);task=dict(operation_id=table['operation_id'],input_pdf=table['source'],input_sha256=table['source_sha256'],page_numbers=sorted(x['page_number'] for x in table['images']),mode='paid-contents',output_directory=str(folder),config_version='paid-contents-v1',images=table['images']);manifest=preflight(task)
  return dict(manifest=manifest,network_sent=False,send_enabled=False,stop_reason='No live App send: prior requests consumed; Mistral authorized retry returned 429/code1300. Future send requires exact payload/endpoint/model/budget approval and durable ledger. Existing $1 total cap and combined .90 allocation are not reset.',budget=dict(total_cap_usd=1,conservative_allocation_usd=.90,c_usage_estimate_usd=.0078547,d_usage_estimate_usd=.0123386,mistral_unknown_reference_hold_usd=.008,not_invoice=True))
 if action=='reload':return view(folder,meta,config)
 if action in ('review','save') and mode=='paid-contents':check(not meta.get('diagnostic_only',True),'DIRECTORY_DIAGNOSTIC_ONLY: persisted guard required')
 if action=='review':
  if mode=='local':check(view(folder,meta,config).get('draft_published') is not False,'DRAFT_NOT_PUBLISHED: incomplete results are read-only')
  expected=request['expected_revision'];actions=request['actions'];check(isinstance(actions,list) and 0<len(actions)<=100,'BOUNDED_REVIEW_REQUIRED')
  if mode in ('local','critical-edition'):
   from scripts.ocr.mvp.store import review_save
   review_save(folder/'operation',dict(expected_revision=expected,input_sha256=request['input_sha256'],actions=actions))
  elif mode=='paid':
   from scripts.ocr.paid_mvp.pipeline import review,export_pdf
   receipt=review(folder/'operation',expected,request['input_sha256'],actions);export_pdf(folder/'operation',receipt['revision'])
  else:
   from scripts.bookmarks.paid_mvp.engine import review
   table=read(folder/meta['table']);result=review(folder/meta['table'],dict(expected_revision=expected,source_sha256=request['input_sha256'],raw_model_sha256=table['raw_model_sha256'],operations=actions));meta['table']=Path(result['draft']).name;write(folder/'desktop-session.json',meta)
  return view(folder,meta,config)
 if action=='recover-save':
  check(mode=='local','LOCAL_RECOVERY_ONLY')
  current=view(folder,meta,config);check(current.get('draft_published') is not False,'DRAFT_NOT_PUBLISHED: incomplete results are read-only');check(current['revision']==request['expected_revision'],'STALE_REVISION')
  from .save_recovery import recover
  from .sessions import mutation_guard
  def validate_recovery():
   source_bound(meta['source'],meta['source_sha256'],request);mutation_guard(meta,config)
   check(view(folder,meta,config)['revision']==current['revision'],'STALE_REVISION')
  recovered=recover(folder,meta,current,request,validate_recovery)
  return dict(**view(folder,meta,config),saved=recovered,save_recovered=True)
 if action=='save':
  current=view(folder,meta,config)
  if mode=='local':check(current.get('draft_published') is not False,'DRAFT_NOT_PUBLISHED: incomplete results are read-only')
  check(current['revision']==request['expected_revision'],'STALE_REVISION')
  selected_path=Path(request['output_path']);check(not selected_path.is_symlink(),'NEW_PDF_REQUIRED')
  dest=selected_path.resolve();check(dest.suffix.lower()=='.pdf' and not dest.exists() and dest!=Path(meta['source']).resolve(),'NEW_PDF_REQUIRED')
  receipt_path=folder/f'save-{uuid.uuid4().hex}.json'
  if mode=='paid-contents':
   from scripts.bookmarks.paid_mvp.engine import save
   result=save(folder/meta['table'],dict(expected_revision=request['expected_revision'],source_sha256=request['input_sha256'],output_pdf=str(dest),selected_ids=request['selected_ids'],explicit_partial_export_confirmed=request.get('partial_confirmed') is True))
   write(folder/'import-completion.json',result)
  else:
   check(current.get('output_pdf'),'NO_DERIVED_PDF');src=Path(current['output_pdf']);check(src.is_file(),'OUTPUT_MISSING')
   derived_sha256=sha(src)
   cancel_paths=[folder/'operation'/name for name in ('CANCEL','cancel')]
   cancellation_state={p:sha(p) if p.is_file() else None for p in cancel_paths}
   import fitz
   with fitz.open(meta['source']) as original,fitz.open(src) as derived:
    check(len(original)==len(derived) and all(a.rect==b.rect for a,b in zip(original,derived)),'HISTORICAL_OUTPUT_SCOPE_REQUIRES_NEW_REVIEW_REVISION')
   def validate_save():
    if mode=='local':
     from .sessions import mutation_guard
     mutation_guard(meta,config)
    check(sha(meta['source'])==meta['source_sha256'],'SOURCE_CHANGED')
    check(all((sha(p) if p.is_file() else None)==h for p,h in cancellation_state.items()),'SAVE_CANCELLED')
    latest=view(folder,meta,config)
    check(latest['revision']==current['revision'] and latest.get('output_pdf')==str(src),'STALE_REVISION')
   recovery_binding=dict(session_id=folder.name,source_pdf=meta['source'],source_sha256=meta['source_sha256'],revision=current['revision'],derived_pdf=str(src),derived_sha256=derived_sha256,runtime_binding=meta.get('runtime_binding'),cancellation_state={p.name:h for p,h in cancellation_state.items()})
   result=atomic_save_copy(src,dest,validate=validate_save,receipt_path=receipt_path,
                          expected_sha256=derived_sha256,
                          result=dict(source_pdf=meta['source'],source_sha256=meta['source_sha256'],review_required=True,export_review=current.get('export_review',[]),revision=current['revision'],recovery_binding=recovery_binding))
  if mode=='paid-contents':
   check(sha(meta['source'])==meta['source_sha256'],'SOURCE_CHANGED');write(receipt_path,result)
  return dict(**view(folder,meta,config),saved=result)
 if action=='search':
  import fitz
  current=view(folder,meta,config);query=request['query'];check(isinstance(query,str) and 0<len(query)<=200,'BOUNDED_QUERY_REQUIRED')
  pdf=request.get('saved_pdf') or current.get('output_pdf');check(pdf and Path(pdf).is_file(),'PDF_MISSING')
  # Restrict search to the derived result or a session-recorded saved output.
  permitted={current.get('output_pdf')}
  import re
  for receipt in folder.glob('save-*.json'):
   if not re.fullmatch(r'save-[0-9a-f]{32}\.json',receipt.name) or receipt.is_symlink():continue
   saved=read(receipt);saved_path=saved.get('output_pdf') or saved.get('artifacts',{}).get('bookmarks_pdf')
   if saved_path==pdf:
    if mode=='local':check(saved.get('source_sha256')==meta['source_sha256'] and saved.get('sha256')==sha(pdf),'SAVED_OUTPUT_CHANGED_OR_UNBOUND')
    permitted.add(saved_path)
  check(pdf in permitted,'SEARCH_OUTPUT_SCOPE')
  hits=[]
  with fitz.open(pdf) as doc:
   for i,page in enumerate(doc):
    for rect in page.search_for(query):hits.append(dict(page=i+1,bbox=list(rect),consumer='PDF exact Unicode'))
    normalized=unicodedata.normalize('NFC',page.get_text());needle=unicodedata.normalize('NFC',query)
    if needle in normalized:hits.append(dict(page=i+1,consumer='App NFC text',bbox=None))
  return dict(hits=hits,pdf=pdf,query=query,normalization='NFC only; exact PDF query reported separately')

if __name__=='__main__':
 try:
  config=packaged_config(os.environ['MUSEION_LOCAL_RESOURCE_ROOT'],os.environ['MUSEION_LOCAL_SESSION_ROOT'],os.environ.get('MUSEION_LOCAL_LEGACY_SESSION_ROOT')) if os.environ.get('MUSEION_LOCAL_RESOURCE_ROOT') else read(os.environ['MUSEION_MVP_RUNTIME_CONFIG'])
  if config.get('tessdata'):os.environ['TESSDATA_PREFIX']=config['tessdata']
  before=None
  if config.get('package_root') and config.get('local_only'):
   from .runtime_contract import loaded_proof
   # These are actual imports, without recognition or helper execution.
   from scripts.ocr.mvp import core,local,store
   from scripts.ocr.free_local import pipeline
   from . import sessions,save_recovery
   import fitz,numpy,cv2,PIL.Image
   before=loaded_proof(config)
  response=main(json.load(sys.stdin),config)
  if before is not None:
   response['loaded_runtime_proof']=dict(before=before,after=loaded_proof(config))
  response['runtime_proof']=dict(python=sys.executable,torch_loaded='torch' in sys.modules,kraken_loaded='kraken' in sys.modules,external_development_runtime=config.get('external_development_runtime',True),resource_root=config.get('package_root'),sys_prefix=sys.prefix)
  print(json.dumps(response,ensure_ascii=False))
 except Exception as error:
  print(json.dumps(dict(status='failed',error_code=type(error).__name__,message=str(error),network_requests=0),ensure_ascii=False));sys.exit(1)
