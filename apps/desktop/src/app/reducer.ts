// The application's state machine. A discriminated union rather than
// scattered booleans (`isLoading`, `isProcessing`, ...) so an invalid
// combination — e.g. "processing" while also "opening a document" —
// cannot be represented, and every transition is a plain, testable
// function. See docs/desktop.md, "Application states".

import { defaultSettings } from "../lib/settings";
import type {
  DocumentSummary,
  EstimateResult,
  PresetId,
  ProcessingCompleted,
  ProcessingSettings,
  UiError,
} from "./types";

/** See docs/desktop.md, "Application states" — a discriminated union
 * rather than `isEstimating`/`estimateError`/`estimateReady`/`estimateStale`
 * booleans, so an invalid combination cannot be represented. */
export type EstimateState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ready"; result: EstimateResult }
  | { kind: "stale"; previous: EstimateResult }
  | { kind: "failed"; error: UiError };

const idleEstimate: EstimateState = { kind: "idle" };

export interface PreviewPaneState {
  originalDataUrl: string | null;
  processedDataUrl: string | null;
  loading: boolean;
  error: UiError | null;
}

const emptyPreview: PreviewPaneState = {
  originalDataUrl: null,
  processedDataUrl: null,
  loading: false,
  error: null,
};

export interface JobProgress {
  jobId: string;
  stage: string;
  pageNumber: number | null;
  pageCount: number;
  fraction: number;
}

export type AppState =
  | { kind: "idle" }
  | { kind: "opening" }
  | { kind: "passwordRequired"; path: string; attemptError: string | null }
  | {
      kind: "ready";
      document: DocumentSummary;
      currentPage: number;
      settings: ProcessingSettings;
      preset: PresetId;
      viewMode: "original" | "processed";
      zoom: number; // 0 means "fit"; otherwise a multiplier, 1 = 100%
      outputPath: string | null;
      preview: PreviewPaneState;
      /** The requestId of the newest preview request issued, so a late
       * response to an older request can be told apart and dropped. */
      latestRequestId: number;
      estimate: EstimateState;
      /** Same staleness pattern as `latestRequestId`, for estimate
       * requests. A separate counter: preview and estimate requests are
       * independent operations and must not share a generation id. */
      latestEstimateRequestId: number;
    }
  | {
      kind: "processing";
      document: DocumentSummary;
      settings: ProcessingSettings;
      outputPath: string;
      job: JobProgress;
      cancelling: boolean;
    }
  | {
      kind: "completed";
      document: DocumentSummary;
      settings: ProcessingSettings;
      report: ProcessingCompleted;
    }
  | { kind: "cancelled"; document: DocumentSummary; settings: ProcessingSettings }
  | { kind: "failed"; document: DocumentSummary | null; error: UiError };

export type Action =
  | { type: "OPEN_STARTED" }
  | { type: "OPEN_SUCCEEDED"; document: DocumentSummary }
  | { type: "OPEN_FAILED"; error: UiError }
  | { type: "PASSWORD_REQUIRED"; path: string }
  | { type: "PASSWORD_RETRY_FAILED"; message: string }
  | { type: "SELECT_PAGE"; page: number }
  | { type: "SET_SETTINGS"; settings: ProcessingSettings; preset: PresetId }
  | { type: "SET_VIEW_MODE"; mode: "original" | "processed" }
  | { type: "SET_ZOOM"; zoom: number }
  | { type: "SET_OUTPUT_PATH"; path: string }
  | { type: "PREVIEW_REQUEST_STARTED"; requestId: number }
  | {
      type: "PREVIEW_SUCCEEDED";
      requestId: number;
      kind: "original" | "processed";
      dataUrl: string;
    }
  | { type: "PREVIEW_FAILED"; requestId: number; error: UiError }
  | { type: "ESTIMATE_REQUEST_STARTED"; requestId: number }
  | { type: "ESTIMATE_SUCCEEDED"; requestId: number; result: EstimateResult }
  | { type: "ESTIMATE_FAILED"; requestId: number; error: UiError }
  | { type: "PROCESSING_STARTED"; jobId: string; pageCount: number; outputPath: string }
  | {
      type: "PROCESSING_PROGRESS";
      jobId: string;
      stage: string;
      pageNumber: number | null;
      pageCount: number;
      fraction: number;
    }
  | { type: "CANCEL_REQUESTED"; jobId: string }
  | { type: "PROCESSING_COMPLETED"; jobId: string; report: ProcessingCompleted }
  | { type: "PROCESSING_CANCELLED"; jobId: string }
  | { type: "PROCESSING_FAILED"; jobId: string; error: UiError }
  | { type: "START_OVER" }
  | { type: "DISMISS_ERROR" };

export const initialState: AppState = { kind: "idle" };

