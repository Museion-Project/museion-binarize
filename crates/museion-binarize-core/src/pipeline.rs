//! The end-to-end PDF conversion pipeline, plus document/page analysis.
//!
//! ```text
//! input.pdf
//!   -> PdfDocumentSession::open    (one open per operation; see
//!                                   document_session.rs)
//!   -> PDFium rasterization        (session.render_page, per page)
//!   -> image-processing core       (image_pipeline)
//!   -> packed bilevel image        (bilevel)
//!   -> CCITT Group 4               (ccitt, via pdf_writer::EncodedPage)
//!   -> rebuilt 1-bit PDF           (pdf_writer)           [process only]
//!   -> temporary file, validated, then atomically persisted [process only]
//! ```
//!
//! `process_pdf` and `analyze_pdf` open exactly one
//! [`crate::document_session::PdfDocumentSession`] each and hand it to a
//! generic `*_with_session` implementation that depends only on the
//! [`crate::document_session::DocumentSession`] trait — never on
//! `pdfium-render` types, and never reopening the source. This is what
//! lets ordinary (non-PDFium) tests prove the single-session behaviour
//! with a mock session; see the tests in this module and
//! `tests/pdf_pipeline.rs`.
//!
//! ## Memory behaviour
//!
//! The source file's bytes are held in memory for the whole operation
//! (the open-bytes snapshot policy — see `crate::document_session`), plus
//! one uncompressed working page, algorithm buffers, and — for
//! `process_pdf` — the growing compressed output PDF assembled in memory.
//! This is **not** O(1) in either source or output size.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::analysis::{self, DocumentAnalysisReport, PageAnalysisReport};
use crate::ccitt;
use crate::document::PdfDocumentInfo;
use crate::document_session::{DocumentSession, PdfDocumentSession, PdfOpenOptions};
use crate::error::{CoreError, Result};
use crate::image_pipeline::process_rendered_page;
use crate::page_selection::PageSelection;
use crate::pdf_writer::{BilevelPdfBuilder, EncodedPage};
use crate::pdfium_backend::{self, PdfiumConfig};
use crate::progress::{ProcessingStage, ProgressEvent, ProgressReporter};
use crate::report::PathMode;
use crate::settings::{BinarizationMethod, ProcessingSettings};
use crate::timing::{duration_to_micros, timed};
use crate::validation::{self, ExpectedOutput, ValidationMode};

/// Schema identifiers for [`ProcessingReport`], exposed so the CLI can
/// wrap it in a [`crate::report::ReportEnvelope`] without hardcoding the
/// name and version in two places.
pub const PROCESS_REPORT_SCHEMA: &str = "museion-binarize-process";
pub const PROCESS_REPORT_SCHEMA_VERSION: &str = "1.0";

/// Options controlling a conversion job.
#[derive(Debug, Clone, Default)]
pub struct PdfProcessingOptions {
    /// Password for an encrypted source document. Never logged.
    pub password: Option<String>,
    /// Whether an existing destination may be replaced. Off by default.
    pub overwrite: bool,
    pub validation: ValidationMode,
    pub pdfium: PdfiumConfig,
    /// A prior [`crate::estimation::SizeEstimateReport`]'s central
    /// estimate and settings fingerprint, if the caller has one and has
    /// already confirmed it was produced for this same document and
    /// [`crate::estimation::settings_fingerprint`]. When present *and*
    /// the fingerprint matches the settings this conversion actually
    /// runs with, the resulting [`ProcessingReport::estimate_comparison`]
    /// is filled in; a mismatched fingerprint is silently treated as "no
    /// estimate available" rather than compared anyway — see
    /// `docs/size-estimation.md`.
    pub prior_estimate: Option<PriorEstimate>,
    /// How the validated output actually gets written to `output`. See
    /// [`OutputWriteStrategy`] — every existing caller (CLI, GitHub
    /// desktop build) keeps the default; only a sandboxed Mac App Store
    /// build opts into the other variant.
    pub output_write_strategy: OutputWriteStrategy,
}

/// Two ways to commit a validated conversion result to `output`, chosen
/// by the caller because only the caller knows whether it is running
/// under App Sandbox.
///
/// # Why this exists
///
/// [`AtomicSameDirectoryRename`](Self::AtomicSameDirectoryRename) writes
/// a temp file *in `output`'s own directory* and `rename(2)`s it into
/// place — atomic on Unix, and documented in `docs/pdf-output.md`. Apple
/// documents that a security-scoped extension granted by an `NSSavePanel`
/// selection covers *exactly* the selected URL, not sibling paths in the
/// same directory ("When the save panel gives you back a URL it extends
/// your sandbox so that you can access exactly that URL" — Apple
/// Developer Forums). Creating a new sibling temp file next to a
/// sandboxed Mac App Store build's user-selected destination is
/// therefore not something the grant obviously covers, and this was
/// evaluated, not assumed — see `docs/mac-app-store-readiness.md`,
/// "Sandboxed output-save architecture."
///
/// [`DirectWriteToDestination`](Self::DirectWriteToDestination) instead
/// validates in a location the process always has unrestricted access to
/// (the system/container temp directory — under App Sandbox this
/// resolves inside the app's own container, no entitlement needed), then
/// writes the already-validated bytes straight to the exact granted
/// `output` path — never touching a second path near it. This trades
/// away the same-directory-rename's crash-window guarantee (see the
/// docs above) in exchange for working within what the sandbox extension
/// actually covers; every other safety property (validate before ever
/// touching an existing destination, cancellation never touches the
/// destination, temp file always cleaned up) is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputWriteStrategy {
    /// M0–M7A behavior, unchanged. Used by the CLI and the GitHub
    /// Developer-ID desktop build — neither ever runs under App Sandbox.
    #[default]
    AtomicSameDirectoryRename,
    /// Mac App Store build only.
    DirectWriteToDestination,
}

/// The minimum a caller needs to supply for
/// [`ProcessingReport::estimate_comparison`] to be computed: the
/// estimate's central byte count and the settings fingerprint it was
/// computed for. Not the whole [`crate::estimation::SizeEstimateReport`],
/// so a caller does not have to keep the full report around just to
/// enable this comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorEstimate {
    pub estimated_output_bytes: u64,
    pub settings_fingerprint: String,
}

/// Per-page results.
///
/// `compressed_bytes` is the field name this project has always used for
/// a page's final CCITT Group 4 size (present since Milestone 2); it is
/// kept rather than renamed to `ccitt_bytes` to match `analyze`'s naming,
/// since renaming an existing `1.0`-schema field is a breaking change
/// this milestone does not need to make (see `docs/reporting.md`'s
/// compatibility policy). Every other field below is new in this
/// milestone and purely additive.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageProcessingReport {
    /// One-based page number.
    pub page_number: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub width_points: f32,
    pub height_points: f32,
    pub compressed_bytes: u64,
    pub pixel_count: u64,
    pub black_pixel_ratio: f64,
    /// `compressed_bytes as f64 / pixel_count as f64`.
    pub bytes_per_pixel: f64,
    pub render_duration_us: u64,
    /// Grayscale conversion, contrast, preprocessing, binarization, and
    /// cleanup combined — see [`crate::timing::StageDurations`] for why
    /// these are not broken out further.
    pub processing_duration_us: u64,
    pub encoding_duration_us: u64,
    /// Sum of `render_duration_us + processing_duration_us +
    /// encoding_duration_us` for this page. Does not include time spent
    /// assembling or persisting the whole output PDF, which is not
    /// attributable to any one page.
    pub total_page_duration_us: u64,
    /// Simple, deterministic, document-relative outlier flags — see
    /// [`classify_page_outliers`]. Never implies a quality judgement: a
    /// page flagged here may simply have more ink or more content than
    /// most of the document, not an error. See `docs/reporting.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A page identified by an aggregate extreme (slowest, largest, or
/// smallest), alongside the value that made it so.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageExtreme {
    pub page_number: u32,
    pub value: u64,
}

/// The result of a completed conversion.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ProcessingReport {
    pub pages_processed: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    /// Wall-clock duration of the whole conversion, in microseconds. See
    /// `crate::timing` for why this crate reports durations this way
    /// instead of floating-point seconds.
    pub elapsed_us: u64,
    pub page_reports: Vec<PageProcessingReport>,
    /// Human-readable description of the PDFium library that was used.
    pub pdfium_library: String,

    // --- Aggregate metrics, added this milestone (all additive) --------
    /// `original_bytes.saturating_sub(output_bytes)` — `0` rather than a
    /// negative number when the output is larger than the input (an
    /// image-heavy or already-dense source can legitimately not shrink).
    pub absolute_bytes_saved: u64,
    /// `1.0 - output_bytes / original_bytes`. `0.871` means the output is
    /// 87.1% smaller. `None` when `original_bytes` is `0`.
    pub size_reduction_fraction: Option<f64>,
    /// `original_bytes / output_bytes`. `None` when `output_bytes` is `0`.
    /// Do not confuse with `size_reduction_fraction`: this crate always
    /// uses these two specific names for these two specific, documented
    /// quantities — see `docs/reporting.md`.
    pub input_to_output_ratio: Option<f64>,
    pub total_pixel_count: u64,
    pub total_black_pixels: u64,
    /// `total_black_pixels / total_pixel_count`. `0.0` when
    /// `total_pixel_count` is `0` (never expected in practice, since a
    /// document always has at least one page).
    pub overall_black_pixel_ratio: f64,
    /// Sum of every page's `compressed_bytes`. Distinct from
    /// `output_bytes`: the completed PDF also has structural/container
    /// overhead (dictionaries, cross-reference table, metadata), so this
    /// is normally somewhat less than `output_bytes`.
    pub total_ccitt_bytes: u64,
    pub mean_ccitt_bytes_per_page: f64,
    pub median_ccitt_bytes_per_page: f64,
    pub min_ccitt_bytes_per_page: u64,
    pub max_ccitt_bytes_per_page: u64,
    pub mean_processing_duration_us: f64,
    pub median_processing_duration_us: f64,
    pub slowest_page: Option<PageExtreme>,
    pub largest_encoded_page: Option<PageExtreme>,
    pub smallest_encoded_page: Option<PageExtreme>,
    /// Present only when a matching prior [`crate::estimation::SizeEstimateReport`]
    /// was supplied to [`process_pdf`]/[`process_with_open_session`] via
    /// [`PdfProcessingOptions::prior_estimate`] — "matching" meaning the
    /// caller already confirmed the same document and
    /// [`crate::estimation::settings_fingerprint`]; this module does not
    /// re-derive that match itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_comparison: Option<crate::estimation::EstimateComparison>,
}

