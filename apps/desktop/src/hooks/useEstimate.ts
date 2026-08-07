// Manual, staleness-safe size estimation — unlike preview, this is not
// run automatically on every settings change (estimation processes real
// pages through the full pipeline and is meaningfully more expensive than
// a debounced preview refresh). A settings change marks a ready estimate
// "stale" (see the reducer's SET_SETTINGS case); running a fresh one is
// always an explicit user action. See docs/desktop.md and
// docs/size-estimation.md.

import { useRef } from "react";

import type { Action } from "../app/reducer";
import type { DocumentSummary, ProcessingSettings } from "../app/types";
import { BackendError, startEstimate } from "../lib/tauri";

export const DEFAULT_ESTIMATE_SAMPLES = 8;

/** Returns a `runEstimate` callback. Always called unconditionally (React's
 * rules of hooks): pass `null` for `document`/`settings` when there is
 * nothing to estimate yet — the returned callback becomes a no-op. */
export function useEstimate(
  document: DocumentSummary | null,
  settings: ProcessingSettings | null,
  dispatch: (action: Action) => void,
): () => void {
  const requestCounter = useRef(0);

  return () => {
    if (!document || !settings) return;

    const requestId = ++requestCounter.current;
    dispatch({ type: "ESTIMATE_REQUEST_STARTED", requestId });

    startEstimate({
      documentId: document.documentId,
      settings,
      samples: DEFAULT_ESTIMATE_SAMPLES,
      requestId,
    })
      .then((result) => {
        if (requestId !== requestCounter.current) return;
        dispatch({ type: "ESTIMATE_SUCCEEDED", requestId, result });
      })
      .catch((error: unknown) => {
        if (requestId !== requestCounter.current) return;
        dispatch({
          type: "ESTIMATE_FAILED",
          requestId,
          error: error instanceof BackendError ? error.error : { code: "internal_error", message: String(error), hint: null, detail: null },
        });
      });
  };
}
