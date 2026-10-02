// Original pages are screen-sized and cached; black-and-white previews are
// computed only when requested, at the actual output DPI/settings.
import {useEffect,useRef} from "react";
import type {Action} from "../app/reducer";
import type {DocumentSummary,ProcessingSettings} from "../app/types";
import {renderPreview} from "../lib/tauri";

const MAIN_PREVIEW_MAX_DIMENSION=1400;
const CACHE_BYTES=16*1024*1024;
export const THUMBNAIL_DPI=20;

export function usePreview(document:DocumentSummary|null,currentPage:number,settings:ProcessingSettings|null,dispatch:(action:Action)=>void,processedEnabled=true,viewMode:"original"|"processed"="original"){
  const requestCounter=useRef(0);
  const cache=useRef<{documentId:string|null;images:Map<string,string>}>({documentId:null,images:new Map()});
  const kind=processedEnabled?viewMode:"original";
  const fingerprint=kind==="processed"?JSON.stringify(settings):"";
  const page=document?.pages[currentPage-1];
  const dpi=kind==="processed"?(settings?.dpi??400):Math.min(600,Math.max(1,Math.floor(MAIN_PREVIEW_MAX_DIMENSION*72/Math.max(page?.widthPoints??595,page?.heightPoints??842))));
  useEffect(()=>{
    if(!document||!settings)return;
    if(cache.current.documentId!==document.documentId)cache.current={documentId:document.documentId,images:new Map()};
    const images=cache.current.images,key=`${currentPage}:${kind}:${dpi}:${fingerprint}`;
    let disposed=false;const requestId=++requestCounter.current;
    const success=(dataUrl:string)=>{if(!disposed)dispatch({type:"PREVIEW_SUCCEEDED",requestId,kind,dataUrl});};
    dispatch({type:"PREVIEW_REQUEST_STARTED",requestId});
    const hit=images.get(key);
    if(hit){images.delete(key);images.set(key,hit);success(hit);return ()=>{disposed=true;};}
    // A short page debounce drops rapid navigation; expensive settings edits
    // get a longer debounce, without delaying original-page display.
    const timer=window.setTimeout(()=>{
      renderPreview({documentId:document.documentId,pageNumber:currentPage,kind,dpi,settings:kind==="processed"?settings:undefined,maxDimension:MAIN_PREVIEW_MAX_DIMENSION,requestId})
        .then(result=>{
          if(disposed)return;
          const dataUrl=`data:image/png;base64,${result.pngBase64}`;
          images.set(key,dataUrl);
          let bytes=0;for(const value of images.values())bytes+=value.length*2;
          while(bytes>CACHE_BYTES&&images.size){const oldest=images.keys().next().value!;bytes-=images.get(oldest)!.length*2;images.delete(oldest);}
          success(dataUrl);
        })
        .catch(error=>{if(!disposed)dispatch({type:"PREVIEW_FAILED",requestId,error:error.error??error});});
    },kind==="original"?40:180);
    return ()=>{disposed=true;window.clearTimeout(timer);};
    // Only the active preview's inputs affect its request. Settings changes do
    // not invalidate originals; document identity bounds cache ownership.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  },[document?.documentId,currentPage,kind,dpi,fingerprint]);
}
