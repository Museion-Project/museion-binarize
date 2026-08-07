// Subscribes once to the museion://processing-* event namespace and turns
// each event into a reducer action. The reducer itself drops any event
// whose jobId does not match the job currently in state, so a stale event
// from a previous job can never corrupt newer state (docs/desktop.md,
// "Concurrency and race-condition review").

import { useEffect } from "react";

import type { Action } from "../app/reducer";
import {
  onProcessingCancelled,
  onProcessingCompleted,
  onProcessingFailed,
  onProcessingProgress,
} from "../lib/tauri";

export function useProcessingEvents(dispatch: (action: Action) => void): void {
  useEffect(() => {
    const unlisten = [
      onProcessingProgress((payload) =>
        dispatch({
          type: "PROCESSING_PROGRESS",
          jobId: payload.jobId,
          stage: payload.stage,
          pageNumber: payload.pageNumber,
          pageCount: payload.pageCount,
          fraction: payload.fraction,
        }),
      ),
      onProcessingCompleted((payload) =>
        dispatch({ type: "PROCESSING_COMPLETED", jobId: payload.jobId, report: payload }),
      ),
      onProcessingCancelled((payload) =>
        dispatch({ type: "PROCESSING_CANCELLED", jobId: payload.jobId }),
      ),
      onProcessingFailed((payload) =>
        dispatch({ type: "PROCESSING_FAILED", jobId: payload.jobId, error: payload.error }),
      ),
    ];

    return () => {
      unlisten.forEach((p) => p?.then((fn) => fn()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
