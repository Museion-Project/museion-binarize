//! Serializable types crossing the Tauri IPC boundary.
//!
//! Every type here is project-owned and UI-appropriate — never a raw
//! `museion_binarize_core` type serialized directly, so a core refactor
//! cannot silently change the frontend's contract. Field names are
//! `camelCase` for TypeScript ergonomics; doc comments note units.
//!
//! No password ever appears in any type here: `open_document` accepts one
//! as a plain `String` argument (never logged, never stored past the one
//! call — see `commands::document`), and nothing in this module can carry
//! it back out.

use serde::{Deserialize, Serialize};

use museion_binarize_core::document::PdfDocumentInfo;
use museion_binarize_core::pipeline::ProcessingReport;

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
/// `museion_binarize_core::settings::ProcessingSettings` field-for-field.
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

/// Progress payload emitted on the `museion://processing-progress` event.
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
pub struct ProcessingCompletedDto {
    pub job_id: String,
    pub output_path: String,
    pub pages_processed: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub elapsed_us: u64,
    pub pdfium_library: String,
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
