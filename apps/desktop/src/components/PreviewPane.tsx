import { t, localizeMessage } from "../lib/i18n";
import {useEffect,useRef,useState} from "react";
import type {PreviewPaneState} from "../app/reducer";
interface Props {
  preview:PreviewPaneState;viewMode:"original"|"processed";zoom:number;pageNumber:number;
  onViewModeChange:(mode:"original"|"processed")=>void;onZoomChange:(zoom:number)=>void;
  showProcessed?:boolean;pageWidth?:number;pageHeight?:number;
  onOriginalLoaded?:(page:number,dataUrl:string)=>void;
  highlight?:{x:number;y:number;width:number;height:number}|null;
}
export function PreviewPane({preview,viewMode,zoom,pageNumber,onViewModeChange,onZoomChange,showProcessed=true,pageWidth=595,pageHeight=842,highlight,onOriginalLoaded}:Props){
  const canvas=useRef<HTMLDivElement>(null),[fitWidth,setFitWidth]=useState(560);
  useEffect(()=>{
    const element=canvas.current;if(!element)return;
    function measure(){if(element){const rect=element.getBoundingClientRect();setFitWidth(Math.max(100,Math.min(rect.width-40,(rect.height-40)*pageWidth/pageHeight)));}}
    measure();if(typeof ResizeObserver==="undefined")return;
    const observer=new ResizeObserver(measure);observer.observe(element);return ()=>observer.disconnect();
  },[pageWidth,pageHeight]);
  const mode=showProcessed?viewMode:"original",dataUrl=mode==="original"?preview.originalDataUrl:preview.processedDataUrl;
  return <section className="preview-pane" aria-label={t("PDF 第{0}页预览", pageNumber)}>
    <div className="preview-toolbar"><div className="preview-toggle" role="radiogroup" aria-label={t("预览模式")}><button role="radio" aria-checked={mode==="original"} className={mode==="original"?"selected":""} onClick={()=>onViewModeChange("original")}>{t("原页")}</button>{showProcessed&&<button role="radio" aria-checked={mode==="processed"} className={mode==="processed"?"selected":""} onClick={()=>onViewModeChange("processed")}>{t("黑白效果")}</button>}</div><div className="preview-zoom"><button aria-label={t("缩小")} onClick={()=>onZoomChange(Math.max(.25,(zoom||1)-.25))}>−</button><button className={zoom===0?"selected":""} aria-label={t("适合页面")} onClick={()=>onZoomChange(0)}>{t("适合")}</button><button aria-label={t("实际大小")} onClick={()=>onZoomChange(1)}>100%</button><button aria-label={t("放大")} onClick={()=>onZoomChange(Math.min(4,(zoom||1)+.25))}>＋</button></div></div>
    <div className="preview-canvas" ref={canvas}>
      {preview.error?<p className="preview-error" role="alert">{t("预览失败：")}{localizeMessage(preview.error.message)}</p>:dataUrl?<div className="page-sheet" style={{width:zoom===0?fitWidth:pageWidth*zoom}}><img key={dataUrl} onLoad={()=>{if(mode==="original")onOriginalLoaded?.(pageNumber,dataUrl);}} src={dataUrl} alt={t("PDF 第{0}页，{1}", pageNumber, mode==="original"?t("原页"):t("黑白效果"))} draggable={false}/>{highlight&&<div className="evidence-highlight" aria-label={t("目录条目位置")} style={{left:`${highlight.x/pageWidth*100}%`,top:`${highlight.y/pageHeight*100}%`,width:`${highlight.width/pageWidth*100}%`,height:`${highlight.height/pageHeight*100}%`}}/>}</div>:<p className="preview-placeholder">{preview.loading?t("正在生成预览…"):t("页面预览")}</p>}
      {preview.loading&&dataUrl&&<span className="preview-loading" role="status">{t("更新预览…")}</span>}
    </div>
  </section>;
}
