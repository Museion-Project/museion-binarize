// Shared TypeScript shapes for the desktop app. These mirror the Rust
// DTOs in `apps/desktop/src-tauri/src/dto.rs` field-for-field (camelCase,
// same optionality) — see docs/desktop.md for the IPC contract. Nothing
// here is invented independently of what the backend actually sends.

export interface PageSummary {
  pageNumber: number;
  widthPoints: number;
  heightPoints: number;
  sourceRotationDegrees: number;
}

export interface DocumentSummary {
  documentId: string;
  fileName: string;
  sourceBytes: number;
  pageCount: number;
  title: string | null;
  author: string | null;
  pdfiumLibrary: string;
  pages: PageSummary[];
}

export type BinarizationMethod = "otsu" | "sauvola" | "manual";
export type DespeckleLevel = "off" | "conservative" | "strong";

/** Mirrors `ProcessingSettingsDto`. The one settings shape the whole app uses. */
export interface ProcessingSettings {
  dpi: number;
  method: BinarizationMethod;
  threshold: number | null;
  sauvolaWindowSize: number | null;
  sauvolaK: number | null;
  contrast: number;
  medianDenoise: boolean;
  backgroundNormalization: boolean;
  backgroundRadius: number | null;
  despeckle: DespeckleLevel;
}

export type PresetId = "default" | "fine-detail" | "noisy-scan" | "custom";

export interface PreviewResult {
  requestId: number;
  pageNumber: number;
  kind: "original" | "processed";
  width: number;
  height: number;
  pngBase64: string;
  renderDpi: number;
  isReducedResolution: boolean;
}

export interface ProcessingStarted {
  jobId: string;
  pageCount: number;
}

export interface ProcessingProgress {
  jobId: string;
  stage: string;
  pageNumber: number | null;
  pageCount: number;
  fraction: number;
}

export interface ProcessingCompleted {
  jobId: string;
  outputPath: string;
  pagesProcessed: number;
  originalBytes: number;
  outputBytes: number;
  elapsedUs: number;
  pdfiumLibrary: string;
}

export interface ProcessingCancelled {
  jobId: string;
}

export interface UiError {
  code: string;
  message: string;
  hint: string | null;
  detail: string | null;
}

export interface ProcessingFailed {
  jobId: string;
  error: UiError;
}

export interface PdfiumStatus {
  resolved: boolean;
  description: string | null;
  error: UiError | null;
}
