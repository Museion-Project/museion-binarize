/** The local compiler's existing JSON contract; physical PDF targets are zero based. */
export interface LocalEntry {
  id: string; title: string; parent: string | null; level: number;
  target_pdf_page: number | null; printed_page: string | null;
  source_page: number; source_bbox: {x:number;y:number;width:number;height:number};
  evidence_ids: string[]; review_reasons: string[]; section_label: string | null;
  [key: string]: unknown;
}
export interface ContentsResult {
  documentId: string; sessionId: string;
  table: { entries: LocalEntry[]; source_entries: LocalEntry[]; navigation_projection: {source_id:string;requires_review:boolean}[]; page_count:number; [key:string]:unknown };
  intake: {routes:{page:number;path:"native_text"|"apple_vision_fast"}[];prepare_seconds:number;selected_pages:number[];mode:string};
}
export interface LocalReadiness {available:boolean;imageRecognitionAvailable:boolean;message:string|null}
export interface LocalToolsProgress {documentId:string;operationId:string;stage:string;pageNumber:number|null;pageCount:number|null}
export interface LocalSaveResult {outputPath:string;pages:number;bookmarksWritten:number;binarized:boolean;pagesProcessed?:number;pagesPreserved?:number;outputBytes:number;elapsedSeconds:number}
export type TreeProjection="navigation"|"source";
export interface ContentsDraft {
  result:ContentsResult; entries:LocalEntry[]; projection:TreeProjection; selectedId:string|null;
  reviewed:boolean; dirty:boolean; history:{entries:LocalEntry[];projection:TreeProjection}[];
}
export function parseContentsPages(value:string,pageCount:number):{pages:number[];error:string|null}{
  if(!value.trim())return {pages:[],error:"请填写目录所在的 PDF 页码。"};
  const pages=new Set<number>();
  for(const part of value.trim().split(/[\s,，、]+/)){
    const match=/^(\d+)(?:[-–—](\d+))?$/.exec(part);
    if(!match)return {pages:[],error:"页码格式如 6-7, 13，请使用 PDF 页码。"};
    const first=Number(match[1]),last=Number(match[2]??match[1]);
    if(first<1||last<first||last>pageCount)return {pages:[],error:`目录页需在 1–${pageCount} 之间。`};
    if(last-first>=40)return {pages:[],error:"一次最多选择40页目录。"};
    for(let page=first;page<=last;page++)pages.add(page);
    if(pages.size>40)return {pages:[],error:"一次最多选择40页目录。"};
  }
  return {pages:[...pages].sort((a,b)=>a-b),error:null};
}
export function orderedTree(entries:LocalEntry[]):LocalEntry[]{
  const ids=new Set(entries.map(e=>e.id));
  if(ids.size!==entries.length||entries.some(e=>e.parent!==null&&!ids.has(e.parent)))throw new Error("目录含重复条目或不存在的父节点。");
  const result:LocalEntry[]=[],seen=new Set<string>();
  function walk(parent:string|null,level:number){
    for(const e of entries.filter(e=>e.parent===parent)){
      if(seen.has(e.id))throw new Error("目录父节点形成循环。");
      seen.add(e.id);result.push({...e,level});walk(e.id,level+1);
    }
  }
  walk(null,0);if(result.length!==entries.length)throw new Error("目录父节点形成循环。");
  return result;
}
export function descendants(entries:LocalEntry[],id:string):Set<string>{
  const ids=new Set([id]);let count=0;
  while(count!==ids.size){count=ids.size;for(const e of entries)if(e.parent&&ids.has(e.parent))ids.add(e.id);}
  return ids;
}
export function changeProjection(draft:ContentsDraft,projection:TreeProjection):LocalEntry[]{
  const original=draft.result.table,omitted=new Set(original.navigation_projection.map(p=>p.source_id));
  if(projection==="navigation"){
    const current=new Map(draft.entries.map(e=>[e.id,e]));
    return orderedTree(draft.entries.filter(e=>!omitted.has(e.id)).map(e=>{
      let parent=e.parent;while(parent&&omitted.has(parent))parent=current.get(parent)?.parent??null;
      return {...e,parent};
    }));
  }
  const current=new Map(draft.entries.map(e=>[e.id,e])),nav=new Map(original.entries.map(e=>[e.id,e]));
  const restored=original.source_entries.filter(e=>current.has(e.id)||omitted.has(e.id)).map(source=>{
    const edited=current.get(source.id);if(!edited)return {...source};
    // Preserve manual titles/targets and manually changed hierarchy. Only lift
    // back the default projected parent when the user chooses source containers.
    return {...edited,parent:edited.parent===nav.get(source.id)?.parent?source.parent:edited.parent};
  });
  const restoredIds=new Set(restored.map(e=>e.id));
  return orderedTree([...restored,...draft.entries.filter(e=>!restoredIds.has(e.id))]);
}
export function entryProblems(entries:LocalEntry[],pageCount:number){
  return {
    missingTargets:entries.filter(e=>e.target_pdf_page===null||!Number.isInteger(e.target_pdf_page)||e.target_pdf_page<0||e.target_pdf_page>=pageCount).length,
    emptyTitles:entries.filter(e=>!e.title.trim()).length,
  };
}
export const STAGE_LABELS:Record<string,string>={
  starting:"准备处理…",rendering_contents:"正在读取目录页面",reading_contents:"正在读取目录页…",preparing_local_recognizer:"正在准备本地图像识别…",
  recognizing_contents:"正在识别目录…",reading_page_numbers:"正在读取目录页码…",mapping_pages:"正在匹配 PDF 目标页…",
  building_tree:"正在整理目录层级…",checking_bookmarks:"正在检查目录…",binarizing:"正在进行黑白处理…",
  writing_bookmarks:"正在写入目录书签…",validating:"正在核对保存结果…",
};

