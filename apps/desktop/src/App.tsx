import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import "./App.css";
import {initialState,reducer} from "./app/reducer";
import type {LocalEntry,ContentsDraft,LocalReadiness,LocalSaveResult,TreeProjection,PreparedBinarizationResult} from "./app/localTools";
import {documentAnalysisLabel,parseBinarizePages,parseContentsPages,orderedTree,changeProjection,entryProblems,STAGE_LABELS} from "./app/localTools";
import {ContentsEditor} from "./components/ContentsEditor";
import {ErrorBoundary} from "./components/ErrorBoundary";
import {ErrorPanel} from "./components/ErrorPanel";
import {PasswordPrompt} from "./components/PasswordPrompt";
import {PreviewPane} from "./components/PreviewPane";
import {PageSidebar} from "./components/PageSidebar";
import {SettingsPanel} from "./components/SettingsPanel";
import {useDocumentAnalysis} from "./hooks/useDocumentAnalysis";
import {usePreview} from "./hooks/usePreview";
import {formatBytes} from "./lib/formatting";
import {prepareLocalBinarization,BackendError,openDocument,pickPdfToOpen,pickOutputDestination,onFileDragDrop,localBookmarkReadiness,generateLocalContents,saveLocalPdf,cancelLocalTools,onLocalToolsProgress} from "./lib/tauri";

type Task={kind:"generate"|"binarize"|"save";id:string;stage:string;cancelling:boolean;page:number|null;count:number|null;started:number};
function message(error:unknown){return error instanceof BackendError?error.error.message:error instanceof Error?error.message:String(error);}