impl ProcessingReport {
    /// Size reduction as a percentage of the original, or `None` when the
    /// original size is unknown (zero).
    pub fn reduction_percent(&self) -> Option<f64> {
        self.size_reduction_fraction.map(|f| f * 100.0)
    }

    /// Mean output bytes per page.
    pub fn average_bytes_per_page(&self) -> Option<f64> {
        if self.pages_processed == 0 {
            return None;
        }
        Some(self.output_bytes as f64 / f64::from(self.pages_processed))
    }
}

/// Simple, deterministic, document-relative outlier classification for
/// one page. Thresholds are fixed, documented constants, not statistical
/// inference — see `docs/reporting.md`. Never returns a judgement about
/// quality: a page with more ink or a larger encoded size is not
/// necessarily a worse scan.
fn classify_page_outliers(
    page: &PageProcessingReport,
    median_ccitt_bytes: f64,
    median_duration_us: f64,
    median_black_ratio: f64,
) -> Vec<String> {
    /// A page's encoded size or processing time must be at least this
    /// many times the document median to be flagged.
    const OUTLIER_RATIO: f64 = 2.0;
    /// A page's black-pixel ratio must differ from the document median
    /// by at least this many absolute percentage points to be flagged.
    const INK_RATIO_DELTA: f64 = 0.15;

    let mut warnings = Vec::new();
    if median_ccitt_bytes > 0.0 && page.compressed_bytes as f64 > OUTLIER_RATIO * median_ccitt_bytes
    {
        warnings.push("large_output".to_string());
    }
    if median_duration_us > 0.0
        && page.total_page_duration_us as f64 > OUTLIER_RATIO * median_duration_us
    {
        warnings.push("slow_processing".to_string());
    }
    if page.black_pixel_ratio > median_black_ratio + INK_RATIO_DELTA {
        warnings.push("high_ink_ratio".to_string());
    }
    if page.black_pixel_ratio < median_black_ratio - INK_RATIO_DELTA {
        warnings.push("low_ink_ratio".to_string());
    }
    warnings
}

/// Opens `input` and reports its structure without processing anything.
/// Opens exactly one document session.
pub fn inspect_pdf(input: &Path, options: &PdfOpenOptions) -> Result<PdfDocumentInfo> {
    let session = PdfDocumentSession::open(input, options)?;
    Ok(session.info().clone())
}

/// Describes which PDFium library a given configuration would use, without
/// opening any document.
pub fn describe_pdfium_library(config: &PdfiumConfig) -> Result<String> {
    let resolved = pdfium_backend::resolve_library(config)?;
    Ok(pdfium_backend::describe_resolved(&resolved))
}

/// Renders and processes a single page, returning the processed bilevel
/// image as a grayscale preview image. Used by the CLI `preview` command.
/// Opens exactly one document session for its one page.
pub fn preview_page(
    input: &Path,
    page_number: u32,
    settings: &ProcessingSettings,
    options: &PdfOpenOptions,
) -> Result<image::GrayImage> {
    settings.validate()?;
    if page_number == 0 {
        return Err(CoreError::InvalidParameter(
            "page numbers are one-based; page 0 does not exist".to_string(),
        ));
    }
    let session = PdfDocumentSession::open(input, options)?;
    let index = page_number - 1;
    if index >= session.info().page_count {
        return Err(CoreError::InvalidParameter(format!(
            "page {page_number} is out of range; the document has {} pages",
            session.info().page_count
        )));
    }

    let rendered = session.render_page(index, settings.dpi)?;
    let result = process_rendered_page(&rendered, settings)?;
    Ok(bilevel_to_gray(&result.bilevel))
}

/// Expands a packed bilevel image into an 8-bit grayscale image so it can
/// be written as an ordinary PNG preview.
pub fn bilevel_to_gray(image: &crate::bilevel::BilevelImage) -> image::GrayImage {
    image::GrayImage::from_fn(image.width, image.height, |x, y| {
        if image.get_pixel(x, y) {
            image::Luma([0])
        } else {
            image::Luma([255])
        }
    })
}

/// Converts `input` into a bilevel CCITT Group 4 PDF at `output`.
///
/// The destination is only touched once a complete, validated document
/// exists: everything is written to a temporary file in the destination
/// directory first, then atomically renamed into place.
///
/// Opens exactly one [`PdfDocumentSession`] for the whole conversion —
/// every page is rendered from that single session, never by reopening
/// `input`.
pub fn process_pdf(
    input: &Path,
    output: &Path,
    settings: &ProcessingSettings,
    options: &PdfProcessingOptions,
    progress: &dyn ProgressReporter,
) -> Result<ProcessingReport> {
    let started = Instant::now();
    settings.validate()?;
    check_destination(input, output, options.overwrite)?;

    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    let open_options = PdfOpenOptions {
        password: options.password.clone(),
        pdfium: options.pdfium.clone(),
        compute_source_hash: false,
    };
    let session = PdfDocumentSession::open(input, &open_options)?;
    process_with_session(
        &session,
        output,
        settings,
        options,
        progress,
        started,
        |path, expected, mode, pdfium| {
            // The real output validator: opens the *completed output* as its
            // own, separate document session (a different file, a different
            // operation — not a second open of the source; see
            // `crate::document_session`).
            validation::validate_output(path, expected, mode, pdfium)
        },
    )
}

/// Converts through an **already-open** [`DocumentSession`] instead of
/// opening a new one from a path.
///
/// Exists for a front end (the desktop app) that keeps one document
/// session open for the lifetime of a document — for preview and
/// thumbnail rendering — and would otherwise have to re-read and re-parse
/// the whole source file a second time just to start a conversion job.
/// Reuses the exact same per-page loop as [`process_pdf`] and, unlike the
/// test-only `process_with_session` this wraps, always uses the real
/// output validator: a caller cannot substitute a fake one through this
/// entry point.
///
/// The destination is checked against the session's own captured source
/// identity (see [`crate::source_identity::SourceIdentity`]), so the
/// input/output aliasing guarantee is unchanged from [`process_pdf`].
pub fn process_with_open_session(
    session: &impl DocumentSession,
    output: &Path,
    settings: &ProcessingSettings,
    options: &PdfProcessingOptions,
    progress: &dyn ProgressReporter,
) -> Result<ProcessingReport> {
    let started = Instant::now();
    settings.validate()?;
    check_destination(
        &session.source_identity().canonical_path,
        output,
        options.overwrite,
    )?;

    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    process_with_session(
        session,
        output,
        settings,
        options,
        progress,
        started,
        validation::validate_output,
    )
}

