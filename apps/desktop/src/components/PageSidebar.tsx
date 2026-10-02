import { t } from "../lib/i18n";
import { useCallback, useEffect, useRef, useState } from "react";

import type { DocumentSummary } from "../app/types";
import { renderPreview } from "../lib/tauri";
import { PageThumbnail } from "./PageThumbnail";

const THUMBNAIL_DPI = 20;
const THUMBNAIL_MAX_DIMENSION = 160;

interface PageSidebarProps {
  document: DocumentSummary;
  currentPage: number;
  onSelect: (pageNumber: number) => void;
  paused?: boolean;
}

/** Vertically scrollable page list with lazily-loaded, cached thumbnails.
 * Only pages that actually scroll into view are ever rendered by the
 * backend — a 100-page document does not render 100 full pages on open.
 * The cache is keyed by document id, so it never survives a document
 * change and never grows unbounded across documents. */
export function PageSidebar({ document, currentPage, onSelect, paused=false }: PageSidebarProps) {
  const cacheRef = useRef<Map<number, string>>(new Map());
  const [, forceRender] = useState(0);
  const pending = useRef<Set<number>>(new Set());

  useEffect(() => {
    cacheRef.current = new Map();
    pending.current = new Set();
  }, [document.documentId]);

  const live=useRef(true),documentId=useRef(document.documentId);
  documentId.current=document.documentId;
  useEffect(()=>{live.current=true;return ()=>{live.current=false;};},[]);
  const queue = useRef<number[]>([]);
  const [queueVersion,setQueueVersion] = useState(0);
  useEffect(()=>{queue.current=[];},[document.documentId]);
  const fetchThumbnail=useCallback((pageNumber:number)=>{
    if(cacheRef.current.has(pageNumber)||pending.current.has(pageNumber)||queue.current.includes(pageNumber))return;
    queue.current.push(pageNumber);setQueueVersion(n=>n+1);
  },[]);
  useEffect(()=>{
    if(paused||pending.current.size||!queue.current.length)return;
    const owner=document.documentId;
    const current=()=>live.current&&documentId.current===owner;
    // Let the main preview enter the worker first. At most one thumbnail is
    // in flight, so it cannot flood the serialized PDFium queue.
    const timer=window.setTimeout(()=>{
      const pages=queue.current.splice(0).sort((a,b)=>Math.abs(a-currentPage)-Math.abs(b-currentPage));
      const pageNumber=pages.shift();queue.current=pages;
      if(pageNumber===undefined)return;
      pending.current.add(pageNumber);
      renderPreview({documentId:document.documentId,pageNumber,kind:"original",dpi:THUMBNAIL_DPI,maxDimension:THUMBNAIL_MAX_DIMENSION,requestId:pageNumber})
        .then(result=>{if(current()){cacheRef.current.set(pageNumber,`data:image/png;base64,${result.pngBase64}`);forceRender(n=>n+1);}})
        .catch(()=>{})
        .finally(()=>{if(current()){pending.current.delete(pageNumber);setQueueVersion(n=>n+1);}});
    },100);
    return ()=>window.clearTimeout(timer);
  },[document.documentId,currentPage,paused,queueVersion]);

  return (
    <nav className="page-sidebar" aria-label={t("页面")}>
      <div className="page-sidebar-list" role="listbox" aria-label={t("页面缩略图")}>
        {document.pages.map((page) => (
          <PageThumbnail
            key={page.pageNumber}
            pageNumber={page.pageNumber}
            selected={page.pageNumber === currentPage}
            dataUrl={cacheRef.current.get(page.pageNumber) ?? null}
            onVisible={fetchThumbnail}
            onSelect={onSelect}
          />
        ))}
      </div>
    </nav>
  );
}
