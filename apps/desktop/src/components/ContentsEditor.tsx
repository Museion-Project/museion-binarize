import { t } from "../lib/i18n";
import type {ContentsDraft,LocalEntry,TreeProjection} from "../app/localTools";
import {descendants,entryProblems} from "../app/localTools";
interface Props {
  draft:ContentsDraft;pageCount:number;currentPage:number;disabled:boolean;
  onChange:(entries:LocalEntry[],projection?:TreeProjection)=>void;
  onProjection:(projection:TreeProjection)=>void;onSelect:(id:string)=>void;
  onSource:(entry:LocalEntry)=>void;onTarget:(page:number)=>void;onReviewed:(value:boolean)=>void;onUndo:()=>void;
}
export function ContentsEditor({draft,pageCount,currentPage,disabled,onChange,onProjection,onSelect,onSource,onTarget,onReviewed,onUndo}:Props){
  const selected=draft.entries.find(e=>e.id===draft.selectedId),problems=entryProblems(draft.entries,pageCount);
  const excluded=selected?descendants(draft.entries,selected.id):new Set<string>();
  function update(fields:Partial<LocalEntry>){if(selected)onChange(draft.entries.map(e=>e.id===selected.id?{...e,...fields}:e));}
  function add(){
    const id=`manual-${crypto.randomUUID()}`;
    onChange([...draft.entries,{id,title:t("新书签"),parent:null,level:0,target_pdf_page:currentPage-1,printed_page:null,source_page:currentPage-1,source_bbox:{x:0,y:0,width:1,height:1},evidence_ids:[],review_reasons:[],section_label:null}]);onSelect(id);
  }
  return <aside className="contents-panel" aria-label={t("目录编辑")}>
    <div className="panel-heading"><div><h2>{t("目录书签")}<span className="count">{draft.entries.length}</span></h2><p>{t("点选条目，查看跳转并修改。")}</p></div><button className="text-button" disabled={disabled||!draft.history.length} onClick={onUndo}>{t("撤销")}</button></div>
    {draft.result.table.navigation_projection.length>0&&<label className="container-choice"><input type="checkbox" checked={draft.projection==="source"} disabled={disabled} onChange={e=>onProjection(e.target.checked?"source":"navigation")}/>{t("保留无页码章题")}<span>{t("保留后需为章题指定目标页。")}</span></label>}
    <div className="contents-tree" role="tree" aria-label={t("可编辑目录树")}>
      {draft.entries.length===0?<div className="panel-empty">{t("暂无条目，可以手动添加。")}</div>:draft.entries.map(entry=><div role="treeitem" aria-level={entry.level+1} aria-selected={entry.id===draft.selectedId} key={entry.id} className={`tree-row ${entry.id===draft.selectedId?"active":""} ${entry.target_pdf_page===null?"unresolved":""}`} style={{paddingLeft:12+Math.min(entry.level,6)*14}}>
        <button className="tree-title" disabled={disabled} title={entry.title} onClick={()=>{onSelect(entry.id);if(entry.target_pdf_page!==null)onTarget(entry.target_pdf_page+1);else onSource(entry);}}>{entry.title||t("未填写标题")}</button>
        <button className="tree-page" disabled={disabled} aria-label={`${entry.title}，${entry.target_pdf_page===null?t("待填写目标页"):t("PDF第{0}页", entry.target_pdf_page+1)}`} onClick={()=>{onSelect(entry.id);if(entry.target_pdf_page!==null)onTarget(entry.target_pdf_page+1);else onSource(entry);}}>{entry.target_pdf_page===null?<span className="pending-dot">{t("待填")}</span>:entry.target_pdf_page+1}</button>
      </div>)}
    </div>
    <div className="tree-actions"><button disabled={disabled} onClick={add}>{t("＋ 添加书签")}</button><button disabled={disabled||!selected} onClick={()=>onChange(draft.entries.filter(e=>!excluded.has(e.id)))}>{excluded.size>1?t("删除所选及{0}个子项", excluded.size-1):t("删除所选")}</button></div>
    {selected&&<fieldset className="entry-fields" disabled={disabled}>
      <legend>{t("编辑所选条目")}</legend>
      <label htmlFor="entry-title">{t("标题")}</label><input id="entry-title" value={selected.title} onChange={e=>update({title:e.target.value})}/>
      <div className="field-pair"><div><label htmlFor="entry-parent">{t("上级条目")}</label><select id="entry-parent" value={selected.parent??""} onChange={e=>update({parent:e.target.value||null})}><option value="">{t("顶层")}</option>{draft.entries.filter(e=>!excluded.has(e.id)).map(e=><option key={e.id} value={e.id}>{e.title}</option>)}</select></div>
      <div><label htmlFor="entry-target">{t("PDF 目标页")}</label><input id="entry-target" type="number" min={1} max={pageCount} value={selected.target_pdf_page===null?"":selected.target_pdf_page+1} placeholder={t("待填写")} onChange={e=>update({target_pdf_page:e.target.value===""?null:Number(e.target.value)-1})}/></div></div>
      <div className="entry-source"><span>{t("印刷页码：")}{selected.printed_page??t("未识别")}</span>{selected.evidence_ids.length>0&&<button type="button" className="text-button" onClick={()=>onSource(selected)}>{t("查看目录原页")}</button>}</div>
      <button className="text-button" onClick={()=>update({target_pdf_page:currentPage-1})}>{t("使用当前预览页（第{0}页）",currentPage)}</button>
    </fieldset>}
    <div className="book-review">
      {problems.missingTargets>0?<p className="attention">{t("还有 {0} 项需要填写有效的目标页。",problems.missingTargets)}</p>:<p>{t("目标页已填写。请查看跳转，并核对层级和标题。")}</p>}
      {problems.emptyTitles>0&&<p className="attention">{t("还有 {0} 项标题为空。",problems.emptyTitles)}</p>}
      <label><input type="checkbox" disabled={disabled||!!problems.missingTargets||!!problems.emptyTitles||!draft.entries.length} checked={draft.reviewed} onChange={e=>onReviewed(e.target.checked)}/>{t("我已核对目录，按当前内容保存")}</label>
    </div>
  </aside>;
}