/// The session- and validator-generic implementation of [`process_pdf`].
/// Depends only on [`DocumentSession`] for rendering, so tests can
/// substitute a mock session to prove this function renders every page
/// from the one session it was given — see the tests below. The output
/// validator is also injected, purely so those tests are not forced to
/// provision a real PDFium library just to reopen a synthetic output PDF
/// that [`process_pdf`] itself always validates for real.
fn process_with_session(
    session: &impl DocumentSession,
    output: &Path,
    settings: &ProcessingSettings,
    options: &PdfProcessingOptions,
    progress: &dyn ProgressReporter,
    started: Instant,
    validate_output: impl FnOnce(&Path, &ExpectedOutput, ValidationMode, &PdfiumConfig) -> Result<()>,
) -> Result<ProcessingReport> {
    let info = session.info().clone();
    let pdfium_library = describe_session_library(session);

    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    progress.report(ProgressEvent::Started {
        total_pages: info.page_count,
    });

    let mut builder = BilevelPdfBuilder::new();
    let mut page_reports = Vec::with_capacity(info.page_count as usize);
    let mut expected_dimensions = Vec::with_capacity(info.page_count as usize);

    for page_info in &info.pages {
        let page_number = page_info.page_number();

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }
        progress.report(ProgressEvent::PageStarted { page: page_number });

        // `geometry` already holds the visible, post-rotation rectangle
        // (see `PageGeometry`), and PDFium renders the page in that same
        // visible orientation. The rebuilt page therefore uses these
        // dimensions verbatim and is written upright, with no /Rotate.
        let width_points = page_info.geometry.width_points;
        let height_points = page_info.geometry.height_points;

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Rendering,
        });
        // Served from the single already-open session: no reopen.
        let (rendered, render_duration_us) =
            timed(|| session.render_page(page_info.index, settings.dpi));
        let rendered = rendered?;

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Binarization,
        });
        let result = process_rendered_page(&rendered, settings)?;
        // The rendered RGB page is the largest buffer in flight; release
        // it as soon as processing has produced the bilevel image.
        drop(rendered);
        let bilevel = result.bilevel;
        let black_pixel_ratio = result.ink_stats.black_pixel_ratio;
        let processing_duration_us = result.stage_durations.grayscale_prep_us
            + result.stage_durations.binarization_us
            + result.stage_durations.cleanup_us;

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Encoding,
        });
        let (pixel_width, pixel_height) = (bilevel.width, bilevel.height);
        let pixel_count = u64::from(pixel_width) * u64::from(pixel_height);
        let (encoded, encoding_duration_us) =
            timed(|| EncodedPage::encode(&bilevel, width_points, height_points));
        let encoded = encoded?;
        drop(bilevel);

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        let compressed_bytes = encoded.compressed_bytes();
        let bytes_per_pixel = if pixel_count == 0 {
            0.0
        } else {
            compressed_bytes as f64 / pixel_count as f64
        };
        let total_page_duration_us =
            render_duration_us + processing_duration_us + encoding_duration_us;
        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Writing,
        });
        builder.add_page(&encoded)?;
        drop(encoded);

        page_reports.push(PageProcessingReport {
            page_number,
            pixel_width,
            pixel_height,
            width_points,
            height_points,
            compressed_bytes,
            pixel_count,
            black_pixel_ratio,
            bytes_per_pixel,
            render_duration_us,
            processing_duration_us,
            encoding_duration_us,
            total_page_duration_us,
            warnings: Vec::new(), // filled in once document-wide medians are known, below
        });
        expected_dimensions.push((width_points, height_points));
        progress.report(ProgressEvent::PageFinished {
            page: page_number,
            compressed_bytes,
        });
    }

    let bytes = builder.finish(&info.metadata)?;

    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    validation::assert_bilevel_ccitt_structure(&bytes)?;

    let temp = write_temporary(output, &bytes, options.output_write_strategy)?;

    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        // `temp` is dropped here, deleting the incomplete output.
        return Err(CoreError::Cancelled);
    }

    progress.report(ProgressEvent::Validating);
    let expected = ExpectedOutput {
        page_count: info.page_count,
        page_dimensions: expected_dimensions,
    };
    validate_output(temp.path(), &expected, options.validation, &options.pdfium)?;

    let output_bytes = bytes.len() as u64;
    persist(
        temp,
        output,
        &bytes,
        options.overwrite,
        options.output_write_strategy,
    )?;

    progress.report(ProgressEvent::Finished);

    let original_bytes = info.source_bytes;
    let absolute_bytes_saved = original_bytes.saturating_sub(output_bytes);
    let size_reduction_fraction =
        (original_bytes > 0).then(|| 1.0 - output_bytes as f64 / original_bytes as f64);
    let input_to_output_ratio =
        (output_bytes > 0).then(|| original_bytes as f64 / output_bytes as f64);

    let total_pixel_count: u64 = page_reports.iter().map(|p| p.pixel_count).sum();
    let total_black_pixels: u64 = page_reports
        .iter()
        .map(|p| (p.black_pixel_ratio * p.pixel_count as f64).round() as u64)
        .sum();
    let overall_black_pixel_ratio = if total_pixel_count == 0 {
        0.0
    } else {
        total_black_pixels as f64 / total_pixel_count as f64
    };

    let total_ccitt_bytes: u64 = page_reports.iter().map(|p| p.compressed_bytes).sum();
    let ccitt_values: Vec<f64> = page_reports
        .iter()
        .map(|p| p.compressed_bytes as f64)
        .collect();
    let duration_values: Vec<f64> = page_reports
        .iter()
        .map(|p| p.total_page_duration_us as f64)
        .collect();
    let black_ratio_values: Vec<f64> = page_reports.iter().map(|p| p.black_pixel_ratio).collect();

    let ccitt_quartiles = crate::estimation::quartiles(&ccitt_values);
    let duration_quartiles = crate::estimation::quartiles(&duration_values);
    let black_ratio_quartiles = crate::estimation::quartiles(&black_ratio_values);

    let mean_ccitt_bytes_per_page = if page_reports.is_empty() {
        0.0
    } else {
        total_ccitt_bytes as f64 / page_reports.len() as f64
    };
    let mean_processing_duration_us = if page_reports.is_empty() {
        0.0
    } else {
        duration_values.iter().sum::<f64>() / page_reports.len() as f64
    };

    let min_ccitt_bytes_per_page = page_reports
        .iter()
        .map(|p| p.compressed_bytes)
        .min()
        .unwrap_or(0);
    let max_ccitt_bytes_per_page = page_reports
        .iter()
        .map(|p| p.compressed_bytes)
        .max()
        .unwrap_or(0);

    let slowest_page = page_reports
        .iter()
        .max_by_key(|p| p.total_page_duration_us)
        .map(|p| PageExtreme {
            page_number: p.page_number,
            value: p.total_page_duration_us,
        });
    let largest_encoded_page = page_reports
        .iter()
        .max_by_key(|p| p.compressed_bytes)
        .map(|p| PageExtreme {
            page_number: p.page_number,
            value: p.compressed_bytes,
        });
    let smallest_encoded_page = page_reports
        .iter()
        .min_by_key(|p| p.compressed_bytes)
        .map(|p| PageExtreme {
            page_number: p.page_number,
            value: p.compressed_bytes,
        });

    // Outlier warnings need the document-wide medians computed above, so
    // this is a second, cheap pass over already-collected data — not a
    // second render or re-processing of any page.
    if let (Some(ccitt_q), Some(duration_q), Some(black_ratio_q)) =
        (ccitt_quartiles, duration_quartiles, black_ratio_quartiles)
    {
        for page in &mut page_reports {
            page.warnings =
                classify_page_outliers(page, ccitt_q.p50, duration_q.p50, black_ratio_q.p50);
        }
    }

    let estimate_comparison = options.prior_estimate.as_ref().and_then(|prior| {
        (prior.settings_fingerprint == crate::estimation::settings_fingerprint(settings)).then(
            || crate::estimation::compare_estimate(prior.estimated_output_bytes, output_bytes),
        )
    });

    Ok(ProcessingReport {
        pages_processed: info.page_count,
        original_bytes,
        output_bytes,
        elapsed_us: duration_to_micros(started.elapsed()),
        page_reports,
        pdfium_library,
        absolute_bytes_saved,
        size_reduction_fraction,
        input_to_output_ratio,
        total_pixel_count,
        total_black_pixels,
        overall_black_pixel_ratio,
        total_ccitt_bytes,
        mean_ccitt_bytes_per_page,
        median_ccitt_bytes_per_page: ccitt_quartiles.map(|q| q.p50).unwrap_or(0.0),
        min_ccitt_bytes_per_page,
        max_ccitt_bytes_per_page,
        mean_processing_duration_us,
        median_processing_duration_us: duration_quartiles.map(|q| q.p50).unwrap_or(0.0),
        slowest_page,
        largest_encoded_page,
        smallest_encoded_page,
        estimate_comparison,
    })
}

/// Options controlling an `analyze` run.
#[derive(Debug, Clone, Default)]
pub struct AnalysisOptions {
    pub password: Option<String>,
    pub pdfium: PdfiumConfig,
    /// Pages to analyze, in [`PageSelection`] syntax (e.g. `"1,3-5"` or
    /// `"all"`). `None` means every page. Parsed once `analyze_pdf` knows
    /// the real page count, so an out-of-range page is reported against
    /// the document actually opened.
    pub pages: Option<String>,
    /// Whether to also run CCITT Group 4 encoding, purely to report its
    /// size — the result is discarded, never written anywhere. Off by
    /// default because it is extra work `analyze`'s core purpose
    /// (choosing settings, comparing methods) does not require.
    pub encode: bool,
    pub path_mode: PathMode,
}

/// Inspects and processes `input` through the real pipeline — rendering
/// and binarizing every selected page — without writing a reconstructed
/// output PDF. See `crate::analysis` for the report shape and what it is
/// not (a benchmark or a quality claim).
///
/// Opens exactly one [`PdfDocumentSession`] for the whole analysis.
pub fn analyze_pdf(
    input: &Path,
    settings: &ProcessingSettings,
    options: &AnalysisOptions,
    progress: &dyn ProgressReporter,
) -> Result<DocumentAnalysisReport> {
    settings.validate()?;
    let open_options = PdfOpenOptions {
        password: options.password.clone(),
        pdfium: options.pdfium.clone(),
        compute_source_hash: false,
    };
    let session = PdfDocumentSession::open(input, &open_options)?;
    let mut report = analyze_with_session(&session, settings, options, progress)?;
    report.source_path = crate::report::display_path(input, options.path_mode);
    Ok(report)
}

/// The session-generic implementation of [`analyze_pdf`]. See
/// [`process_with_session`] for why this split exists.
fn analyze_with_session(
    session: &impl DocumentSession,
    settings: &ProcessingSettings,
    options: &AnalysisOptions,
    progress: &dyn ProgressReporter,
) -> Result<DocumentAnalysisReport> {
    let started = Instant::now();
    let info = session.info().clone();
    let pdfium_library = describe_session_library(session);

    let selection = match &options.pages {
        Some(raw) => PageSelection::parse(raw, info.page_count)?,
        None => PageSelection::all(info.page_count),
    };

    progress.report(ProgressEvent::Started {
        total_pages: selection.len() as u32,
    });

    let mut pages = Vec::with_capacity(selection.len());
    let mut failed_page_count = 0u32;
    let mut total_visible_area_points2 = 0.0f64;

    for &index in selection.indices() {
        let page_info = &info.pages[index as usize];
        let page_number = page_info.page_number();

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }
        progress.report(ProgressEvent::PageStarted { page: page_number });

        match analyze_one_page(session, page_info, settings, options.encode) {
            Ok(page_report) => {
                total_visible_area_points2 +=
                    f64::from(page_report.width_points) * f64::from(page_report.height_points);
                pages.push(page_report);
            }
            Err(_) => {
                // `analyze` is a diagnostic tool: one unreadable page must
                // not prevent reporting on the rest of a long book. The
                // page is simply absent from `pages`, and counted here.
                failed_page_count += 1;
            }
        }

        progress.report(ProgressEvent::PageFinished {
            page: page_number,
            compressed_bytes: 0,
        });
    }

    progress.report(ProgressEvent::Finished);

    let page_duration = analysis::aggregate_page_durations(&pages);
    let analyzed_page_count = pages.len() as u32;

    Ok(DocumentAnalysisReport {
        source_path: None, // filled in by analyze_pdf, which knows the real path
        source_bytes: info.source_bytes,
        page_count: info.page_count,
        analyzed_page_count,
        failed_page_count,
        total_visible_area_points2,
        dpi: settings.dpi,
        method: method_name(settings.method),
        total_duration_us: duration_to_micros(started.elapsed()),
        page_duration,
        pdfium_library,
        pages,
    })
}

