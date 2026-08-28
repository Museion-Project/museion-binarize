// The single integration layer between the React app and Tauri. Every
// `invoke()` call and every event name lives here — components never call
// `invoke` or `listen` directly, so the IPC contract has exactly one place
// to change. See docs/desktop.md.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, type DragDropEvent } from "@tauri-apps/api/window";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import type {
  DocumentSummary,
  EstimateResult,
  PdfiumStatus,
  ProcessingCancelled,
  ProcessingCompleted,
  ProcessingFailed,
  ProcessingProgress,
  ProcessingSettings,
  ProcessingStarted,
  ReviewIssue,
  PreviewResult,
  UiError,
  BookmarkCandidate,
  AutoBookmarkRequest,
  AutoBookmarkStarted,
  AutoBookmarkStageEvent,
  AutoBookmarkResult,
  AutoBookmarkFailed,
  ApiRouteOptions,
  ApiCredentialPresence,
  ApiConsentSummary,
  ApiPlanRequest,
  ApiRunRequest,
  ApiTaskProgress,
} from "../app/types";

/** Thrown for every failed command; `error` is the backend's structured DTO. */
export class BackendError extends Error {
  readonly error: UiError;
  constructor(error: UiError) {
    super(error.message);
    this.name = "BackendError";
    this.error = error;
  }
}

export function loadReviewQueue(packagePath: string): Promise<ReviewIssue[]> {
  return call("load_review_queue", { packagePath });
}

export function addReviewRevision(request: {
  packagePath: string;
  revisionId?: string;
  targetRef: string;
  baseEvidenceDigest: string;
  text: string;
  aiSuggested: boolean;
}): Promise<void> {
  return call("add_review_revision", request);
}

/** The effective bookmark tree for a package, as project-owned DTOs. */
export function loadBookmarkTree(packagePath: string): Promise<BookmarkCandidate[]> {
  return call("load_bookmark_tree", { packagePath });
}

/**
 * Starts one automatic table-of-contents run. Resolves as soon as the run is
 * handed to the worker thread; stages and the result arrive as
 * `mpdf://auto-bookmark-*` events, so the UI never blocks on a long book.
 */
export function startAutoBookmark(request: AutoBookmarkRequest): Promise<AutoBookmarkStarted> {
  return call("start_auto_bookmark", { request });
}

export function cancelAutoBookmark(jobId: string, documentId: string): Promise<void> {
  return call("cancel_auto_bookmark", { jobId, documentId });
}

export function onAutoBookmarkStage(
  handler: (payload: AutoBookmarkStageEvent) => void,
): Promise<UnlistenFn> {
  return listen<AutoBookmarkStageEvent>("mpdf://auto-bookmark-stage", (event) =>
    handler(event.payload),
  );
}

export function onAutoBookmarkCompleted(
  handler: (payload: AutoBookmarkResult) => void,
): Promise<UnlistenFn> {
  return listen<AutoBookmarkResult>("mpdf://auto-bookmark-completed", (event) =>
    handler(event.payload),
  );
}

export function onAutoBookmarkCancelled(
  handler: (payload: AutoBookmarkStageEvent) => void,
): Promise<UnlistenFn> {
  return listen<AutoBookmarkStageEvent>("mpdf://auto-bookmark-cancelled", (event) =>
    handler(event.payload),
  );
}

export function onAutoBookmarkFailed(
  handler: (payload: AutoBookmarkFailed) => void,
): Promise<UnlistenFn> {
  return listen<AutoBookmarkFailed>("mpdf://auto-bookmark-failed", (event) =>
    handler(event.payload),
  );
}

/** Opens the native "choose an MDP package folder" dialog. */
export async function pickPackageDirectory(): Promise<string | null> {
  const selection = await openDialog({ multiple: false, directory: true });
  return typeof selection === "string" ? selection : null;
}
export function confirmBookmark(packagePath: string, candidateId: string): Promise<void> { return call("confirm_bookmark", { packagePath, candidateId }); }
export function rejectBookmark(packagePath: string, candidateId: string): Promise<void> { return call("reject_bookmark", { packagePath, candidateId }); }
export function editBookmark(packagePath: string, candidateId: string, title: string): Promise<void> { return call("edit_bookmark", { packagePath, candidateId, title }); }
export function reparentBookmark(packagePath: string, candidateId: string, parentId: string | null, level: number): Promise<void> { return call("reparent_bookmark", { packagePath, candidateId, parentId, level }); }

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    // Structured failures reject with the UiErrorDto itself (Tauri
    // serializes a command's Err(UiErrorDto) as the rejection value).
    if (isUiError(raw)) {
      throw new BackendError(raw);
    }
    throw new BackendError({
      code: "internal_error",
      message: String(raw),
      hint: null,
      detail: null,
    });
  }
}

