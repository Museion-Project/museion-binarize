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
  PreviewResult,
  UiError,
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
// Namespaced `museion://processing-*` events, matching the backend (see
// `commands::processing`). Each helper returns the `UnlistenFn` Tauri
// gives back, so callers can detach cleanly on unmount.

export function onProcessingProgress(
  handler: (payload: ProcessingProgress) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingProgress>("museion://processing-progress", (event) =>
    handler(event.payload),
  );
}

export function onProcessingCompleted(
  handler: (payload: ProcessingCompleted) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingCompleted>("museion://processing-completed", (event) =>
    handler(event.payload),
  );
}

export function onProcessingCancelled(
  handler: (payload: ProcessingCancelled) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingCancelled>("museion://processing-cancelled", (event) =>
    handler(event.payload),
  );
}

export function onProcessingFailed(
  handler: (payload: ProcessingFailed) => void,
): Promise<UnlistenFn> {
  return listen<ProcessingFailed>("museion://processing-failed", (event) =>
    handler(event.payload),
  );
}
