import { t } from "../lib/i18n";
import {useEffect,useState} from "react";
import type {DocumentAnalysisStatus} from "../app/localTools";
import {documentAnalysisStatus} from "../lib/tauri";

/** Observe the document-open job; this does not launch another analysis. */
export function useDocumentAnalysis(documentId:string|null){
  const [status,setStatus]=useState<DocumentAnalysisStatus|null>(null);
  useEffect(()=>{
    let disposed=false,timer:number|undefined;
    if(!documentId){setStatus(null);return;}
    setStatus({documentId,textLayer:"checking",paginationStatus:"running",sequenceCount:0,sampledPages:0,message:null});
    const refresh=async()=>{
      try{
        const value=await documentAnalysisStatus(documentId);
        if(disposed||value.documentId!==documentId)return;
        setStatus(value);
        if(value.paginationStatus==="running")timer=window.setTimeout(()=>void refresh(),450);
      }catch{
        if(!disposed)setStatus({documentId,textLayer:"unavailable",paginationStatus:"unavailable",sequenceCount:0,sampledPages:0,message:t("文档信息暂不可用。")});
      }
    };
    void refresh();
    return ()=>{disposed=true;if(timer!==undefined)window.clearTimeout(timer);};
  },[documentId]);
  return status?.documentId===documentId?status:null;
}