export function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case "OPEN_STARTED":
      return { kind: "opening" };

    case "OPEN_SUCCEEDED":
      return {
        kind: "ready",
        document: action.document,
        currentPage: 1,
        settings: state.kind === "ready" ? state.settings : defaultSettings(),
        preset: "default",
        viewMode: "processed",
        zoom: 0,
        outputPath: null,
        preview: emptyPreview,
        latestRequestId: 0,
        estimate: idleEstimate,
        latestEstimateRequestId: 0,
      };

    case "OPEN_FAILED":
      return { kind: "failed", document: null, error: action.error };

    case "PASSWORD_REQUIRED":
      return { kind: "passwordRequired", path: action.path, attemptError: null };

    case "PASSWORD_RETRY_FAILED":
      if (state.kind !== "passwordRequired") return state;
      return { ...state, attemptError: action.message };

    case "SELECT_PAGE": {
      if (state.kind !== "ready") return state;
      return { ...state, currentPage: action.page, preview: emptyPreview };
    }

    case "SET_SETTINGS": {
      if (state.kind !== "ready") return state;
      return {
        ...state,
        settings: action.settings,
        preset: action.preset,
        preview: { ...state.preview, processedDataUrl: null },
        // A settings change invalidates a *ready* estimate (it was
        // computed for the settings that just changed) but a running one
        // is left alone — the hook that debounces estimate requests will
        // issue a fresh one, which naturally supersedes it via the
        // requestId check below. A "stale" or "failed" estimate has
        // already been invalidated by an earlier change and is not
        // re-wrapped.
        estimate:
          state.estimate.kind === "ready"
            ? { kind: "stale", previous: state.estimate.result }
            : state.estimate,
      };
    }

    case "SET_VIEW_MODE": {
      if (state.kind !== "ready") return state;
      return { ...state, viewMode: action.mode };
    }

    case "SET_ZOOM": {
      if (state.kind !== "ready") return state;
      return { ...state, zoom: action.zoom };
    }

    case "SET_OUTPUT_PATH": {
      if (state.kind !== "ready") return state;
      return { ...state, outputPath: action.path };
    }

    case "PREVIEW_REQUEST_STARTED": {
      if (state.kind !== "ready") return state;
      return {
        ...state,
        latestRequestId: action.requestId,
        preview: { ...state.preview, loading: true, error: null },
      };
    }

    case "PREVIEW_SUCCEEDED": {
      if (state.kind !== "ready" || action.requestId !== state.latestRequestId) return state;
      return {
        ...state,
        preview: {
          ...state.preview,
          loading: false,
          error: null,
          originalDataUrl:
            action.kind === "original" ? action.dataUrl : state.preview.originalDataUrl,
          processedDataUrl:
            action.kind === "processed" ? action.dataUrl : state.preview.processedDataUrl,
        },
      };
    }

    case "PREVIEW_FAILED": {
      if (state.kind !== "ready" || action.requestId !== state.latestRequestId) return state;
      return { ...state, preview: { ...state.preview, loading: false, error: action.error } };
    }

    case "ESTIMATE_REQUEST_STARTED": {
      if (state.kind !== "ready") return state;
      return {
        ...state,
        latestEstimateRequestId: action.requestId,
        estimate: { kind: "running" },
      };
    }

    case "ESTIMATE_SUCCEEDED": {
      if (state.kind !== "ready" || action.requestId !== state.latestEstimateRequestId) {
        return state;
      }
      return { ...state, estimate: { kind: "ready", result: action.result } };
    }

    case "ESTIMATE_FAILED": {
      if (state.kind !== "ready" || action.requestId !== state.latestEstimateRequestId) {
        return state;
      }
      return { ...state, estimate: { kind: "failed", error: action.error } };
    }

    case "PROCESSING_STARTED": {
      if (state.kind !== "ready") return state;
      return {
        kind: "processing",
        document: state.document,
        settings: state.settings,
        outputPath: action.outputPath,
        cancelling: false,
        job: {
          jobId: action.jobId,
          stage: "queued",
          pageNumber: null,
          pageCount: action.pageCount,
          fraction: 0,
        },
      };
    }

    case "PROCESSING_PROGRESS": {
      if (state.kind !== "processing" || action.jobId !== state.job.jobId) return state;
      return {
        ...state,
        job: {
          jobId: action.jobId,
          stage: action.stage,
          pageNumber: action.pageNumber,
          pageCount: action.pageCount,
          fraction: action.fraction,
        },
      };
    }

    case "CANCEL_REQUESTED": {
      if (state.kind !== "processing" || action.jobId !== state.job.jobId) return state;
      return { ...state, cancelling: true };
    }

    case "PROCESSING_COMPLETED": {
      if (state.kind !== "processing" || action.jobId !== state.job.jobId) return state;
      return {
        kind: "completed",
        document: state.document,
        settings: state.settings,
        report: action.report,
      };
    }

    case "PROCESSING_CANCELLED": {
      if (state.kind !== "processing" || action.jobId !== state.job.jobId) return state;
      return { kind: "cancelled", document: state.document, settings: state.settings };
    }

    case "PROCESSING_FAILED": {
      if (state.kind !== "processing" || action.jobId !== state.job.jobId) return state;
      return { kind: "failed", document: state.document, error: action.error };
    }

    case "START_OVER": {
      const document =
        state.kind === "completed" || state.kind === "cancelled"
          ? state.document
          : state.kind === "failed"
            ? state.document
            : null;
      if (!document) return { kind: "idle" };
      return {
        kind: "ready",
        document,
        currentPage: 1,
        settings: "settings" in state ? state.settings : defaultSettings(),
        preset: "default",
        viewMode: "processed",
        zoom: 0,
        outputPath: null,
        preview: emptyPreview,
        latestRequestId: 0,
        estimate: idleEstimate,
        latestEstimateRequestId: 0,
      };
    }

    case "DISMISS_ERROR": {
      if (state.kind !== "failed") return state;
      return state.document
        ? {
            kind: "ready",
            document: state.document,
            currentPage: 1,
            settings: defaultSettings(),
            preset: "default",
            viewMode: "processed",
            zoom: 0,
            outputPath: null,
            preview: emptyPreview,
            latestRequestId: 0,
            estimate: idleEstimate,
            latestEstimateRequestId: 0,
          }
        : { kind: "idle" };
    }

    default:
      return state;
  }
}
