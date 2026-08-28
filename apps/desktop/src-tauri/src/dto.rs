//! Serializable types crossing the Tauri IPC boundary.
//!
//! Every type here is project-owned and UI-appropriate — never a raw
//! `mpdf_core` type serialized directly, so a core refactor
//! cannot silently change the frontend's contract. Field names are
//! `camelCase` for TypeScript ergonomics; doc comments note units.
//!
//! No password ever appears in any type here: `open_document` accepts one
//! as a plain `String` argument (never logged, never stored past the one
//! call — see `commands::document`), and nothing in this module can carry
//! it back out.

use serde::{Deserialize, Serialize};

use mpdf_core::document::PdfDocumentInfo;
use mpdf_core::estimation::{PageSizeEstimateSample, SizeEstimateReport};
use mpdf_core::jobs::{JobProgress, JobStatus};
use mpdf_core::pipeline::ProcessingReport;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiRouteOptionsDto {
    pub routes: Vec<String>,
    pub default_route: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiCredentialPresenceDto {
    pub profile_id: String,
    pub present: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiPlanRequestDto {
    pub document_id: String,
    pub endpoint: String,
    pub provider: String,
    pub model: String,
    pub budget_micros: u64,
    pub currency: String,
    pub retention: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiConsentSummaryDto {
    pub plan_digest: String,
    pub origin: String,
    pub provider: String,
    pub model: String,
    pub source_digest: String,
    pub source_bytes: u64,
    pub page_count: u32,
    pub budget_micros: u64,
    pub currency: String,
    pub retention: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiRunRequestDto {
    pub plan: ApiPlanRequestDto,
    pub consent: String,
    pub profile_id: String,
    pub route: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTaskProgressDto {
    pub task_id: String,
    pub state: String,
    pub used_cost_micros: u64,
    pub budget_micros: u64,
    pub retention: String,
    pub artifact_path: String,
    pub fallback_reason: Option<String>,
}

/// One page's geometry, as reported after a document opens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSummaryDto {
    /// One-based, matching what the user sees.
    pub page_number: u32,
    pub width_points: f32,
    pub height_points: f32,
    pub source_rotation_degrees: u32,
}

/// Everything the UI shows immediately after a document opens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummaryDto {
    /// Opaque identifier for the one document session this window can have
    /// open at a time (see `docs/desktop.md` for the one-document-per-
    /// window decision). Echoed back by every later command that needs to
    /// prove it is talking about the still-open document, not a stale one.
    pub document_id: String,
    pub file_name: String,
    pub source_bytes: u64,
    pub page_count: u32,
    pub title: Option<String>,
    pub author: Option<String>,
    pub pdfium_library: String,
    pub pages: Vec<PageSummaryDto>,
}

impl DocumentSummaryDto {
    pub fn build(
        document_id: String,
        file_name: String,
        info: &PdfDocumentInfo,
        pdfium_library: String,
    ) -> Self {
        Self {
            document_id,
            file_name,
            source_bytes: info.source_bytes,
            page_count: info.page_count,
            title: info.metadata.title.clone(),
            author: info.metadata.author.clone(),
            pdfium_library,
            pages: info
                .pages
                .iter()
                .map(|page| PageSummaryDto {
                    page_number: page.page_number(),
                    width_points: page.geometry.width_points,
                    height_points: page.geometry.height_points,
                    source_rotation_degrees: page.source_rotation.degrees(),
                })
                .collect(),
        }
    }
}

/// The processing settings the frontend can express, mirroring
/// `mpdf_core::settings::ProcessingSettings` field-for-field.
/// One conversion point (`settings::to_processing_settings`) turns this
/// into the real settings type; nothing downstream ever sees this DTO.
/// The backend validates every field itself — frontend control ranges are
/// a convenience, never trusted as the actual bound.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingSettingsDto {
    pub dpi: u16,
    /// `"otsu"`, `"sauvola"`, or `"manual"`.
    pub method: String,
    /// Required when `method == "manual"`.
    pub threshold: Option<u8>,
    /// Sauvola window size (odd). `None` uses the DPI-derived default.
    pub sauvola_window_size: Option<u32>,
    /// Sauvola sensitivity constant. `None` uses the core default.
    pub sauvola_k: Option<f32>,
    pub contrast: f32,
    pub median_denoise: bool,
    pub background_normalization: bool,
    pub background_radius: Option<u32>,
    /// `"off"`, `"conservative"`, or `"strong"`.
    pub despeckle: String,
}

/// A request to render one page, either in its original rasterized form
/// or through the real processing pipeline.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequestDto {
    pub document_id: String,
    /// One-based.
    pub page_number: u32,
    /// `"original"` or `"processed"`.
    pub kind: String,
    /// Render resolution. The frontend chooses this explicitly — low for
    /// a thumbnail, the selected conversion DPI for the main preview —
    /// rather than the backend guessing an intent from `kind` alone.
    pub dpi: u16,
    /// Required when `kind == "processed"`.
    pub settings: Option<ProcessingSettingsDto>,
    /// Caps the longest side of the returned image, in pixels. The render
    /// itself still happens at `dpi` (so the processed preview reflects
    /// the real algorithm output at the real settings); this only bounds
    /// what crosses the IPC boundary and what the UI displays. `None`
    /// returns the image at its rendered size.
    pub max_dimension: Option<u32>,
    /// A generation id the frontend assigns and increments per request,
    /// so it can discard a response that arrives after a newer request
    /// was already issued (see `docs/desktop.md` on preview staleness).
    /// Opaque to the backend — only ever echoed back.
    pub request_id: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResultDto {
    pub request_id: u64,
    pub page_number: u32,
    pub kind: String,
    pub width: u32,
    pub height: u32,
    /// Base64-encoded PNG bytes. See `docs/desktop.md` for why previews
    /// cross the IPC boundary as inline PNG bytes rather than a
    /// filesystem path or a raw pixel array.
    pub png_base64: String,
    pub render_dpi: u16,
    /// `true` when `max_dimension` caused this image to be downscaled
    /// after rendering — the UI may want to note that the preview is not
    /// pixel-for-pixel what a full render at this DPI would show.
    pub is_reduced_resolution: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartProcessingRequestDto {
    pub document_id: String,
    pub output_path: String,
    pub settings: ProcessingSettingsDto,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingStartedDto {
    pub job_id: String,
    pub page_count: u32,
}

/// Durable M2 task progress exposed to the desktop shell. Unlike the fine
/// grained processing event below, this snapshot can be reconstructed after
/// a restart from SQLite and never implies that OCR has run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // M2 command wiring lands with the desktop job controller.
pub struct PersistentJobProgressDto {
    pub job_id: String,
    pub status: String,
    pub total_pages: u32,
    pub completed_pages: u32,
    pub failed_pages: u32,
    pub cancelled_pages: u32,
}

/// Local OCR settings shown by the desktop controller. Paths are explicit
/// operator configuration; the backend never discovers or downloads a
/// provider executable/model.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // consumed by the forthcoming durable OCR controller
pub struct LocalOcrSettingsDto {
    pub provider: String,
    pub provider_executable: Option<String>,
    pub model_dir: Option<String>,
    pub jobs_db: String,
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrProviderStatusDto {
    pub provider: String,
    pub available: bool,
    pub diagnostic: String,
}

/// Durable OCR status returned from the SQLite job store. Page errors are
/// intentionally separate from provider settings so the UI can refresh them
/// after a restart without retaining document content in memory.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrJobStatusDto {
    pub job_id: String,
    pub status: String,
    pub total_pages: u32,
    pub completed_pages: u32,
    pub failed_pages: u32,
    pub cancelled_pages: u32,
    pub page_errors: Vec<LocalOcrPageErrorDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrPageErrorDto {
    pub page_number: u32,
    pub message: String,
}

impl From<JobProgress> for PersistentJobProgressDto {
    fn from(progress: JobProgress) -> Self {
        let status = match progress.status {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Cancelling => "cancelling",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        };
        Self {
            job_id: progress.job_id,
            status: status.to_string(),
            total_pages: progress.page_count,
            completed_pages: progress.completed_pages,
            failed_pages: progress.failed_pages,
            cancelled_pages: progress.cancelled_pages,
        }
    }
}

/// Progress payload emitted on the `mpdf://processing-progress` event.
/// Deliberately coarse — see `docs/desktop.md` on event granularity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingProgressDto {
    pub job_id: String,
    /// A stable string, not a raw Rust enum name: `"queued"`,
    /// `"rendering"`, `"binarizing"`, `"encoding"`, `"writing"`,
    /// `"validating"`.
    pub stage: String,
    pub page_number: Option<u32>,
    pub page_count: u32,
    /// `0.0..=1.0`, estimated from pages completed plus the current
    /// page's stage; not a precise measurement.
    pub fraction: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageExtremeDto {
    pub page_number: u32,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateComparisonDto {
    pub estimated_output_bytes: u64,
    pub actual_output_bytes: u64,
    pub absolute_error_bytes: u64,
    pub relative_error_fraction: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingCompletedDto {
    pub job_id: String,
    pub output_path: String,
    pub pages_processed: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub elapsed_us: u64,
    pub pdfium_library: String,

    // Primary-summary metrics (docs/desktop.md, "GUI completion improvements").
    pub absolute_bytes_saved: u64,
    pub size_reduction_fraction: Option<f64>,
    pub input_to_output_ratio: Option<f64>,

    // Advanced/expandable-details metrics.
    pub median_processing_duration_us: f64,
    pub overall_black_pixel_ratio: f64,
    pub slowest_page: Option<PageExtremeDto>,
    pub largest_encoded_page: Option<PageExtremeDto>,
    pub smallest_encoded_page: Option<PageExtremeDto>,

    /// `null` unless this conversion's settings matched a prior estimate
    /// for the same document — see `state::CachedEstimate`. Always
    /// present as an explicit JSON `null` rather than an omitted key
    /// (no `skip_serializing_if`), so the frontend's `T | null` field
    /// types match what actually crosses the wire in every case.
    pub estimate_comparison: Option<EstimateComparisonDto>,
}

impl ProcessingCompletedDto {
    pub fn from_report(job_id: String, output_path: String, report: &ProcessingReport) -> Self {
        Self {
            job_id,
            output_path,
            pages_processed: report.pages_processed,
            original_bytes: report.original_bytes,
            output_bytes: report.output_bytes,
            elapsed_us: report.elapsed_us,
            pdfium_library: report.pdfium_library.clone(),
            absolute_bytes_saved: report.absolute_bytes_saved,
            size_reduction_fraction: report.size_reduction_fraction,
            input_to_output_ratio: report.input_to_output_ratio,
            median_processing_duration_us: report.median_processing_duration_us,
            overall_black_pixel_ratio: report.overall_black_pixel_ratio,
            slowest_page: report.slowest_page.map(|p| PageExtremeDto {
                page_number: p.page_number,
                value: p.value,
            }),
            largest_encoded_page: report.largest_encoded_page.map(|p| PageExtremeDto {
                page_number: p.page_number,
                value: p.value,
            }),
            smallest_encoded_page: report.smallest_encoded_page.map(|p| PageExtremeDto {
                page_number: p.page_number,
                value: p.value,
            }),
            estimate_comparison: report.estimate_comparison.map(|c| EstimateComparisonDto {
                estimated_output_bytes: c.estimated_output_bytes,
                actual_output_bytes: c.actual_output_bytes,
                absolute_error_bytes: c.absolute_error_bytes,
                relative_error_fraction: c.relative_error_fraction,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingCancelledDto {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingFailedDto {
    pub job_id: String,
    pub error: UiErrorDto,
}

/// A structured, stable-coded error. The frontend must switch on `code`,
/// never parse `message` to decide what happened — `message` is for
/// display only and may change wording between versions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiErrorDto {
    pub code: String,
    pub message: String,
    /// A short, actionable next step, when one exists.
    pub hint: Option<String>,
    /// Additional technical detail (e.g. every PDFium path that was
    /// tried), meant for an expandable "technical detail" area, never the
    /// primary error text.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfiumStatusDto {
    pub resolved: bool,
    pub description: Option<String>,
    pub error: Option<UiErrorDto>,
}

/// A request to estimate the converted output's size — see
/// `docs/size-estimation.md`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateRequestDto {
    pub document_id: String,
    pub settings: ProcessingSettingsDto,
    /// How many pages to sample. Validated and clamped server-side by
    /// `mpdf_core::estimation::resolve_sample_count` —
    /// exactly the same bounds the CLI's `estimate --samples` enforces.
    pub samples: u32,
    /// A generation id the frontend assigns and increments per request,
    /// echoed back so a response to a superseded request can be
    /// discarded — the same pattern `PreviewRequestDto::request_id` uses.
    pub request_id: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSizeEstimateSampleDto {
    pub page_number: u32,
    pub raster_width: u32,
    pub raster_height: u32,
    pub black_pixel_ratio: f64,
    pub ccitt_bytes: u64,
    pub bytes_per_pixel: f64,
}

impl From<&PageSizeEstimateSample> for PageSizeEstimateSampleDto {
    fn from(sample: &PageSizeEstimateSample) -> Self {
        Self {
            page_number: sample.page_number,
            raster_width: sample.raster_width,
            raster_height: sample.raster_height,
            black_pixel_ratio: sample.black_pixel_ratio,
            ccitt_bytes: sample.ccitt_bytes,
            bytes_per_pixel: sample.bytes_per_pixel,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateResultDto {
    pub request_id: u64,
    pub document_page_count: u32,
    pub sampled_pages: Vec<PageSizeEstimateSampleDto>,
    pub estimated_output_bytes: u64,
    pub estimated_lower_bytes: u64,
    pub estimated_upper_bytes: u64,
    /// `"quartiles"` or `"min_max"` — see `docs/size-estimation.md` for
    /// why the range is never called a confidence interval.
    pub range_method: String,
    pub dpi: u16,
    pub method: String,
    pub estimate_total_duration_us: u64,
    /// Always `true`. Carried in the data itself, not only implied by
    /// which command produced it.
    pub experimental: bool,
}

impl EstimateResultDto {
    pub fn build(request_id: u64, report: &SizeEstimateReport) -> Self {
        Self {
            request_id,
            document_page_count: report.document_page_count,
            sampled_pages: report.sampled_pages.iter().map(Into::into).collect(),
            estimated_output_bytes: report.estimated_output_bytes,
            estimated_lower_bytes: report.estimated_lower_bytes,
            estimated_upper_bytes: report.estimated_upper_bytes,
            range_method: match report.range_method {
                mpdf_core::estimation::EstimationRangeMethod::Quartiles => "quartiles".to_string(),
                mpdf_core::estimation::EstimationRangeMethod::MinMax => "min_max".to_string(),
            },
            dpi: report.dpi,
            method: report.method.clone(),
            estimate_total_duration_us: report.estimate_total_duration_us,
            experimental: report.experimental,
        }
    }
}

// --- Automatic bookmarks (v2) ----------------------------------------
//
// Project-owned shapes only: no `mpdf_core` bookmark type crosses the IPC
// boundary, no local source path is ever returned to the frontend, and no
// OCR text, credential, or provider response appears in any field here.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBookmarkRequestDto {
    /// The document this request belongs to. A response from a superseded
    /// window is rejected rather than applied to whatever is open now.
    pub document_id: String,
    /// MDP package directory chosen by the user.
    pub package_path: String,
    /// Destination for the outlined PDF. The source is never modified.
    pub output_path: String,
    /// Authorize replacing an existing regular output file.
    pub overwrite: bool,
    /// Authorize replacing existing candidates and report. Independent of
    /// `overwrite`, and still refused when human reviews exist.
    pub regenerate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBookmarkStartedDto {
    pub job_id: String,
    pub document_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBookmarkStageDto {
    pub job_id: String,
    /// One of `analyzing_toc`, `aligning`, `writing_pdf`, `validating`.
    pub stage: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoBookmarkResultDto {
    pub job_id: String,
    pub document_id: String,
    /// `existing_outline`, `toc_aligned`, or `safe_refusal`.
    pub mode: String,
    /// `auto_confirmed`, `needs_review`, or `safe_refusal`.
    pub status: String,
    pub toc_page_count: u32,
    pub parsed_entries: u32,
    pub auto_confirmed: u32,
    pub needs_review: u32,
    pub skipped: u32,
    pub written_bookmarks: u32,
    pub safe_refusal_reason: Option<String>,
    pub report_path: String,
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBookmarkFailedDto {
    pub job_id: String,
    pub error: UiErrorDto,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkBboxDto {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkScoreDto {
    pub title_match: u32,
    pub page_mapping: u32,
    pub numbering_hierarchy: u32,
    pub body_layout: u32,
    pub ocr_quality: u32,
    pub sequence_uniqueness: u32,
    pub total: u32,
    /// The scale every component is measured against (10,000).
    pub maximum: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkAlignmentDto {
    pub toc_page_index: u32,
    pub body_page_index: Option<u32>,
    pub printed_label: Option<String>,
    pub page_residual: Option<i64>,
    pub mapping_offset: Option<i64>,
    pub runner_up_margin: u32,
    pub secondary_key_only: bool,
    pub geometry_quality: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkCandidateDto {
    pub candidate_id: String,
    pub source_title: String,
    pub effective_title: String,
    pub effective_level: u16,
    pub effective_parent_id: Option<String>,
    pub target_page_id: String,
    pub physical_page_index: u32,
    pub master_bbox: Option<BookmarkBboxDto>,
    /// `proposed`, `needs_review`, `confirmed`, `rejected`, `auto_confirmed`,
    /// or `skipped`. Automatic and human confirmation stay distinguishable.
    pub status: String,
    pub confidence: f32,
    pub score: Option<BookmarkScoreDto>,
    pub alignment: Option<BookmarkAlignmentDto>,
    pub automatic_reason: Option<String>,
    pub reason_codes: Vec<String>,
    pub evidence_count: u32,
}

impl From<&mpdf_core::bookmarks::BookmarkCandidate> for BookmarkCandidateDto {
    fn from(candidate: &mpdf_core::bookmarks::BookmarkCandidate) -> Self {
        use mpdf_core::bookmarks::BookmarkStatus;
        Self {
            candidate_id: candidate.candidate_id.clone(),
            source_title: candidate.source_title.clone(),
            effective_title: candidate.effective_title.clone(),
            effective_level: candidate.effective_level,
            effective_parent_id: candidate.effective_parent_id.clone(),
            target_page_id: candidate.target_page_id.clone(),
            physical_page_index: candidate.physical_page_index,
            master_bbox: candidate.master_bbox.map(|bbox| BookmarkBboxDto {
                x: bbox.x,
                y: bbox.y,
                width: bbox.width,
                height: bbox.height,
            }),
            status: match candidate.status {
                BookmarkStatus::Proposed => "proposed",
                BookmarkStatus::NeedsReview => "needs_review",
                BookmarkStatus::Confirmed => "confirmed",
                BookmarkStatus::Rejected => "rejected",
                BookmarkStatus::AutoConfirmed => "auto_confirmed",
                BookmarkStatus::Skipped => "skipped",
            }
            .to_owned(),
            confidence: candidate.confidence,
            score: candidate
                .confidence_breakdown
                .map(|breakdown| BookmarkScoreDto {
                    title_match: breakdown.title_match,
                    page_mapping: breakdown.page_mapping,
                    numbering_hierarchy: breakdown.numbering_hierarchy,
                    body_layout: breakdown.body_layout,
                    ocr_quality: breakdown.ocr_quality,
                    sequence_uniqueness: breakdown.sequence_uniqueness,
                    total: breakdown.total,
                    maximum: mpdf_core::bookmarks::MAX_SCORE,
                }),
            alignment: candidate
                .alignment_evidence
                .as_ref()
                .map(|evidence| BookmarkAlignmentDto {
                    toc_page_index: evidence.toc_page_index,
                    body_page_index: evidence.body_page_index,
                    printed_label: evidence.printed_label_raw.clone(),
                    page_residual: evidence.page_residual,
                    mapping_offset: evidence.mapping_offset,
                    runner_up_margin: evidence.runner_up_margin,
                    secondary_key_only: evidence.secondary_key_only,
                    geometry_quality: evidence.geometry_quality.clone(),
                }),
            automatic_reason: candidate
                .automatic_decision
                .as_ref()
                .map(|decision| decision.reason.clone()),
            reason_codes: candidate.reason_codes.clone(),
            evidence_count: candidate.evidence.len() as u32,
        }
    }
}