/// Renders, processes, and (if `encode`) CCITT-encodes one page, purely
/// for measurement — this is the one place that work happens, shared by
/// `analyze` and by [`estimate_output_size`], so an estimate is never
/// produced by a second, subtly different page-processing path. Neither
/// caller writes an output PDF from this function's result.
fn analyze_one_page(
    session: &impl DocumentSession,
    page_info: &crate::document::PdfPageInfo,
    settings: &ProcessingSettings,
    encode: bool,
) -> Result<PageAnalysisReport> {
    let (rendered, render_us) = timed(|| session.render_page(page_info.index, settings.dpi));
    let rendered = rendered?;
    let result = process_rendered_page(&rendered, settings)?;
    drop(rendered);

    let raw_raster_bytes_estimate =
        u64::from(result.bilevel.width) * u64::from(result.bilevel.height) * 3;
    let packed_bilevel_bytes = (result.bilevel.stride * result.bilevel.height as usize) as u64;

    let (ccitt_bytes, ccitt_encode_us) = if encode {
        let (bytes, us) = timed(|| ccitt::encode_g4(&result.bilevel));
        (Some(bytes.len() as u64), Some(us))
    } else {
        (None, None)
    };
    let ccitt_bytes_per_pixel = ccitt_bytes.map(|bytes| {
        let pixel_count = f64::from(result.bilevel.width) * f64::from(result.bilevel.height);
        bytes as f64 / pixel_count
    });

    let mut stage_durations = result.stage_durations;
    stage_durations.render_us = render_us;
    stage_durations.ccitt_encode_us = ccitt_encode_us;
    stage_durations.total_us = render_us
        + stage_durations.grayscale_prep_us
        + stage_durations.binarization_us
        + stage_durations.cleanup_us
        + ccitt_encode_us.unwrap_or(0);

    Ok(PageAnalysisReport {
        page_index: page_info.index,
        page_number: page_info.page_number(),
        width_points: page_info.geometry.width_points,
        height_points: page_info.geometry.height_points,
        source_rotation_degrees: page_info.source_rotation.degrees(),
        pixel_width: result.bilevel.width,
        pixel_height: result.bilevel.height,
        pixel_count: u64::from(result.bilevel.width) * u64::from(result.bilevel.height),
        grayscale: result.grayscale_stats,
        threshold: result.threshold,
        ink: result.ink_stats,
        raw_raster_bytes_estimate,
        packed_bilevel_bytes,
        ccitt_bytes,
        ccitt_bytes_per_pixel,
        stage_durations,
        warnings: Vec::new(),
    })
}

/// Options controlling an `estimate` run.
#[derive(Debug, Clone)]
pub struct EstimationOptions {
    pub password: Option<String>,
    pub pdfium: PdfiumConfig,
    /// How many pages to sample. Validated and clamped to the document's
    /// actual page count by [`crate::estimation::resolve_sample_count`].
    pub samples: u32,
}

impl Default for EstimationOptions {
    fn default() -> Self {
        Self {
            password: None,
            pdfium: PdfiumConfig::default(),
            samples: crate::estimation::DEFAULT_SAMPLE_COUNT,
        }
    }
}

/// Estimates the converted output's size by rendering, processing, and
/// CCITT-encoding a small, deterministic sample of pages — the exact same
/// per-page work `process`/`analyze --encode` do, through
/// [`analyze_one_page`] — and extrapolating from each sampled page's
/// measured bytes-per-pixel to every page in the document. Never
/// constructs or validates an output PDF: that is the whole reason
/// estimation is cheaper than a full conversion. See
/// `docs/size-estimation.md` for the full methodology.
///
/// Opens exactly one [`PdfDocumentSession`] for the whole estimate.
pub fn estimate_output_size(
    input: &Path,
    settings: &ProcessingSettings,
    options: &EstimationOptions,
    progress: &dyn ProgressReporter,
) -> Result<crate::estimation::SizeEstimateReport> {
    settings.validate()?;
    let open_options = PdfOpenOptions {
        password: options.password.clone(),
        pdfium: options.pdfium.clone(),
        compute_source_hash: false,
    };
    let session = PdfDocumentSession::open(input, &open_options)?;
    estimate_with_session(&session, settings, options, progress)
}

/// Estimates through an **already-open** [`DocumentSession`], for exactly
/// the same reason [`process_with_open_session`] exists: a front end that
/// keeps one session open for the document's lifetime should not have to
/// re-read and re-parse the source a second time just to estimate.
pub fn estimate_with_open_session(
    session: &impl DocumentSession,
    settings: &ProcessingSettings,
    options: &EstimationOptions,
    progress: &dyn ProgressReporter,
) -> Result<crate::estimation::SizeEstimateReport> {
    settings.validate()?;
    estimate_with_session(session, settings, options, progress)
}

/// The session-generic implementation shared by [`estimate_output_size`]
/// and [`estimate_with_open_session`].
fn estimate_with_session(
    session: &impl DocumentSession,
    settings: &ProcessingSettings,
    options: &EstimationOptions,
    progress: &dyn ProgressReporter,
) -> Result<crate::estimation::SizeEstimateReport> {
    use crate::estimation::{
        self, EstimationRangeMethod, PageSizeEstimateSample, QUARTILE_MIN_SAMPLES,
    };

    let started = Instant::now();
    let info = session.info().clone();
    let pdfium_library = describe_session_library(session);

    let sample_count = estimation::resolve_sample_count(options.samples, info.page_count)?;
    let sample_indices = estimation::select_sample_pages(info.page_count, sample_count);

    progress.report(ProgressEvent::Started {
        total_pages: sample_indices.len() as u32,
    });

    let mut samples = Vec::with_capacity(sample_indices.len());
    let mut sample_durations_us = Vec::with_capacity(sample_indices.len());

    for &index in &sample_indices {
        let page_info = &info.pages[index as usize];
        let page_number = page_info.page_number();

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }
        progress.report(ProgressEvent::PageStarted { page: page_number });

        // Always encode: the estimate is meaningless without a real
        // measured CCITT byte count for the sampled page.
        let page_report = analyze_one_page(session, page_info, settings, true)?;
        let ccitt_bytes = page_report
            .ccitt_bytes
            .expect("analyze_one_page(.., encode = true) always sets ccitt_bytes");
        let bytes_per_pixel = page_report
            .ccitt_bytes_per_pixel
            .expect("set alongside ccitt_bytes");

        sample_durations_us.push(page_report.stage_durations.total_us);
        samples.push(PageSizeEstimateSample {
            page_number,
            page_index: index,
            width_points: page_report.width_points,
            height_points: page_report.height_points,
            raster_width: page_report.pixel_width,
            raster_height: page_report.pixel_height,
            pixel_count: page_report.pixel_count,
            black_pixel_ratio: page_report.ink.black_pixel_ratio,
            packed_bytes: page_report.packed_bilevel_bytes,
            ccitt_bytes,
            bytes_per_pixel,
            processing_duration_us: page_report.stage_durations.total_us,
        });

        progress.report(ProgressEvent::PageFinished {
            page: page_number,
            compressed_bytes: ccitt_bytes,
        });
    }

    progress.report(ProgressEvent::Finished);

    if samples.is_empty() {
        return Err(CoreError::InvalidDocument(
            "the document contains no pages to sample".to_string(),
        ));
    }

    let bytes_per_pixel_values: Vec<f64> = samples.iter().map(|s| s.bytes_per_pixel).collect();
    let quartiles = estimation::quartiles(&bytes_per_pixel_values)
        .expect("samples is non-empty, checked above");
    let min_bytes_per_pixel = bytes_per_pixel_values
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_bytes_per_pixel = bytes_per_pixel_values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let mean_bytes_per_pixel =
        bytes_per_pixel_values.iter().sum::<f64>() / bytes_per_pixel_values.len() as f64;

    let range_method = if samples.len() < QUARTILE_MIN_SAMPLES {
        EstimationRangeMethod::MinMax
    } else {
        EstimationRangeMethod::Quartiles
    };
    let (lower_bpp, upper_bpp) = match range_method {
        EstimationRangeMethod::Quartiles => (quartiles.p25, quartiles.p75),
        EstimationRangeMethod::MinMax => (min_bytes_per_pixel, max_bytes_per_pixel),
    };

    // Total pixel count across *every* document page at the selected DPI
    // — not just the sampled ones — computed from geometry alone (no
    // rendering needed for the pages that were not sampled). u128
    // accumulation: a long, large-page document could otherwise overflow
    // a 64-bit running sum of pixel products before the final cast.
    let mut total_pixels: u128 = 0;
    for page in &info.pages {
        let (width, height) = page.geometry.pixel_size(settings.dpi)?;
        total_pixels = total_pixels
            .checked_add(u128::from(width) * u128::from(height))
            .ok_or_else(|| {
                CoreError::ResourceLimitExceeded(
                    "total document pixel count overflowed while estimating output size"
                        .to_string(),
                )
            })?;
    }

    // Raw CCITT bytes are only part of a real output PDF: every page also
    // carries a page object, an image XObject dictionary, and a content
    // stream, and the document itself carries a header/catalog/trailer.
    // For pages that compress to almost nothing this structural overhead
    // dominates, so extrapolating bytes-per-pixel alone systematically
    // underestimates the real file (measured: 40-46% low on synthetic
    // fixtures before this was accounted for). `measure_container_overhead`
    // gets these numbers by building trivial reference pages through the
    // exact same writer `process` uses, not by fitting anything to a
    // corpus — see docs/size-estimation.md, "Why the estimate adds a
    // container-overhead term".
    let overhead = crate::pdf_writer::measure_container_overhead();
    let container_bytes = overhead
        .per_page_bytes
        .checked_mul(u64::from(info.page_count))
        .and_then(|per_page_total| per_page_total.checked_add(overhead.fixed_document_bytes))
        .ok_or_else(|| {
            CoreError::ResourceLimitExceeded(
                "estimated container overhead overflowed while estimating output size".to_string(),
            )
        })?;

    let extrapolate = |bytes_per_pixel: f64| -> u64 {
        let image_bytes = total_pixels as f64 * bytes_per_pixel.max(0.0);
        let image_bytes = if !image_bytes.is_finite() || image_bytes <= 0.0 {
            0
        } else {
            image_bytes.min(u64::MAX as f64) as u64
        };
        image_bytes.saturating_add(container_bytes)
    };

    // The central estimate extrapolates a *total* (sum over every page),
    // and the mean — not the median — is the mathematically appropriate
    // central-tendency statistic for that: total ~= mean x page_count is
    // exact in expectation, while the median carries no such guarantee
    // once bytes-per-pixel is skewed or multimodal across page content
    // (a real, measured effect: a mix of near-empty and heavily-inked
    // page types, evaluated against a real PDFium build, made the
    // median-based estimate underestimate the true total by 40-46% on
    // synthetic mixed-content fixtures — see
    // `estimate_accuracy_on_a_heterogeneous_synthetic_document_is_within_the_engineering_threshold`
    // in `tests/pdf_pipeline.rs` and docs/size-estimation.md, "Why mean,
    // not median, for the central estimate"). The median is still
    // reported (`median_bytes_per_pixel`) as a useful, outlier-resistant
    // data point, and the quartile/min-max spread below still describes
    // the range around the mean.
    let estimated_output_bytes = extrapolate(mean_bytes_per_pixel);
    let estimated_lower_bytes = extrapolate(lower_bpp).min(estimated_output_bytes);
    let estimated_upper_bytes = extrapolate(upper_bpp).max(estimated_output_bytes);

    let sampled_total_encoded_bytes: u64 = samples.iter().map(|s| s.ccitt_bytes).sum();

    let sample_duration_stats = estimation::quartiles(
        &sample_durations_us
            .iter()
            .map(|&us| us as f64)
            .collect::<Vec<_>>(),
    )
    .expect("samples is non-empty, checked above");
    let mean_sample_duration_us =
        sample_durations_us.iter().sum::<u64>() as f64 / sample_durations_us.len() as f64;

    Ok(crate::estimation::SizeEstimateReport {
        document_page_count: info.page_count,
        sampled_pages: samples,
        sampled_total_encoded_bytes,
        min_bytes_per_pixel,
        mean_bytes_per_pixel,
        median_bytes_per_pixel: quartiles.p50,
        max_bytes_per_pixel,
        range_method,
        estimated_output_bytes,
        estimated_lower_bytes,
        estimated_upper_bytes,
        dpi: settings.dpi,
        method: method_name(settings.method),
        settings_fingerprint: estimation::settings_fingerprint(settings),
        estimate_total_duration_us: duration_to_micros(started.elapsed()),
        mean_sample_duration_us,
        median_sample_duration_us: sample_duration_stats.p50,
        pdfium_library,
        experimental: true,
    })
}