function App(){
  const [state,dispatch]=useReducer(reducer,initialState);
  const ready=state.kind==="ready"?state:null;
  const [binarize,setBinarize]=useState(true),[bookmarks,setBookmarks]=useState(false);
  const [rangeMode,setRangeMode]=useState<"all"|"custom">("all"),[binarizeRange,setBinarizeRange]=useState("");
  const [skippedPages,setSkippedPages]=useState<number[]>([]);
  const [contentsPages,setContentsPages]=useState(""),[mode,setMode]=useState<"auto"|"image">("auto");
  const [readiness,setReadiness]=useState<LocalReadiness|null>(null);
  const [draft,setDraft]=useState<ContentsDraft|null>(null);
  const [task,setTask]=useState<Task|null>(null),taskRef=useRef<Task|null>(null),opening=useRef(false);
  const [error,setError]=useState<string|null>(null),[notice,setNotice]=useState<string|null>(null);
  const [result,setResult]=useState<LocalSaveResult|null>(null);
  const [prepared,setPrepared]=useState<(PreparedBinarizationResult&{signature:string})|null>(null);
  const analysis=useDocumentAnalysis(ready?.document.documentId??null);
  const [highlight,setHighlight]=useState<LocalEntry|null>(null);
  const [dragActive,setDragActive]=useState(false),[elapsed,setElapsed]=useState(0);
  const [confirm,setConfirm]=useState<{message:string;action:()=>void}|null>(null);
  const currentDocument=useRef<string|null>(null);
  currentDocument.current=ready?.document.documentId??null;
  const busy=!!task||state.kind==="opening";

  const range=useMemo(()=>parseBinarizePages(rangeMode==="all"?"all":binarizeRange,ready?.document.pageCount??0,skippedPages),[rangeMode,binarizeRange,ready?.document.pageCount,skippedPages]);
  const processingSignature=JSON.stringify([ready?.document.documentId,ready?.settings,range.pages]);
  const preparedMatches=!!prepared&&prepared.signature===processingSignature;
  const currentIncluded=range.pages.includes(ready?.currentPage??0);
  useEffect(()=>{
    setRangeMode("all");setBinarizeRange("");setSkippedPages([]);
  },[ready?.document.documentId]);
  useEffect(()=>{
    if(!currentIncluded&&ready?.viewMode==="processed")dispatch({type:"SET_VIEW_MODE",mode:"original"});
  },[currentIncluded,ready?.viewMode]);
  function toggleSkip(){
    if(!ready)return;
    setSkippedPages(p=>p.includes(ready.currentPage)?p.filter(n=>n!==ready.currentPage):[...p,ready.currentPage]);setResult(null);
  }

  usePreview(ready&&!task?ready.document:null,ready?.currentPage??1,ready?.settings??null,dispatch,binarize&&currentIncluded,ready?.viewMode??"original");

  useEffect(()=>{
    if(!task)return;
    const timer=window.setInterval(()=>setElapsed(Math.floor((Date.now()-task.started)/1000)),500);
    return ()=>window.clearInterval(timer);
  },[task?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(()=>{
    let disposed=false,stop:(()=>void)|undefined;
    onLocalToolsProgress(event=>{
      if(currentDocument.current!==event.documentId||taskRef.current?.id!==event.operationId)return;
      setTask(t=>t&&t.id===event.operationId?{...t,stage:event.stage,page:event.pageNumber,count:event.pageCount}:t);
    }).then(unlisten=>{if(disposed)unlisten();else stop=unlisten;}).catch(()=>{});
    return ()=>{disposed=true;stop?.();};
  },[]);
  useEffect(()=>{
    let disposed=false;
    if(!bookmarks||!ready)return;
    setReadiness(null);
    localBookmarkReadiness().then(value=>{if(!disposed)setReadiness(value);}).catch(e=>{if(!disposed)setReadiness({available:false,imageRecognitionAvailable:false,message:message(e)});});
    return ()=>{disposed=true;};
  },[bookmarks,ready?.document.documentId]); // eslint-disable-line react-hooks/exhaustive-deps

  const doOpenPath=useCallback(async(path:string)=>{
    if(taskRef.current||opening.current)return;
    opening.current=true;setError(null);setNotice(null);setResult(null);setHighlight(null);
    dispatch({type:"OPEN_STARTED"});
    try{
      const document=await openDocument(path);
      setDraft(null);setPrepared(null);setContentsPages("");setMode("auto");
      dispatch({type:"OPEN_SUCCEEDED",document});
    }catch(e){
      if(e instanceof BackendError&&e.error.code==="password_required")dispatch({type:"PASSWORD_REQUIRED",path});
      else dispatch({type:"OPEN_FAILED",error:e instanceof BackendError?e.error:{code:"internal_error",message:message(e),hint:null,detail:null}});
    }finally{opening.current=false;}
  },[]);
  const requestOpen=useCallback((path:string)=>{
    if(taskRef.current||opening.current)return;
    if(draft?.dirty)setConfirm({message:"当前目录有未保存的内容。打开其他 PDF 将丢弃这些修改。",action:()=>{void doOpenPath(path);}});
    else void doOpenPath(path);
  },[draft?.dirty,doOpenPath]);
  const handleOpen=useCallback(async()=>{
    if(taskRef.current||opening.current)return;
    try{const path=await pickPdfToOpen();if(path)requestOpen(path);}catch(e){setError(message(e));}
  },[requestOpen]);
  useEffect(()=>{
    let disposed=false,stop:(()=>void)|undefined;
    onFileDragDrop(event=>{
      if(event.type==="leave"){setDragActive(false);return;}
      if(taskRef.current||opening.current)return;
      if(event.type==="enter"){setDragActive(true);return;}
      if(event.type!=="drop")return;
      setDragActive(false);
      if(event.paths.length!==1||!/\.pdf$/i.test(event.paths[0])){setError("请一次打开一个 PDF 文件。");return;}
      requestOpen(event.paths[0]);
    }).then(unlisten=>{if(disposed)unlisten();else stop=unlisten;}).catch(()=>{});
    return ()=>{disposed=true;stop?.();};
  },[requestOpen]);
  function begin(kind:Task["kind"]){
    if(taskRef.current)return null;
    const task:Task={kind,id:crypto.randomUUID(),stage:"starting",cancelling:false,page:null,count:null,started:Date.now()};
    taskRef.current=task;setTask(task);setElapsed(0);setError(null);setNotice(null);setResult(null);return task;
  }
  function end(id:string){if(taskRef.current?.id===id){taskRef.current=null;setTask(null);}}
  async function startBinarize(){
    if(!ready||range.error)return;
    const current=begin("binarize");if(!current)return;
    const documentId=ready.document.documentId,signature=processingSignature;
    setPrepared(null);
    try{
      const result=await prepareLocalBinarization({documentId,operationId:current.id,settings:ready.settings,binarizePages:range.pages});
      if(currentDocument.current!==documentId)return;
      setPrepared({...result,signature});setNotice(`已完成 ${result.pagesProcessed} 页黑白处理，可保存或继续生成目录。`);
      if(currentIncluded)dispatch({type:"SET_VIEW_MODE",mode:"processed"});
    }catch(e){if(e instanceof BackendError&&e.error.code==="cancelled")setNotice("已取消黑白处理，未保存文件。");else setError(message(e));}
    finally{end(current.id);}
  }
  async function generate(){
    if(!ready)return;
    const parsed=parseContentsPages(contentsPages,ready.document.pageCount);
    if(parsed.error){setError(parsed.error);return;}
    const current=begin("generate");if(!current)return;
    const documentId=ready.document.documentId;
    try{
      const response=await generateLocalContents({documentId,operationId:current.id,pages:parsed.pages,mode});
      if(currentDocument.current!==documentId)return;
      const entries=orderedTree(response.table.entries);
      setDraft({result:response,entries,projection:"navigation",selectedId:entries[0]?.id??null,reviewed:false,dirty:true,history:[]});
      setHighlight(null);dispatch({type:"SELECT_PAGE",page:parsed.pages[0]});dispatch({type:"SET_VIEW_MODE",mode:"original"});
      setNotice(entries.length?`已生成 ${entries.length} 个目录条目，请核对后保存。`:"没有识别到目录条目。请检查目录页，或手动添加书签。");
    }catch(e){if(e instanceof BackendError&&e.error.code==="cancelled")setNotice("已取消生成，之前的目录仍保留。");else setError(message(e));}
    finally{end(current.id);}
  }
  function requestGenerate(){
    if(draft?.dirty)setConfirm({message:"重新生成目录将替换当前目录和未保存的修改。",action:()=>{void generate();}});
    else void generate();
  }
  function changeEntries(entries:LocalEntry[],projection?:TreeProjection){
    try{const ordered=orderedTree(entries);setDraft(d=>d?{...d,entries:ordered,projection:projection??d.projection,reviewed:false,dirty:true,history:[{entries:d.entries,projection:d.projection},...d.history].slice(0,30),selectedId:ordered.some(e=>e.id===d.selectedId)?d.selectedId:ordered[0]?.id??null}:d);setResult(null);}catch(e){setError(message(e));}
  }
  function selectPage(page:number){if(ready&&!task&&Number.isInteger(page)&&page>=1&&page<=ready.document.pageCount){setHighlight(null);dispatch({type:"SELECT_PAGE",page});}}
  function showSource(entry:LocalEntry){
    if(!ready||task)return;dispatch({type:"SELECT_PAGE",page:entry.source_page+1});dispatch({type:"SET_VIEW_MODE",mode:"original"});setHighlight(entry);
  }
  async function save(){
    if(!ready||taskRef.current)return;
    if(binarize&&!preparedMatches){setError("请先点击黑白处理的开始。");return;}
    if(binarize&&range.error){setError(range.error);return;}
    if(!binarize&&!bookmarks){setError("请至少选择一项处理内容。");return;}
    if(bookmarks&&draft&&(mode!==draft.result.intake.mode||parseContentsPages(contentsPages,ready.document.pageCount).pages.join(",")!==draft.result.intake.selected_pages.join(","))){setError("目录页或读取方式已改变，请重新生成目录。");return;}
    if(bookmarks&&(!draft||!draft.reviewed)){setError("请先生成并核对目录，再确认保存。");return;}
    if(bookmarks&&draft){const p=entryProblems(draft.entries,ready.document.pageCount);if(p.missingTargets||p.emptyTitles){setError("请补齐目录标题和 PDF 目标页。");return;}}
    const suffix=[binarize?"binarized":null,bookmarks?"bookmarks":null].filter(Boolean).join("-");
    let output:string|null;
    try{output=await pickOutputDestination(`${ready.document.fileName.replace(/\.pdf$/i,"")}-${suffix}.pdf`);}catch(e){setError(message(e));return;}
    if(!output)return;
    const current=begin("save");if(!current)return;
    try{
      const completed=await saveLocalPdf({documentId:ready.document.documentId,operationId:current.id,outputPath:output,binarize,binarizePages:binarize?range.pages:undefined,preparedId:binarize&&preparedMatches?prepared?.preparedId:undefined,settings:ready.settings,bookmarkSessionId:bookmarks&&draft?draft.result.sessionId:null,entries:bookmarks&&draft?draft.entries.map(({id,title,parent,target_pdf_page})=>({id,title,parent,target_pdf_page})):[],projection:draft?.projection??"navigation",reviewAccepted:bookmarks&&!!draft?.reviewed});
      setResult(completed);setNotice(null);if(bookmarks)setDraft(d=>d?{...d,dirty:false}:d);
    }catch(e){if(e instanceof BackendError&&e.error.code==="cancelled")setNotice("已取消，未保存文件。目录修改仍保留。");else setError(message(e));}
    finally{end(current.id);}
  }
  function cancel(){const current=taskRef.current;if(!current)return;setTask(t=>t?{...t,cancelling:true}:t);void cancelLocalTools(current.id).catch(e=>setError(message(e)));}

  useEffect(()=>{
    function onKey(event:KeyboardEvent){
      const target=event.target as HTMLElement;
      if(target?.matches("input, textarea, select")||target?.isContentEditable)return;
      if((event.metaKey||event.ctrlKey)&&event.key.toLowerCase()==="o"){event.preventDefault();void handleOpen();}
      if((event.metaKey||event.ctrlKey)&&event.key==="Enter"){event.preventDefault();void save();}
      if(ready&&!task&&event.key==="ArrowRight"){event.preventDefault();selectPage(ready.currentPage+1);}
      if(ready&&!task&&event.key==="ArrowLeft"){event.preventDefault();selectPage(ready.currentPage-1);}
    }
    window.addEventListener("keydown",onKey);return ()=>window.removeEventListener("keydown",onKey);
  });
  const page=ready?.document.pages[(ready?.currentPage??1)-1];
  const chosenPages=ready?parseContentsPages(contentsPages,ready.document.pageCount):null;
  const contentsStale=!!draft&&(mode!==draft.result.intake.mode||!!chosenPages?.error||chosenPages?.pages.join(",")!==draft.result.intake.selected_pages.join(","));
  const canSave=!!ready&&!busy&&(binarize||bookmarks)&&(!binarize||!range.error&&preparedMatches)&&(!bookmarks||!!draft?.reviewed&&!contentsStale);
  return <ErrorBoundary><main className="app-shell">
    <header className="app-header"><div className="brand"><span className="brand-mark">M</span><div>Museion <strong>Binarize</strong></div></div><span className="header-divider"/><div className="document-name">{ready?<><strong title={ready.document.fileName}>{ready.document.fileName}</strong><span>{ready.document.pageCount} 页 · {formatBytes(ready.document.sourceBytes)}</span></>:<span>本地 PDF 工具</span>}</div><button onClick={()=>void handleOpen()} disabled={busy} title="⌘O / Ctrl+O">打开 PDF</button></header>
    <div className="workflow-line" aria-label="使用步骤"><span className={ready?"complete":"current"}>1 打开文档</span><i>›</i><span className={ready&&!result?"current":""}>2 开始处理 · 预览与修改</span><i>›</i><span className={result?"complete":""}>3 保存新 PDF</span></div>
    {error&&<div className="inline-error" role="alert"><span>{error}</span><button aria-label="关闭错误提示" onClick={()=>setError(null)}>×</button></div>}
    <div className={`workspace ${bookmarks&&ready?"has-contents":""}`}>
      <aside className="tools-panel" aria-label="处理工具"><div className="tool-options"><h2>处理内容</h2><p className="panel-intro">可以单独使用，也可以一起保存。</p>
        <section className={`tool-section ${binarize?"enabled":""}`}><div className="tool-header"><label className="tool-heading"><input type="checkbox" checked={binarize} disabled={!ready||busy} onChange={e=>{setBinarize(e.target.checked);setResult(null);if(!e.target.checked)dispatch({type:"SET_VIEW_MODE",mode:"original"});}}/><span><strong>黑白处理</strong><small>清理底色，输出黑白 PDF</small></span></label><button className="tool-action" disabled={!ready||busy||!binarize||!!range.error} aria-busy={task?.kind==="binarize"} onClick={()=>void startBinarize()}>开始</button></div>{binarize&&ready&&<><div className="settings-panel binarize-range"><label htmlFor="binarize-scope">处理范围</label><select id="binarize-scope" value={rangeMode} disabled={busy} onChange={e=>{setRangeMode(e.target.value as "all"|"custom");setResult(null);}}><option value="all">全部页面</option><option value="custom">指定页码</option></select><button disabled={busy} onClick={()=>{setRangeMode("custom");setBinarizeRange(String(ready.currentPage));setSkippedPages(p=>p.filter(n=>n!==ready.currentPage));setResult(null);}}>仅本页</button>{rangeMode==="custom"&&<input aria-label="黑白处理页码" placeholder="如 1-20, 35" value={binarizeRange} disabled={busy} onChange={e=>{setBinarizeRange(e.target.value);setResult(null);}}/>}<small role={range.error?"alert":undefined}>{range.error??`处理 ${range.pages.length} 页，其余 ${ready.document.pageCount-range.pages.length} 页保留原样。`}</small>{skippedPages.length>0&&<div><small>已跳过：{[...skippedPages].sort((a,b)=>a-b).join(", ")}</small><button disabled={busy} onClick={()=>{setSkippedPages([]);setResult(null);}}>恢复全部跳过页</button></div>}</div>{prepared&&<p className="field-hint prepared-status">{preparedMatches?`已处理 ${prepared.pagesProcessed} 页 · ${prepared.elapsedSeconds.toFixed(1)} 秒`:"范围或参数已修改，请再次开始。"}</p>}<SettingsPanel settings={ready.settings} preset={ready.preset} disabled={busy} onChange={(settings,preset)=>{dispatch({type:"SET_SETTINGS",settings,preset});setResult(null);}}/></>}</section>
        <section className={`tool-section ${bookmarks?"enabled":""}`}><div className="tool-header"><label className="tool-heading"><input type="checkbox" checked={bookmarks} disabled={!ready||busy} onChange={e=>{setBookmarks(e.target.checked);setResult(null);}}/><span><strong>目录书签</strong><small>从目录页生成可点击的书签</small></span></label><button className="tool-action" disabled={!ready||!bookmarks||busy||!readiness?.available} aria-busy={task?.kind==="generate"} onClick={requestGenerate}>生成</button></div>
          {bookmarks&&ready&&<div className="contents-controls"><label htmlFor="contents-pages">目录所在的 PDF 页码</label><input id="contents-pages" value={contentsPages} disabled={busy} placeholder="例如 6-7, 13" onChange={e=>{setContentsPages(e.target.value);setError(null);setResult(null);}}/><p className="field-hint">使用预览上方的页码，不是书内印刷页码。</p><button className="text-button" disabled={busy} onClick={()=>{const parsed=contentsPages.trim()?parseContentsPages(contentsPages,ready.document.pageCount):{pages:[],error:null};if(parsed.error){setError(parsed.error);return;}setContentsPages([...new Set([...parsed.pages,ready.currentPage])].sort((a,b)=>a-b).join(", "));}}>＋ 加入当前页（{ready.currentPage}）</button>
          <details><summary>读取方式</summary><label className="sr-only" htmlFor="contents-mode">目录读取方式</label><select id="contents-mode" value={mode} disabled={busy} onChange={e=>{setMode(e.target.value as "auto"|"image");setResult(null);}}><option value="auto">自动：读取文字或识别图像</option><option value="image" disabled={readiness?.imageRecognitionAvailable===false}>从页面图像识别</option></select><p className="field-hint">已有文字不完整时，可尝试从图像重新识别。</p></details>
          {contentsStale&&<p className="attention">目录页或读取方式已改变，请重新生成。</p>}
          {!readiness?<p className="field-hint">正在检查本地目录组件…</p>:!readiness.available?<p className="attention" role="alert">{readiness.message}</p>:draft&&<p className="field-hint">本次：{draft.result.intake.routes.filter(r=>r.path==="native_text").length} 页读取文字，{draft.result.intake.routes.filter(r=>r.path==="apple_vision_fast").length} 页图像识别。</p>}
          </div>}
        </section>
        </div><div className="tool-section unavailable"><button className="tool-heading" disabled aria-label="OCR 正文识别，暂未开放"><span className="disabled-tool-icon">T</span><span><strong>OCR 正文识别 <em>暂未开放</em></strong><small>让扫描文档可搜索、可复制</small></span></button></div>
        <p className="local-note">所有当前操作在本机完成。<br/>保存为新文件，原 PDF 保持不变。</p>
      </aside>
      {!ready?<section className="empty-workspace">
        {state.kind==="idle"&&<><div className="empty-document-icon">PDF</div><h1>把 PDF 整理得更好用</h1><p>黑白处理、添加目录书签。<br/>选择工具，开始处理，再保存。</p><button className="primary" onClick={()=>void handleOpen()}>选择 PDF</button><small>也可以把文件拖到这里</small></>}
        {state.kind==="opening"&&<p role="status">正在打开 PDF…</p>}
        {state.kind==="failed"&&<ErrorPanel error={state.error} onDismiss={()=>dispatch({type:"DISMISS_ERROR"})}/>}
      </section>:<section className="preview-area" aria-label="文档预览"><PageSidebar key={ready.document.documentId} document={ready.document} currentPage={ready.currentPage} onSelect={selectPage} paused={busy||ready.preview.loading}/><div className="document-preview"><div className="page-navigation"><div><button aria-label="上一页" disabled={busy||ready.currentPage<=1} onClick={()=>selectPage(ready.currentPage-1)}>‹</button><label htmlFor="current-page">PDF 第</label><input id="current-page" type="number" min={1} max={ready.document.pageCount} value={ready.currentPage} disabled={busy} onChange={e=>selectPage(Number(e.target.value))}/><span>/ {ready.document.pageCount} 页</span><button aria-label="下一页" disabled={busy||ready.currentPage>=ready.document.pageCount} onClick={()=>selectPage(ready.currentPage+1)}>›</button></div>{binarize&&<button className="skip-page" disabled={busy||(!currentIncluded&&!skippedPages.includes(ready.currentPage))} onClick={toggleSkip}>{skippedPages.includes(ready.currentPage)?"恢复本页处理":currentIncluded?"跳过本页":"本页保留原样"}</button>}<span className="preview-caption">{highlight?"目录原页 · 已标出条目位置":"预览"}</span></div><PreviewPane preview={ready.preview} viewMode={ready.viewMode} zoom={ready.zoom} pageNumber={ready.currentPage} onViewModeChange={mode=>dispatch({type:"SET_VIEW_MODE",mode})} onZoomChange={zoom=>dispatch({type:"SET_ZOOM",zoom})} showProcessed={binarize&&currentIncluded} pageWidth={page?.widthPoints??595} pageHeight={page?.heightPoints??842} highlight={highlight?.source_page===ready.currentPage-1&&ready.viewMode==="original"?highlight.source_bbox:null}/></div></section>}
      {bookmarks&&ready&&(draft?<ContentsEditor draft={draft} pageCount={ready.document.pageCount} currentPage={ready.currentPage} disabled={busy||contentsStale} onChange={changeEntries} onProjection={projection=>changeEntries(changeProjection(draft,projection),projection)} onSelect={id=>setDraft(d=>d?{...d,selectedId:id}:d)} onSource={showSource} onTarget={selectPage} onReviewed={reviewed=>setDraft(d=>d?{...d,reviewed}:d)} onUndo={()=>setDraft(d=>d&&d.history.length?{...d,entries:d.history[0].entries,projection:d.history[0].projection,history:d.history.slice(1),reviewed:false,dirty:true}:d)}/>:<aside className="contents-panel contents-empty"><h2>目录书签</h2><div className="panel-empty"><span className="outline-icon">☷</span><h3>先选择目录页</h3><p>在左侧填写目录所在页，<br/>生成后在这里编辑和核对。</p><small>书签中的目标页使用 PDF 页码。</small></div></aside>)}
    </div>
    <footer className="save-bar">
      <div className="save-status" role="status" aria-live="polite">{task?<><strong>{task.cancelling?"正在取消，请等待当前步骤结束…":STAGE_LABELS[task.stage]??"正在处理…"}</strong><span>{task.page&&task.count?`PDF 第 ${task.page} 页 / 共 ${task.count} 页 · `:""}{elapsed} 秒</span></>:result?<><strong className="success">已保存新 PDF</strong><span title={result.outputPath}>{result.pages} 页{result.pagesProcessed!==undefined?` · 黑白处理 ${result.pagesProcessed} 页`:""}{result.bookmarksWritten?` · ${result.bookmarksWritten} 个书签`:""} · {formatBytes(result.outputBytes)} · {result.elapsedSeconds.toFixed(1)} 秒</span></>:<><strong>{ready?[binarize?"黑白处理":null,bookmarks?"目录书签":null].filter(Boolean).join(" ＋ ")||"请选择处理内容":"等待打开文档"}</strong><span>{notice??(bookmarks&&!draft?"生成目录后，可修改并保存。":bookmarks&&!draft?.reviewed?"核对目录后，勾选整份目录确认。":"预览满意后，保存为新文件。")}</span></>}{ready&&<small className="document-analysis-status" title={analysis?.message??(analysis?.sampledPages?`页码基于 ${analysis.sampledPages} 页图像抽样与一致规则，生成后可核对跳转。`:undefined)}>{documentAnalysisLabel(analysis)}</small>}</div>
      {result&&!task&&<><button onClick={()=>requestOpen(result.outputPath)}>打开结果</button><button onClick={()=>void revealItemInDir(result.outputPath).catch(e=>setError(message(e)))}>在 Finder 中显示</button></>}
      {task?<button onClick={cancel} disabled={task.cancelling}>取消</button>:<button className="primary save-button" disabled={!canSave} onClick={()=>void save()} title="⌘Enter / Ctrl+Enter">保存新 PDF…</button>}
    </footer>
    {dragActive&&<div className="file-drop-overlay" role="status">松开以打开 PDF</div>}
    {state.kind==="passwordRequired"&&<PasswordPrompt fileName={state.path.split(/[/\\]/).pop()??state.path} attemptError={state.attemptError} onCancel={()=>dispatch({type:"DISMISS_ERROR"})} onSubmit={async password=>{try{const document=await openDocument(state.path,password);setDraft(null);setPrepared(null);setContentsPages("");dispatch({type:"OPEN_SUCCEEDED",document});}catch(e){dispatch({type:"PASSWORD_RETRY_FAILED",message:message(e)});}}}/>}
    {confirm&&<div className="modal-overlay" role="dialog" aria-modal="true" aria-label="未保存的修改"><div className="confirm-dialog"><h2>未保存的修改</h2><p>{confirm.message}</p><div><button autoFocus onClick={()=>setConfirm(null)}>保留当前内容</button><button onClick={()=>{const action=confirm.action;setConfirm(null);action();}}>丢弃并继续</button></div></div></div>}
  </main></ErrorBoundary>;
}
export default App;
