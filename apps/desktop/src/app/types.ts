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

/**
 * The main flow, in the order `mpdf_core::orchestrator` enforces. The list is
 * sent by the backend on start so the UI never invents its own sequence.
 */
export type PipelineStage =
  | "analyzing_source"
  | "ocr_original"
  | "deriving_text"
  | "generating_bookmarks"
  | "awaiting_review"
  | "binarizing_visuals"
  | "assembling_final_pdf"
  | "validating"
  | "cancelled";

/** Human labels for each stage. Kept beside the type so they cannot drift. */
export const PIPELINE_STAGE_LABELS: Record<PipelineStage, string> = {
  analyzing_source: "Reading the original PDF",
  ocr_original: "Recognizing text on the original pages",
  deriving_text: "Building the text layer",
  generating_bookmarks: "Compiling bookmarks",
  awaiting_review: "Waiting for your review",
  binarizing_visuals: "Binarizing the visible pages",
  assembling_final_pdf: "Assembling the final PDF",
  validating: "Verifying the finished file",
  cancelled: "Cancelled",
};

export type ReviewPolicy = "pause" | "confirmed" | "reviewed";

export interface LocalPipelineRequest {
  documentId: string;
  outputPath: string;
  workspacePath: string;
  settings: ProcessingSettings;
  languageProfile: string;
  engine?: string | null;
  providerExecutable?: string | null;
  modelDir?: string | null;
  onReview: ReviewPolicy;
  overwrite: boolean;
  ocrDpi?: number | null;
  /** Omitted means local. A cloud mode is never the fallback of an omission. */
  ocrProviderMode?: OcrProviderModeId | null;
  /** The user confirmed that page images will be uploaded. */
  cloudConsent?: boolean;
  /** A slot *label*. There is no field anywhere that carries a key. */
  credentialSlot?: string | null;
  cloudFallback?: CloudFallback | null;
  cloudEndpoint?: string | null;
  maxCredits?: number | null;
  creditsPerPage?: number | null;
}

export type OcrProviderModeId = "local" | "gemini-byok" | "mpdf-credits";

/** What to do when a cloud page fails. */
export type CloudFallback = "local" | "fail";

export interface OcrProviderMode {
  id: OcrProviderModeId;
  displayName: string;
  defaultMode: boolean;
  usesNetwork: boolean;
  requiresCredential: boolean;
  executionLocation: string;
  productionReady: boolean;
  availability: "stable" | "beta" | "unavailable";
  /** Why this mode may not be used for real work. Rendered verbatim. */
  blockers: string[];
  model: string | null;
  credentialPresent: boolean;
  structuredBboxDefaultEnabled: boolean;
}

export interface OcrProviderStatus {
  defaultMode: OcrProviderModeId;
  modes: OcrProviderMode[];
}

/**
 * Everything the app is ever told about a stored key: which slot, whether
 * something is in it, and a constant mask. There is deliberately no field
 * that could hold a prefix, a suffix, a length or a digest.
 */
export interface MaskedCredential {
  slot: string;
  present: boolean;
  masked: string;
}

export interface ConnectionTest {
  mode: string;
  providerName: string;
  model: string;
  modelAvailable: boolean;
  credential: MaskedCredential;
  /** Already redacted and length-bounded by the backend. */
  diagnostic: string;
}

export interface LocalPipelineStarted {
  jobId: string;
  documentId: string;
  workspacePath: string;
  stages: PipelineStage[];
  resumed: boolean;
}

export interface LocalPipelineStageEvent {
  jobId: string;
  documentId: string;
  stage: PipelineStage;
}

export interface LocalPipelineCompleted {
  jobId: string;
  documentId: string;
  /** How the run itself ended. A bookmark refusal is not a run status. */
  status: "completed" | "awaiting_review";
  /** What happened to the outline, independently of `status`. */
  bookmarkStatus: "written" | "none_confirmed" | "safe_refusal";
  stageReached: PipelineStage;
  outputPath: string | null;
  pageCount: number;
  ocrPages: number;
  ocrErrors: number;
  bookmarksWritten: number;
  bookmarksAutoConfirmed: number;
  bookmarksNeedingReview: number;
  bookmarksSkipped: number;
  textLayerWords: number;
  refusalReason: string | null;
  ocrProvenance: string[];
  providerMode: OcrProviderModeId;
  /** 1-based page numbers whose text came from the local engine. */
  cloudFallbackPages: number[];
  cloudInputTokens: number;
  cloudOutputTokens: number;
  cloudUsageMayBeIncomplete: boolean;
  creditsReserved: number;
  creditsCharged: number;
  creditsReleased: number;
  creditsRefunded: number;
  creditsSettled: boolean;
  notProductionReadyReason: string | null;
}

export interface LocalPipelineFailed {
  jobId: string;
  documentId: string;
  error: UiError;
}

export interface LocalOcrReadinessRequest {
  languageProfile: string;
  engine?: string | null;
  providerExecutable?: string | null;
  modelDir?: string | null;
}

export interface LocalOcrReadiness {
  ready: boolean;
  engine: string;
  languageProfile: string;
  clearedForProduction: boolean;
  missingFiles: string[];
  modelSet: string | null;
  modelLicense: string | null;
  diagnostic: string;
  availableProfiles: string[];
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
  documentId: string;
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
  documentId: string;
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