/** Physical PDF pages to process; skipped/unselected pages remain in the PDF. */
export function parseBinarizePages(value:string,pageCount:number,skipped:number[]=[]):{pages:number[];error:string|null}{
  const selected=new Set<number>(),skip=new Set(skipped);
  const terms=value.trim().toLowerCase()==="all"?[`1-${pageCount}`]:value.trim().split(/[,，、]+/);
  if(!value.trim()||terms.length>10000)return {pages:[],error:"请填写处理范围，如 1-20, 35。"};
  for(const term of terms){
    const match=/^\s*(\d+)\s*(?:[-–—]\s*(\d+))?\s*$/.exec(term);
    if(!match)return {pages:[],error:"处理范围格式如 1-20, 35，请使用 PDF 页码。"};
    const first=Number(match[1]),last=Number(match[2]??match[1]);
    if(!Number.isSafeInteger(last)||first<1||last<first||last>pageCount)return {pages:[],error:`处理页需在 1–${pageCount} 之间。`};
    for(let p=first;p<=last;p++)if(!skip.has(p))selected.add(p);
  }
  return {pages:[...selected].sort((a,b)=>a-b),error:selected.size?null:"没有待处理页，请调整范围或恢复跳过的页面。"};
}

export interface DocumentAnalysisStatus {
  documentId:string;textLayer:"checking"|"present"|"absent"|"mixed"|"unavailable";
  paginationStatus:"running"|"ready"|"partial"|"unavailable"|"failed";
  sequenceCount:number;sampledPages:number;elapsedSeconds?:number;message:string|null;
}
export interface PreparedBinarizationResult {documentId:string;preparedId:string;pagesProcessed:number;elapsedSeconds:number}
export function documentAnalysisLabel(status:DocumentAnalysisStatus|null){
  const layer={checking:"检测中",present:"有",absent:"无",mixed:"部分页面有",unavailable:"暂不可用"};
  const pagination={running:"重建中…",ready:`已重建${status?.sequenceCount?` · ${status.sequenceCount} 套`:""}`,partial:"已部分重建",unavailable:"未识别到",failed:"暂不可用"};
  return `文字层：${layer[status?.textLayer??"checking"]}\u3000·\u3000排版页码：${pagination[status?.paginationStatus??"running"]}`;
}