fn method_name(method: BinarizationMethod) -> String {
    match method {
        BinarizationMethod::Otsu => "otsu",
        BinarizationMethod::Manual { .. } => "manual",
        BinarizationMethod::Sauvola(_) => "sauvola",
    }
    .to_string()
}

/// Describes the PDFium library a session bound to, for a
/// [`DocumentSession`] generically (a mock used in tests can return
/// anything descriptive; it never has to construct a real
/// `ResolvedLibrary`).
fn describe_session_library(session: &impl DocumentSession) -> String {
    // `PdfDocumentSession` exposes the real, precise description; the
    // generic bound here only needs *a* string, so this small
    // specialization point lives on the trait object's info instead of
    // requiring every mock to fabricate a `ResolvedLibrary`.
    session.pdfium_library_description()
}

/// Rejects unsafe destinations before any work begins.
fn check_destination(input: &Path, output: &Path, overwrite: bool) -> Result<()> {
    if paths_refer_to_same_file(input, output) {
        return Err(CoreError::DestinationConflict(
            "the output path is the same file as the input; the source is never overwritten"
                .to_string(),
        ));
    }
    // Checked before the generic "already exists" case: a directory is
    // never a valid destination, and suggesting `--overwrite` for one
    // would send the user down a path that cannot work.
    if output.is_dir() {
        return Err(CoreError::DestinationConflict(format!(
            "{} is a directory, not a file",
            output.display()
        )));
    }
    if output.exists() && !overwrite {
        return Err(CoreError::DestinationConflict(format!(
            "{} already exists; pass the overwrite option to replace it",
            output.display()
        )));
    }
    Ok(())
}

/// Compares two paths for filesystem identity. `output` typically does not
/// exist yet at the moment this runs (it is about to be created), so this
/// does not simply require both paths to exist: when a path cannot be
/// canonicalized directly, its parent directory is canonicalized instead
/// and the file name re-appended, which still catches two different
/// spellings (relative vs absolute, `./`) of the same not-yet-existing
/// destination. See the identical rationale on
/// `museion_binarize_cli::output::paths_refer_to_same_file`, which shares
/// this logic for the CLI's `--report` aliasing check.
fn paths_refer_to_same_file(a: &Path, b: &Path) -> bool {
    match (normalize_for_comparison(a), normalize_for_comparison(b)) {
        (Some(na), Some(nb)) => na == nb,
        _ => a == b,
    }
}

fn normalize_for_comparison(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let file_name = path.file_name()?;
    let canonical_parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => std::fs::canonicalize(parent).ok()?,
        None => std::env::current_dir().ok()?,
    };
    Some(canonical_parent.join(file_name))
}

/// Writes `bytes` to a temporary file, for later validation.
///
/// [`OutputWriteStrategy::AtomicSameDirectoryRename`] places it in
/// `output`'s own directory, so the later rename stays on one filesystem
/// and is therefore atomic. [`OutputWriteStrategy::DirectWriteToDestination`]
/// places it in the system/container temp directory instead — a location
/// this process always has unrestricted access to, sandboxed or not — since
/// that strategy never renames it into place at all (see `persist`).
fn write_temporary(
    output: &Path,
    bytes: &[u8],
    strategy: OutputWriteStrategy,
) -> Result<tempfile::NamedTempFile> {
    let directory = match strategy {
        OutputWriteStrategy::AtomicSameDirectoryRename => {
            output.parent().filter(|p| !p.as_os_str().is_empty())
        }
        OutputWriteStrategy::DirectWriteToDestination => None,
    };
    let mut temp = match directory {
        Some(dir) => tempfile::Builder::new()
            .prefix(".museion-binarize-")
            .suffix(".pdf.partial")
            .tempfile_in(dir),
        None => tempfile::Builder::new()
            .prefix(".museion-binarize-")
            .suffix(".pdf.partial")
            .tempfile(),
    }
    .map_err(|e| CoreError::TemporaryFile(format!("could not create a temporary file: {e}")))?;

    use std::io::Write;
    temp.write_all(bytes).map_err(|e| {
        CoreError::TemporaryFile(format!("could not write the temporary file: {e}"))
    })?;
    temp.flush().map_err(|e| {
        CoreError::TemporaryFile(format!("could not flush the temporary file: {e}"))
    })?;
    temp.as_file()
        .sync_all()
        .map_err(|e| CoreError::TemporaryFile(format!("could not sync the temporary file: {e}")))?;
    Ok(temp)
}

/// Commits the already-validated `bytes` to `output`, per `strategy`.
///
/// # `AtomicSameDirectoryRename` (M0–M7A, unchanged)
///
/// Moves the validated temporary file (which lives in `output`'s own
/// directory — see `write_temporary`) into place.
///
/// * **Unix (including macOS):** `rename(2)` replaces an existing
///   destination atomically. The old file is never unlinked first, so at
///   every instant the destination names either the complete old document
///   or the complete new one — never nothing.
/// * **Windows:** `MoveFileEx` without `MOVEFILE_REPLACE_EXISTING` fails
///   when the destination exists, and `std::fs::rename` does not set that
///   flag, so the old file is unlinked immediately before the rename.
///   That leaves a narrow window in which neither name exists. This is a
///   real limitation, not a theoretical one; it is recorded in
///   `docs/pdf-output.md` and no cross-platform atomicity is claimed.
///
/// The destination is only touched after the replacement has been fully
/// written, synced, and validated.
///
/// # `DirectWriteToDestination` (Mac App Store build only)
///
/// Writes `bytes` (already validated, via the container-local temp file)
/// straight to `output` — the one path a sandboxed build's Powerbox grant
/// actually covers — rather than renaming a sibling temp file into place.
/// The destination is still only touched after validation succeeds, so a
/// validation failure or cancellation still leaves an existing destination
/// completely untouched, exactly as `AtomicSameDirectoryRename` guarantees.
/// What is **not** preserved: if the process is killed or crashes *during*
/// this specific write, the destination can be left holding a partial
/// file — `rename(2)`'s all-or-nothing swap is what `AtomicSameDirectoryRename`
/// gets and this strategy does not, because achieving it would require
/// either a rename across two different granted paths (not demonstrably
/// covered by the sandbox extension a save-panel selection grants — see
/// `OutputWriteStrategy`'s own docs) or an Apple-coordinated replacement
/// API this milestone does not implement. See
/// `docs/mac-app-store-readiness.md`, "Sandboxed output-save architecture,"
/// for the full record of this trade-off and why it was made here rather
/// than left unresolved.
fn persist(
    temp: tempfile::NamedTempFile,
    output: &Path,
    bytes: &[u8],
    overwrite: bool,
    strategy: OutputWriteStrategy,
) -> Result<()> {
    match strategy {
        OutputWriteStrategy::AtomicSameDirectoryRename => {
            // Windows cannot rename onto an existing file; Unix can, and
            // doing so atomically is the whole point, so it must not be
            // unlinked there.
            #[cfg(windows)]
            if overwrite && output.exists() {
                std::fs::remove_file(output).map_err(|e| CoreError::io(output, e))?;
            }
            #[cfg(not(windows))]
            let _ = overwrite;

            let path = temp.path().to_path_buf();
            temp.persist(output).map_err(|e| {
                CoreError::TemporaryFile(format!(
                    "could not move the completed output from {} into place: {}",
                    path.display(),
                    e.error
                ))
            })?;
            Ok(())
        }
        OutputWriteStrategy::DirectWriteToDestination => {
            // `check_destination` already rejected an existing
            // destination without `overwrite` before any work began, so
            // reaching here means writing directly is intended.
            let _ = overwrite;
            write_direct(output, bytes)?;
            // `temp` (the container-local validation copy) is dropped
            // here, deleting it — it was never the file that mattered to
            // the caller.
            Ok(())
        }
    }
}

