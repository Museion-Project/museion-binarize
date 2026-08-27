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

export interface PageExtreme {
  pageNumber: number;
  value: number;
}

export interface EstimateComparison {
  estimatedOutputBytes: number;
  actualOutputBytes: number;
  absoluteErrorBytes: number;
  relativeErrorFraction: number;
}

export interface ProcessingCompleted {
  jobId: string;
  outputPath: string;
  pagesProcessed: number;
  originalBytes: number;
  outputBytes: number;
  elapsedUs: number;
  pdfiumLibrary: string;
  absoluteBytesSaved: number;
  sizeReductionFraction: number | null;
  inputToOutputRatio: number | null;
  medianProcessingDurationUs: number;
  overallBlackPixelRatio: number;
  slowestPage: PageExtreme | null;
  largestEncodedPage: PageExtreme | null;
  smallestEncodedPage: PageExtreme | null;
  estimateComparison: EstimateComparison | null;
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

export interface ReviewIssue {
  issueId: string;
  targetRef: string;
  pageId: string;
  pageIndex: number;
  bbox: { x: number; y: number; width: number; height: number };
  baseEvidenceDigest: string;
  kind: "low_confidence" | "reading_order_gap" | "unicode_normalization" | "empty_region";
  severity: "info" | "warning" | "error";
  reason: string;
  status: "open";
  coordinateSpace?: string | null;
  sourceText?: string | null;
  effectiveText?: string | null;
  confidence?: number | null;
}

export interface BookmarkCandidate {
  candidateId: string;
  sourceTitle: string;
  effectiveTitle: string;
  sourceLevel: number;
  effectiveLevel: number;
  sourceParentId: string | null;
  effectiveParentId: string | null;
  targetPageId: string;
  physicalPageIndex: number;
  masterBbox: { x: number; y: number; width: number; height: number } | null;
  evidence: unknown[];
  confidence: number;
  status: "proposed" | "needs_review" | "confirmed" | "rejected";
  reasonCodes: string[];
  ruleTrace: string[];
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

export interface PageSizeEstimateSample {
  pageNumber: number;
  rasterWidth: number;
  rasterHeight: number;
  blackPixelRatio: number;
  ccittBytes: number;
  bytesPerPixel: number;
}

export type EstimationRangeMethod = "quartiles" | "min_max";

export interface EstimateResult {
  requestId: number;
  documentPageCount: number;
  sampledPages: PageSizeEstimateSample[];
  estimatedOutputBytes: number;
  estimatedLowerBytes: number;
  estimatedUpperBytes: number;
  rangeMethod: EstimationRangeMethod;
  dpi: number;
  method: BinarizationMethod;
  estimateTotalDurationUs: number;
  experimental: boolean;
}