function isUiError(value: unknown): value is UiError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}

export function projectInfo(): Promise<{ name: string; phase: string }> {
  return call("project_info");
}

export function apiRouteOptions(): Promise<ApiRouteOptions> {
  return call("api_route_options");
}

export function apiCredentialPresence(profileId: string): Promise<ApiCredentialPresence> {
  return call("api_credential_presence", { profileId });
}

export function prepareApiPlan(request: ApiPlanRequest): Promise<ApiConsentSummary> {
  return call("api_prepare_plan", { request });
}

export function runApiTask(request: ApiRunRequest): Promise<ApiTaskProgress> {
  return call("api_run_task", { request });
}

export function cancelApiTask(): Promise<void> {
  return call("api_cancel_current");
}

export function pdfiumStatus(): Promise<PdfiumStatus> {
  return call("pdfium_status");
}

export function openDocument(path: string, password?: string): Promise<DocumentSummary> {
  return call("open_document", { path, password: password ?? null });
}

export function closeDocument(): Promise<void> {
  return call("close_document");
}

export interface PreviewRequest {
  documentId: string;
  pageNumber: number;
  kind: "original" | "processed";
  dpi: number;
  settings?: ProcessingSettings;
  maxDimension?: number;
  requestId: number;
}

export function renderPreview(request: PreviewRequest): Promise<PreviewResult> {
  return call("render_preview", { request });
}

export interface StartProcessingRequest {
  documentId: string;
  outputPath: string;
  settings: ProcessingSettings;
  overwrite: boolean;
}

export function startProcessing(request: StartProcessingRequest): Promise<ProcessingStarted> {
  return call("start_processing", { request });
}

export function cancelProcessing(jobId: string): Promise<void> {
  return call("cancel_processing", { jobId });
}

export interface EstimateRequest {
  documentId: string;
  settings: ProcessingSettings;
  samples: number;
  requestId: number;
}

/** Experimental output-size estimate — see docs/size-estimation.md. Awaits
 * the result directly (unlike `startProcessing`, no events): estimation is
 * bounded and fast enough that the request/response shape `renderPreview`
 * already uses fits better than the fire-and-forget job pattern. */
export function startEstimate(request: EstimateRequest): Promise<EstimateResult> {
  return call("start_estimate", { request });
}

/** Opens the native "choose a PDF" dialog. Returns `null` if the user cancelled. */
export async function pickPdfToOpen(): Promise<string | null> {
  const selection = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "PDF files", extensions: ["pdf"] }],
  });
  return typeof selection === "string" ? selection : null;
}

/** Opens the native "save output as" dialog with a suggested filename. */
export async function pickOutputDestination(defaultFileName: string): Promise<string | null> {
  const selection = await saveDialog({
    defaultPath: defaultFileName,
    filters: [{ name: "PDF files", extensions: ["pdf"] }],
  });
  return selection ?? null;
}

/**
 * Listens for operating-system file drags over the application window.
 * Tauri handles native file drops before the webview, so this must use
 * its window API rather than HTML drag/drop events.
 */
export function onFileDragDrop(
  handler: (event: DragDropEvent) => void,
): Promise<UnlistenFn> {
  return getCurrentWindow().onDragDropEvent((event) => handler(event.payload));
}

// --- Progress event bridge -------------------------------------------
//
// Namespaced `mpdf://processing-*` events, matching the backend (see
// `commands::processing`). Each helper returns the `UnlistenFn` Tauri
// gives back, so callers can detach cleanly on unmount.

export function onProcessingProgress(
  handler: (payload: ProcessingProgress) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingProgress>("mpdf://processing-progress", (event) =>
    handler(event.payload),
  );
}

export function onProcessingCompleted(
  handler: (payload: ProcessingCompleted) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingCompleted>("mpdf://processing-completed", (event) =>
    handler(event.payload),
  );
}

export function onProcessingCancelled(
  handler: (payload: ProcessingCancelled) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingCancelled>("mpdf://processing-cancelled", (event) =>
    handler(event.payload),
  );
}

export function onProcessingFailed(
  handler: (payload: ProcessingFailed) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingFailed>("mpdf://processing-failed", (event) =>
    handler(event.payload),
  );
}