/// The `DirectWriteToDestination` half of `persist`: writes `bytes`
/// straight to the exact path a sandboxed build's Powerbox grant covers,
/// flushed and synced before returning.
fn write_direct(output: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(output).map_err(|e| CoreError::io(output, e))?;
    file.write_all(bytes)
        .map_err(|e| CoreError::io(output, e))?;
    file.sync_all().map_err(|e| CoreError::io(output, e))?;
    Ok(())
}

/// The temporary file name pattern, exposed so tests can assert that no
/// partial output is left behind.
pub const TEMP_FILE_PREFIX: &str = ".museion-binarize-";

/// Convenience: returns the paths of any leftover Museion temporary files
/// in `directory`. Used by tests to prove cleanup happened.
pub fn leftover_temporary_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(TEMP_FILE_PREFIX))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_write_over_the_input_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.pdf");
        std::fs::write(&path, b"%PDF-1.7").unwrap();
        let err = check_destination(&path, &path, true).unwrap_err();
        assert!(matches!(err, CoreError::DestinationConflict(_)));
    }

    #[test]
    fn refuses_an_existing_destination_unless_overwrite_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.pdf");
        let output = dir.path().join("out.pdf");
        std::fs::write(&input, b"in").unwrap();
        std::fs::write(&output, b"existing").unwrap();

        assert!(check_destination(&input, &output, false).is_err());
        assert!(check_destination(&input, &output, true).is_ok());
    }

    #[test]
    fn accepts_a_fresh_destination() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.pdf");
        std::fs::write(&input, b"in").unwrap();
        assert!(check_destination(&input, &dir.path().join("new.pdf"), false).is_ok());
    }

    #[test]
    fn refuses_a_directory_as_destination() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.pdf");
        std::fs::write(&input, b"in").unwrap();
        let err = check_destination(&input, dir.path(), true).unwrap_err();
        assert!(matches!(err, CoreError::DestinationConflict(_)));
    }

    /// A directory destination must be reported as a directory whether or
    /// not overwrite is set — never as "already exists; pass overwrite",
    /// which would suggest a fix that cannot work.
    #[test]
    fn a_directory_destination_is_reported_as_a_directory_regardless_of_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.pdf");
        std::fs::write(&input, b"in").unwrap();

        for overwrite in [false, true] {
            let err = check_destination(&input, dir.path(), overwrite).unwrap_err();
            let CoreError::DestinationConflict(message) = err else {
                panic!("expected a destination conflict");
            };
            assert!(
                message.contains("is a directory"),
                "overwrite={overwrite}: expected a directory error, got {message:?}"
            );
            assert!(
                !message.contains("overwrite option"),
                "overwrite={overwrite}: must not suggest overwriting a directory"
            );
        }
    }

    /// On Unix the replacement must be a single atomic rename: the old
    /// destination is never unlinked first, so the path always names a
    /// complete document.
    #[cfg(unix)]
    #[test]
    fn unix_overwrite_replaces_the_destination_without_unlinking_it_first() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        std::fs::write(&output, b"old contents").unwrap();
        let old_inode = std::fs::metadata(&output).unwrap().ino();

        let temp =
            write_temporary(&output, b"new contents", OutputWriteStrategy::default()).unwrap();
        persist(
            temp,
            &output,
            b"new contents",
            true,
            OutputWriteStrategy::default(),
        )
        .unwrap();

        assert_eq!(std::fs::read(&output).unwrap(), b"new contents");
        assert_ne!(
            std::fs::metadata(&output).unwrap().ino(),
            old_inode,
            "the destination should have been replaced by the temporary file"
        );
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    /// A failure before persistence must leave the previous destination
    /// completely intact and drop the temporary file.
    #[test]
    fn a_failure_before_persistence_leaves_the_old_destination_intact() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        std::fs::write(&output, b"original document").unwrap();

        {
            let _temp = write_temporary(
                &output,
                b"replacement that never lands",
                OutputWriteStrategy::default(),
            )
            .unwrap();
        }

        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"original document",
            "the previous output must survive a failure before persistence"
        );
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn temporary_file_is_created_beside_the_destination_and_persists_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");

        let temp =
            write_temporary(&output, b"%PDF-1.7 test", OutputWriteStrategy::default()).unwrap();
        assert_eq!(temp.path().parent().unwrap(), dir.path());
        assert!(
            !output.exists(),
            "destination must not exist before persist"
        );

        persist(
            temp,
            &output,
            b"%PDF-1.7 test",
            false,
            OutputWriteStrategy::default(),
        )
        .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"%PDF-1.7 test");
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn dropping_the_temporary_file_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        {
            let _temp =
                write_temporary(&output, b"partial", OutputWriteStrategy::default()).unwrap();
            assert_eq!(leftover_temporary_files(dir.path()).len(), 1);
        }
        assert!(
            leftover_temporary_files(dir.path()).is_empty(),
            "a dropped temporary file must be removed"
        );
        assert!(!output.exists(), "the destination must never appear");
    }

    #[test]
    fn overwrite_replaces_the_destination_contents() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        std::fs::write(&output, b"old").unwrap();

        let temp = write_temporary(&output, b"new", OutputWriteStrategy::default()).unwrap();
        persist(temp, &output, b"new", true, OutputWriteStrategy::default()).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"new");
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn direct_write_strategy_writes_bytes_straight_to_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");

        let temp = write_temporary(
            &output,
            b"%PDF-1.7 test",
            OutputWriteStrategy::DirectWriteToDestination,
        )
        .unwrap();
        // Unlike AtomicSameDirectoryRename, the temp file is NOT beside
        // the destination — it must never assume sibling-path access.
        assert_ne!(temp.path().parent().unwrap(), dir.path());
        assert!(
            !output.exists(),
            "destination must not exist before persist"
        );

        persist(
            temp,
            &output,
            b"%PDF-1.7 test",
            false,
            OutputWriteStrategy::DirectWriteToDestination,
        )
        .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"%PDF-1.7 test");
        assert!(
            leftover_temporary_files(dir.path()).is_empty(),
            "no sibling temp file may ever appear next to the destination"
        );
    }

    #[test]
    fn direct_write_strategy_overwrite_replaces_the_destination_contents() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        std::fs::write(&output, b"old").unwrap();

        let temp = write_temporary(
            &output,
            b"new",
            OutputWriteStrategy::DirectWriteToDestination,
        )
        .unwrap();
        persist(
            temp,
            &output,
            b"new",
            true,
            OutputWriteStrategy::DirectWriteToDestination,
        )
        .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"new");
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn direct_write_strategy_failure_before_persistence_leaves_the_old_destination_intact() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        std::fs::write(&output, b"original document").unwrap();

        {
            let _temp = write_temporary(
                &output,
                b"replacement that never lands",
                OutputWriteStrategy::DirectWriteToDestination,
            )
            .unwrap();
            // Simulates cancellation/validation failure: persist is never
            // called, so the destination must be untouched and the
            // container-local temp file must not leak into `dir`.
        }

        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"original document",
            "the previous output must survive a failure before persistence"
        );
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn report_computes_reduction_and_average_size() {
        let report = ProcessingReport {
            pages_processed: 4,
            original_bytes: 1000,
            output_bytes: 250,
            elapsed_us: 1_000_000,
            page_reports: Vec::new(),
            pdfium_library: "test".to_string(),
            size_reduction_fraction: Some(0.75),
            ..Default::default()
        };
        assert_eq!(report.reduction_percent(), Some(75.0));
        assert_eq!(report.average_bytes_per_page(), Some(62.5));
    }

    #[test]
    fn report_handles_degenerate_inputs_without_dividing_by_zero() {
        let report = ProcessingReport {
            pages_processed: 0,
            original_bytes: 0,
            output_bytes: 0,
            elapsed_us: 0,
            page_reports: Vec::new(),
            pdfium_library: "test".to_string(),
            ..Default::default()
        };
        assert_eq!(report.reduction_percent(), None);
        assert_eq!(report.average_bytes_per_page(), None);
    }

    #[test]
    fn report_reports_negative_reduction_when_output_is_larger() {
        let report = ProcessingReport {
            pages_processed: 1,
            original_bytes: 100,
            output_bytes: 150,
            elapsed_us: 0,
            page_reports: Vec::new(),
            pdfium_library: "test".to_string(),
            size_reduction_fraction: Some(-0.5),
            ..Default::default()
        };
        assert_eq!(report.reduction_percent(), Some(-50.0));
    }

    #[test]
    fn size_reduction_fraction_and_ratio_match_the_documented_formulas() {
        // 51.8 MB -> 6.7 MB is the observed real-document baseline from
        // the Milestone 4 native acceptance run (docs/desktop-testing.md)
        // — used here only as a plausible round-number example, not as a
        // hard-coded expectation of what any future run must produce.
        let original_bytes = 1_000u64;
        let output_bytes = 129u64; // ~87.1% reduction, ~7.75:1 ratio
        let size_reduction_fraction = 1.0 - output_bytes as f64 / original_bytes as f64;
        let input_to_output_ratio = original_bytes as f64 / output_bytes as f64;
        assert!((size_reduction_fraction - 0.871).abs() < 0.001);
        assert!((input_to_output_ratio - 7.75).abs() < 0.01);
    }

    #[test]
    fn preview_rejects_page_zero_before_touching_pdfium() {
        let settings = ProcessingSettings {
            dpi: 300,
            method: BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: Default::default(),
            cleanup: Default::default(),
        };
        let err = preview_page(
            Path::new("/nonexistent.pdf"),
            0,
            &settings,
            &PdfOpenOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidParameter(_)));
    }

    #[test]
    fn bilevel_expands_to_black_and_white_grayscale() {
        use crate::bilevel::{BilevelImage, BinaryMask};
        let mut mask = BinaryMask::new(4, 2);
        mask.set(0, 0, true);
        let gray = bilevel_to_gray(&BilevelImage::from_mask(&mask));
        assert_eq!(gray.get_pixel(0, 0)[0], 0, "set bit must render black");
        assert_eq!(gray.get_pixel(1, 0)[0], 255, "clear bit must render white");
    }

    #[test]
    fn method_name_matches_the_documented_strings() {
        assert_eq!(method_name(BinarizationMethod::Otsu), "otsu");
        assert_eq!(
            method_name(BinarizationMethod::Manual { threshold: 1 }),
            "manual"
        );
        assert_eq!(
            method_name(BinarizationMethod::Sauvola(Default::default())),
            "sauvola"
        );
    }

    // --- Single-session behaviour, proved without PDFium -------------
    //
    // `MockSession` is a `DocumentSession` backed by pre-rendered, purely
    // synthetic in-memory pages. It counts every `render_page` call. If
    // `process_with_session`/`analyze_with_session` ever reopened the
    // source or created a second session, there would be nothing to
    // reopen here — the mock *is* the one session, constructed exactly
    // once by the test, and the assertions below prove every page came
    // from it.

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use crate::document::{PdfDocumentInfo, PdfMetadata, PdfPageInfo};
    use crate::document_session::DocumentSession;
    use crate::page_geometry::{PageGeometry, PageRotation};
    use crate::source_identity::SourceIdentity;
    use image::{DynamicImage, Rgb, RgbImage};

    struct MockSession {
        info: PdfDocumentInfo,
        identity: SourceIdentity,
        render_calls: AtomicU32,
        /// Records the index passed to every `render_page` call, in
        /// order, so a test can assert exactly which pages were served
        /// and how many times — not just a count.
        rendered_indices: Mutex<Vec<u32>>,
    }

    impl MockSession {
        fn with_pages(count: u32) -> Self {
            let pages = (0..count)
                .map(|index| PdfPageInfo {
                    index,
                    geometry: PageGeometry::new(200.0, 150.0).unwrap(),
                    source_rotation: PageRotation::None,
                })
                .collect();
            Self {
                info: PdfDocumentInfo {
                    page_count: count,
                    pages,
                    metadata: PdfMetadata::default(),
                    source_bytes: 1_234,
                },
                identity: SourceIdentity {
                    canonical_path: PathBuf::from("/mock/source.pdf"),
                    byte_len: 1_234,
                    modified_time: None,
                    content_sha256: None,
                },
                render_calls: AtomicU32::new(0),
                rendered_indices: Mutex::new(Vec::new()),
            }
        }

        fn render_call_count(&self) -> u32 {
            self.render_calls.load(Ordering::SeqCst)
        }
    }

    impl DocumentSession for MockSession {
        fn info(&self) -> &PdfDocumentInfo {
            &self.info
        }

        fn source_identity(&self) -> &SourceIdentity {
            &self.identity
        }

        fn pdfium_library_description(&self) -> String {
            "mock session (no real PDFium library involved)".to_string()
        }

        fn render_page(&self, index: u32, _dpi: u16) -> Result<DynamicImage> {
            self.render_calls.fetch_add(1, Ordering::SeqCst);
            self.rendered_indices.lock().unwrap().push(index);
            // A small, cheap synthetic page: a light background with one
            // dark block, big enough for binarization to produce both
            // black and white pixels.
            let mut rgb = RgbImage::from_pixel(20, 20, Rgb([250, 250, 250]));
            for y in 5..15 {
                for x in 5..15 {
                    rgb.put_pixel(x, y, Rgb([10, 10, 10]));
                }
            }
            Ok(DynamicImage::ImageRgb8(rgb))
        }
    }

    fn mock_settings() -> ProcessingSettings {
        ProcessingSettings {
            dpi: 300,
            method: BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: Default::default(),
            cleanup: Default::default(),
        }
    }

    /// `process_with_open_session` always uses the real output validator
    /// (unlike `process_with_session`, whose validator is injectable only
    /// for this test module), so its success path needs a real PDFium
    /// library and is covered by the `#[ignore]`d integration test
    /// `process_with_open_session_reuses_an_already_open_session_without_reopening_the_source`
    /// in `tests/pdf_pipeline.rs`. This ordinary test covers the one thing
    /// that must reject *before* validation is ever reached: an output
    /// path that aliases the session's own source.
    #[test]
    fn process_with_open_session_rejects_output_aliasing_the_session_source() {
        let session = MockSession::with_pages(2);
        let output = session.source_identity().canonical_path.clone();

        let err = process_with_open_session(
            &session,
            &output,
            &mock_settings(),
            &PdfProcessingOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap_err();

        assert!(matches!(err, CoreError::DestinationConflict(_)));
        assert_eq!(
            session.render_call_count(),
            0,
            "an aliased destination must be rejected before any page is rendered"
        );
    }

    #[test]
    fn process_with_session_renders_every_page_exactly_once_from_the_one_session() {
        let session = MockSession::with_pages(5);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");

        // No real PDFium library is available in this ordinary test, so
        // the output validator is stubbed out here — proving the
        // single-session *rendering* behaviour does not require
        // reopening a synthetic PDF with a real PDFium binding. The real
        // `process_pdf` always uses the genuine reopen-and-render
        // validator; see the `#[ignore]`d integration tests for that.
        let report = process_with_session(
            &session,
            &output,
            &mock_settings(),
            &PdfProcessingOptions::default(),
            &crate::progress::NullProgressReporter,
            Instant::now(),
            |_path, _expected, _mode, _pdfium| Ok(()),
        )
        .unwrap();

        assert_eq!(report.pages_processed, 5);
        assert_eq!(
            session.render_call_count(),
            5,
            "exactly one render_page call per page, from the single session"
        );
        assert_eq!(
            *session.rendered_indices.lock().unwrap(),
            vec![0, 1, 2, 3, 4],
            "pages must be rendered once each, in order, from the one session"
        );
        assert!(output.exists());
    }

    #[test]
    fn analyze_with_session_renders_every_selected_page_exactly_once() {
        let session = MockSession::with_pages(4);
        let options = AnalysisOptions::default();

        let report = analyze_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();

        assert_eq!(report.analyzed_page_count, 4);
        assert_eq!(report.failed_page_count, 0);
        assert_eq!(session.render_call_count(), 4);
        assert_eq!(*session.rendered_indices.lock().unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn analyze_with_session_honours_a_page_selection_and_renders_only_those_pages() {
        let session = MockSession::with_pages(10);
        let options = AnalysisOptions {
            pages: Some("2,4-5".to_string()),
            ..Default::default()
        };

        let report = analyze_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();

        assert_eq!(report.analyzed_page_count, 3);
        assert_eq!(session.render_call_count(), 3);
        assert_eq!(
            *session.rendered_indices.lock().unwrap(),
            vec![1, 3, 4],
            "only the selected zero-based indices must be rendered"
        );
        let reported_numbers: Vec<u32> = report.pages.iter().map(|p| p.page_number).collect();
        assert_eq!(reported_numbers, vec![2, 4, 5]);
    }

    #[test]
    fn analyze_report_carries_the_configured_dpi_and_method() {
        let session = MockSession::with_pages(1);
        let mut settings = mock_settings();
        settings.dpi = 600;
        let report = analyze_with_session(
            &session,
            &settings,
            &AnalysisOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert_eq!(report.dpi, 600);
        assert_eq!(report.method, "otsu");
    }

    #[test]
    fn analyze_computes_total_visible_area_from_the_analyzed_pages() {
        let session = MockSession::with_pages(2);
        let report = analyze_with_session(
            &session,
            &mock_settings(),
            &AnalysisOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        // Each mock page is 200.0 x 150.0 points.
        assert!((report.total_visible_area_points2 - 2.0 * 200.0 * 150.0).abs() < 1e-6);
    }

    #[test]
    fn analyze_rejects_an_out_of_range_page_selection() {
        let session = MockSession::with_pages(3);
        let options = AnalysisOptions {
            pages: Some("5".to_string()),
            ..Default::default()
        };
        let err = analyze_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidParameter(_)));
        assert_eq!(
            session.render_call_count(),
            0,
            "an invalid selection must be rejected before any page is rendered"
        );
    }

    #[test]
    fn analyze_with_encode_reports_ccitt_bytes() {
        let session = MockSession::with_pages(1);
        let options = AnalysisOptions {
            encode: true,
            ..Default::default()
        };
        let report = analyze_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        let page = &report.pages[0];
        assert!(page.ccitt_bytes.is_some());
        assert!(page.ccitt_bytes_per_pixel.is_some());
        assert!(page.stage_durations.ccitt_encode_us.is_some());
    }

    #[test]
    fn analyze_without_encode_omits_ccitt_fields() {
        let session = MockSession::with_pages(1);
        let report = analyze_with_session(
            &session,
            &mock_settings(),
            &AnalysisOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        let page = &report.pages[0];
        assert!(page.ccitt_bytes.is_none());
        assert!(page.ccitt_bytes_per_pixel.is_none());
        assert!(page.stage_durations.ccitt_encode_us.is_none());
    }

    // --- estimate_with_session ------------------------------------------

    impl MockSession {
        /// A session whose pages have the given `(width_points,
        /// height_points)` geometry each — used to prove the estimator's
        /// extrapolation actually accounts for each page's own declared
        /// size rather than assuming every page matches the sampled ones.
        fn with_page_sizes(sizes: &[(f32, f32)]) -> Self {
            let pages = sizes
                .iter()
                .enumerate()
                .map(|(i, &(w, h))| PdfPageInfo {
                    index: i as u32,
                    geometry: PageGeometry::new(w, h).unwrap(),
                    source_rotation: PageRotation::None,
                })
                .collect();
            Self {
                info: PdfDocumentInfo {
                    page_count: sizes.len() as u32,
                    pages,
                    metadata: PdfMetadata::default(),
                    source_bytes: 1_234,
                },
                identity: SourceIdentity {
                    canonical_path: PathBuf::from("/mock/source.pdf"),
                    byte_len: 1_234,
                    modified_time: None,
                    content_sha256: None,
                },
                render_calls: AtomicU32::new(0),
                rendered_indices: Mutex::new(Vec::new()),
            }
        }
    }

    /// A [`ProgressReporter`] that reports cancelled once `report` has
    /// been called at least `threshold` times — enough to force
    /// cancellation at a specific point in a loop without needing real
    /// concurrency.
    struct CancelAfter {
        calls: AtomicU32,
        threshold: u32,
    }

    impl ProgressReporter for CancelAfter {
        fn report(&self, _event: ProgressEvent) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        fn is_cancelled(&self) -> bool {
            self.calls.load(Ordering::SeqCst) >= self.threshold
        }
    }

    #[test]
    fn estimate_uses_exactly_the_deterministic_sample_pages() {
        let session = MockSession::with_pages(20);
        let options = EstimationOptions {
            samples: 8,
            ..Default::default()
        };
        let report = estimate_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();

        let expected = crate::estimation::select_sample_pages(20, 8);
        assert_eq!(*session.rendered_indices.lock().unwrap(), expected);
        assert_eq!(
            report
                .sampled_pages
                .iter()
                .map(|s| s.page_index)
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn estimate_central_value_is_between_lower_and_upper() {
        let session = MockSession::with_pages(20);
        let report = estimate_with_session(
            &session,
            &mock_settings(),
            &EstimationOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert!(report.estimated_lower_bytes <= report.estimated_output_bytes);
        assert!(report.estimated_output_bytes <= report.estimated_upper_bytes);
    }

    #[test]
    fn estimate_uses_quartiles_for_the_default_sample_count() {
        let session = MockSession::with_pages(20);
        let report = estimate_with_session(
            &session,
            &mock_settings(),
            &EstimationOptions::default(), // 8 samples, >= QUARTILE_MIN_SAMPLES
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert_eq!(
            report.range_method,
            crate::estimation::EstimationRangeMethod::Quartiles
        );
    }

    #[test]
    fn estimate_uses_min_max_for_a_small_sample_count() {
        let session = MockSession::with_pages(20);
        let options = EstimationOptions {
            samples: 2,
            ..Default::default()
        };
        let report = estimate_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert_eq!(
            report.range_method,
            crate::estimation::EstimationRangeMethod::MinMax
        );
    }

    /// The mock's synthetic rendered content is identical regardless of a
    /// page's declared geometry (it ignores both `dpi` and the caller's
    /// page size), so two documents that differ only in page geometry
    /// must produce *identical* measured bytes-per-pixel but *different*
    /// estimated totals — proving the extrapolation is driven by each
    /// page's own real pixel count, not a uniform per-page assumption.
    #[test]
    fn estimate_scales_with_true_per_page_pixel_count_for_mixed_page_sizes() {
        let uniform = MockSession::with_page_sizes(&[(200.0, 150.0); 4]);
        let mixed = MockSession::with_page_sizes(&[
            (200.0, 150.0),
            (200.0, 150.0),
            (400.0, 300.0), // 4x the area of a uniform page
            (400.0, 300.0),
        ]);
        let options = EstimationOptions {
            samples: 4, // sample every page, deterministically
            ..Default::default()
        };
        let settings = mock_settings();

        let uniform_report = estimate_with_session(
            &uniform,
            &settings,
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        let mixed_report = estimate_with_session(
            &mixed,
            &settings,
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap();

        assert_eq!(
            uniform_report.median_bytes_per_pixel, mixed_report.median_bytes_per_pixel,
            "identical rendered content must measure identical bytes-per-pixel regardless of declared page size"
        );

        let expected_uniform_pixels: u128 = (0..4)
            .map(|_| {
                let (w, h) = PageGeometry::new(200.0, 150.0)
                    .unwrap()
                    .pixel_size(settings.dpi)
                    .unwrap();
                u128::from(w) * u128::from(h)
            })
            .sum();
        let expected_mixed_pixels: u128 = [
            (200.0, 150.0),
            (200.0, 150.0),
            (400.0, 300.0),
            (400.0, 300.0),
        ]
        .iter()
        .map(|&(w, h)| {
            let (pw, ph) = PageGeometry::new(w, h)
                .unwrap()
                .pixel_size(settings.dpi)
                .unwrap();
            u128::from(pw) * u128::from(ph)
        })
        .sum();

        // `estimated_output_bytes` now also carries a fixed PDF-container
        // overhead (page/image/content objects, document trailer — see
        // `pdf_writer::measure_container_overhead`), which is identical for
        // both reports since they share the same page count. That constant
        // must be backed out before comparing ratios, or it skews the
        // ratio toward 1 regardless of how the image content itself scales.
        let overhead = crate::pdf_writer::measure_container_overhead();
        let container_bytes = overhead.fixed_document_bytes
            + overhead.per_page_bytes * uniform.info.page_count as u64;
        let expected_ratio = expected_mixed_pixels as f64 / expected_uniform_pixels as f64;
        let actual_ratio = (mixed_report.estimated_output_bytes - container_bytes) as f64
            / (uniform_report.estimated_output_bytes - container_bytes) as f64;
        assert!(
            (actual_ratio - expected_ratio).abs() < 0.01,
            "expected ratio {expected_ratio}, got {actual_ratio}"
        );
    }

    #[test]
    fn estimate_rejects_an_invalid_sample_count_before_rendering_anything() {
        let session = MockSession::with_pages(20);
        let options = EstimationOptions {
            samples: 0,
            ..Default::default()
        };
        let err = estimate_with_session(
            &session,
            &mock_settings(),
            &options,
            &crate::progress::NullProgressReporter,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidParameter(_)));
        assert_eq!(session.render_call_count(), 0);
    }

    #[test]
    fn estimate_report_serializes_without_nan_and_round_trips() {
        let session = MockSession::with_pages(8);
        let report = estimate_with_session(
            &session,
            &mock_settings(),
            &EstimationOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let back: crate::estimation::SizeEstimateReport = serde_json::from_str(&json).unwrap();
        // Not a full `assert_eq!(report, back)`: a mean computed by
        // summing floats in a particular order is not guaranteed to
        // round-trip to a bit-identical f64 through decimal text, even
        // though JSON itself is not lossy here. Compare structurally
        // where exactness is meaningful (integers, counts, the sample
        // list) and approximately for derived floating-point statistics.
        assert_eq!(report.document_page_count, back.document_page_count);
        assert_eq!(report.sampled_pages, back.sampled_pages);
        assert_eq!(report.estimated_output_bytes, back.estimated_output_bytes);
        assert_eq!(report.estimated_lower_bytes, back.estimated_lower_bytes);
        assert_eq!(report.estimated_upper_bytes, back.estimated_upper_bytes);
        assert_eq!(report.range_method, back.range_method);
        assert_eq!(report.settings_fingerprint, back.settings_fingerprint);
        for (a, b) in [
            (report.mean_bytes_per_pixel, back.mean_bytes_per_pixel),
            (report.median_bytes_per_pixel, back.median_bytes_per_pixel),
            (report.min_bytes_per_pixel, back.min_bytes_per_pixel),
            (report.max_bytes_per_pixel, back.max_bytes_per_pixel),
        ] {
            assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        }
        assert!(report.experimental);
    }

    #[test]
    fn estimate_is_deterministic_for_identical_settings_and_samples() {
        let a = estimate_with_session(
            &MockSession::with_pages(37),
            &mock_settings(),
            &EstimationOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        let b = estimate_with_session(
            &MockSession::with_pages(37),
            &mock_settings(),
            &EstimationOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert_eq!(
            a.sampled_pages
                .iter()
                .map(|s| s.page_index)
                .collect::<Vec<_>>(),
            b.sampled_pages
                .iter()
                .map(|s| s.page_index)
                .collect::<Vec<_>>()
        );
        assert_eq!(a.estimated_output_bytes, b.estimated_output_bytes);
        assert_eq!(a.estimated_lower_bytes, b.estimated_lower_bytes);
        assert_eq!(a.estimated_upper_bytes, b.estimated_upper_bytes);
    }

    #[test]
    fn estimate_is_cancelled_before_any_page_renders_when_already_cancelled() {
        let session = MockSession::with_pages(20);
        let reporter = CancelAfter {
            calls: AtomicU32::new(0),
            threshold: 1, // trips after the initial `Started` report
        };
        let err = estimate_with_session(
            &session,
            &mock_settings(),
            &EstimationOptions::default(),
            &reporter,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::Cancelled));
        assert_eq!(session.render_call_count(), 0);
    }

    #[test]
    fn estimate_cancellation_mid_run_stops_before_the_next_sample() {
        let session = MockSession::with_pages(20);
        // Started(1) + PageStarted(2) + PageFinished(3) for the first
        // sample, then the loop's next cancellation check trips before a
        // second page is ever rendered.
        let reporter = CancelAfter {
            calls: AtomicU32::new(0),
            threshold: 3,
        };
        let err = estimate_with_session(
            &session,
            &mock_settings(),
            &EstimationOptions::default(),
            &reporter,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::Cancelled));
        assert_eq!(
            session.render_call_count(),
            1,
            "exactly one sample must have rendered before cancellation was observed"
        );
    }

    #[test]
    fn estimate_settings_fingerprint_matches_the_settings_used() {
        let session = MockSession::with_pages(4);
        let settings = mock_settings();
        let report = estimate_with_session(
            &session,
            &settings,
            &EstimationOptions::default(),
            &crate::progress::NullProgressReporter,
        )
        .unwrap();
        assert_eq!(
            report.settings_fingerprint,
            crate::estimation::settings_fingerprint(&settings)
        );
    }
}
