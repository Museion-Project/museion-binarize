import { useCallback, useEffect, useRef, useState } from "react";

import type { DocumentSummary } from "../app/types";
import { renderPreview } from "../lib/tauri";
import { PageThumbnail } from "./PageThumbnail";

const THUMBNAIL_DPI = 72;
const THUMBNAIL_MAX_DIMENSION = 160;

interface PageSidebarProps {
  document: DocumentSummary;
  currentPage: number;
  onSelect: (pageNumber: number) => void;
}

/** Vertically scrollable page list with lazily-loaded, cached thumbnails.
 * Only pages that actually scroll into view are ever rendered by the
 * backend — a 100-page document does not render 100 full pages on open.
 * The cache is keyed by document id, so it never survives a document
 * change and never grows unbounded across documents. */
export function PageSidebar({ document, currentPage, onSelect }: PageSidebarProps) {
  const cacheRef = useRef<Map<number, string>>(new Map());
  const [, forceRender] = useState(0);
  const pending = useRef<Set<number>>(new Set());

  useEffect(() => {
    cacheRef.current = new Map();
    pending.current = new Set();
  }, [document.documentId]);

  const fetchThumbnail = useCallback(
    (pageNumber: number) => {
      if (cacheRef.current.has(pageNumber) || pending.current.has(pageNumber)) return;
      pending.current.add(pageNumber);
      renderPreview({
        documentId: document.documentId,
        pageNumber,
        kind: "original",
        dpi: THUMBNAIL_DPI,
        maxDimension: THUMBNAIL_MAX_DIMENSION,
        requestId: pageNumber,
      })
        .then((result) => {
          cacheRef.current.set(pageNumber, `data:image/png;base64,${result.pngBase64}`);
          forceRender((n) => n + 1);
        })
        .catch(() => {
          // A failed thumbnail is not fatal to the rest of the sidebar;
          // it simply stays a placeholder. The main preview pane surfaces
          // the real error for the selected page.
        })
        .finally(() => {
          pending.current.delete(pageNumber);
        });
    },
    [document.documentId],
  );

  return (
    <nav className="page-sidebar" aria-label="Pages">
      <div className="page-sidebar-list" role="listbox" aria-label="Page thumbnails">
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
