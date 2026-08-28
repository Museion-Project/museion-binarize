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

/** Explicit M6 routing choices. `local` is the offline-safe default. */
export type ApiRoute = "local" | "api" | "api_then_local";
export interface ApiRouteOptions { routes: ApiRoute[]; defaultRoute: ApiRoute; }
export interface ApiCredentialPresence { profileId: string; present: boolean; }
export interface ApiPlanRequest { documentId: string; endpoint: string; provider: string; model: string; budgetMicros: number; currency: string; retention: "delete_after_result" | "keep_until_deleted"; }
export interface ApiConsentSummary { planDigest: string; origin: string; provider: string; model: string; sourceDigest: string; sourceBytes: number; pageCount: number; budgetMicros: number; currency: string; retention: string; }
export interface ApiRunRequest { plan: ApiPlanRequest; consent: string; profileId: string; route: ApiRoute; }
export interface ApiTaskProgress { taskId: string; state: string; usedCostMicros: number; budgetMicros: number; retention: string; artifactPath: string; fallbackReason: string | null; }

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

/** One bookmark candidate as the backend's `BookmarkCandidateDto`. */
export interface BookmarkCandidate {
  candidateId: string;
  sourceTitle: string;
  effectiveTitle: string;
  effectiveLevel: number;
  effectiveParentId: string | null;
  targetPageId: string;
  physicalPageIndex: number;
  masterBbox: { x: number; y: number; width: number; height: number } | null;
  /**
   * `auto_confirmed` is a deterministic rule decision; `confirmed` is a
   * person's. The two are always displayed distinctly.
   */
  status:
    | "proposed"
    | "needs_review"
    | "confirmed"
    | "rejected"
    | "auto_confirmed"
    | "skipped";
  confidence: number;
  score: BookmarkScore | null;
  alignment: BookmarkAlignment | null;
  automaticReason: string | null;
  reasonCodes: string[];
  evidenceCount: number;
}

/** Integer score components behind an automatic decision. */
export interface BookmarkScore {
  titleMatch: number;
  pageMapping: number;
  numberingHierarchy: number;
  bodyLayout: number;
  ocrQuality: number;
  sequenceUniqueness: number;
  total: number;
  maximum: number;
}

/** Where a compiled entry came from and how its printed page mapped. */
export interface BookmarkAlignment {
  tocPageIndex: number;
  bodyPageIndex: number | null;
  printedLabel: string | null;
  pageResidual: number | null;
  mappingOffset: number | null;
  runnerUpMargin: number;
  secondaryKeyOnly: boolean;
  geometryQuality: string;
}

export interface AutoBookmarkRequest {
  documentId: string;
  packagePath: string;
  outputPath: string;
  overwrite: boolean;
  regenerate: boolean;
}

export interface AutoBookmarkStarted {
  jobId: string;
  documentId: string;
}

export type AutoBookmarkStage =
  | "analyzing_toc"
  | "aligning"
  | "writing_pdf"
  | "validating"
  | "cancelled";

export interface AutoBookmarkStageEvent {
  jobId: string;
  stage: AutoBookmarkStage;
}

export interface AutoBookmarkResult {
  jobId: string;
  documentId: string;
  mode: "existing_outline" | "toc_aligned" | "safe_refusal";
  status: "auto_confirmed" | "needs_review" | "safe_refusal";
  tocPageCount: number;
  parsedEntries: number;
  autoConfirmed: number;
  needsReview: number;
  skipped: number;
  writtenBookmarks: number;
  safeRefusalReason: string | null;
  reportPath: string;
  outputPath: string | null;
}

export interface AutoBookmarkFailed {
  jobId: string;
  error: UiError;
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
