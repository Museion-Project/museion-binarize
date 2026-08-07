// Debounced, staleness-safe preview fetching for the current page.
//
// A settings change must invalidate only the processed preview (not
// re-fetch the original), a page change must invalidate both, and a
// rapid sequence of changes must not fire one request per keystroke or
// let an old response overwrite a newer one. See docs/desktop.md,
// "Preview invalidation".

import { useEffect, useRef } from "react";

import type { Action } from "../app/reducer";
import type { DocumentSummary, ProcessingSettings } from "../app/types";
import { renderPreview } from "../lib/tauri";

const DEBOUNCE_MS = 200;
const THUMBNAIL_DPI = 72;
const MAIN_PREVIEW_MAX_DIMENSION = 1400;

async function toDataUrl(pngBase64: string): Promise<string> {
  return `data:image/png;base64,${pngBase64}`;
}

/** Fetches both the original and processed preview for `currentPage`,
 * debounced, and dispatches the result only if it is still the newest
 * request by the time it resolves.
 *
 * Always called unconditionally (React's rules of hooks): pass `null` for
 * `document`/`settings` when there is nothing to preview yet, and the
 * effect below simply does nothing. */
export function usePreview(
  document: DocumentSummary | null,
  currentPage: number,
  settings: ProcessingSettings | null,
  dispatch: (action: Action) => void,
): void {
  const requestCounter = useRef(0);

  useEffect(() => {
    if (!document || !settings) return;

    const timer = window.setTimeout(() => {
      const requestId = ++requestCounter.current;
      dispatch({ type: "PREVIEW_REQUEST_STARTED", requestId });

      const original = renderPreview({
        documentId: document.documentId,
        pageNumber: currentPage,
        kind: "original",
        dpi: settings.dpi,
        maxDimension: MAIN_PREVIEW_MAX_DIMENSION,
        requestId,
      });
      const processed = renderPreview({
        documentId: document.documentId,
        pageNumber: currentPage,
        kind: "processed",
        dpi: settings.dpi,
        settings,
        maxDimension: MAIN_PREVIEW_MAX_DIMENSION,
        requestId,
      });

      original
        .then(async (result) => {
          if (requestId !== requestCounter.current) return;
          dispatch({
            type: "PREVIEW_SUCCEEDED",
            requestId,
            kind: "original",
            dataUrl: await toDataUrl(result.pngBase64),
          });
        })
        .catch((error) => {
          if (requestId !== requestCounter.current) return;
          dispatch({ type: "PREVIEW_FAILED", requestId, error: error.error ?? error });
        });

      processed
        .then(async (result) => {
          if (requestId !== requestCounter.current) return;
          dispatch({
            type: "PREVIEW_SUCCEEDED",
            requestId,
            kind: "processed",
            dataUrl: await toDataUrl(result.pngBase64),
          });
        })
        .catch((error) => {
          if (requestId !== requestCounter.current) return;
          dispatch({ type: "PREVIEW_FAILED", requestId, error: error.error ?? error });
        });
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [document?.documentId, currentPage, JSON.stringify(settings)]);
}

export { THUMBNAIL_DPI };
