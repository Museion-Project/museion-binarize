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
}

/// Per-page results.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageProcessingReport {
    /// One-based page number.
    pub page_number: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub width_points: f32,
    pub height_points: f32,
    pub compressed_bytes: u64,
}

/// The result of a completed conversion.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
}

impl ProcessingReport {
    /// Size reduction as a percentage of the original, or `None` when the
    /// original size is unknown (zero).
    pub fn reduction_percent(&self) -> Option<f64> {
        if self.original_bytes == 0 {
            return None;
        }
        let original = self.original_bytes as f64;
        let output = self.output_bytes as f64;
        Some((original - output) / original * 100.0)
    }

    /// Mean output bytes per page.
    pub fn average_bytes_per_page(&self) -> Option<f64> {
        if self.pages_processed == 0 {
            return None;
        }
        Some(self.output_bytes as f64 / f64::from(self.pages_processed))
    }
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
        let rendered = session.render_page(page_info.index, settings.dpi)?;

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

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Encoding,
        });
        let (pixel_width, pixel_height) = (bilevel.width, bilevel.height);
        let encoded = EncodedPage::encode(&bilevel, width_points, height_points)?;
        drop(bilevel);

        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        let compressed_bytes = encoded.compressed_bytes();
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

    let temp = write_temporary(output, &bytes)?;

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
    persist(temp, output, options.overwrite)?;

    progress.report(ProgressEvent::Finished);
    Ok(ProcessingReport {
        pages_processed: info.page_count,
        original_bytes: info.source_bytes,
        output_bytes,
        elapsed_us: duration_to_micros(started.elapsed()),
        page_reports,
        pdfium_library,
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

        match analyze_one_page(session, page_info, settings, options) {
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

fn analyze_one_page(
    session: &impl DocumentSession,
    page_info: &crate::document::PdfPageInfo,
    settings: &ProcessingSettings,
    options: &AnalysisOptions,
) -> Result<PageAnalysisReport> {
    let (rendered, render_us) = timed(|| session.render_page(page_info.index, settings.dpi));
    let rendered = rendered?;
    let result = process_rendered_page(&rendered, settings)?;
    drop(rendered);

    let raw_raster_bytes_estimate =
        u64::from(result.bilevel.width) * u64::from(result.bilevel.height) * 3;
    let packed_bilevel_bytes = (result.bilevel.stride * result.bilevel.height as usize) as u64;

    let (ccitt_bytes, ccitt_encode_us) = if options.encode {
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

/// Writes `bytes` to a temporary file in the destination's directory, so
/// the later rename stays on one filesystem and is therefore atomic.
fn write_temporary(output: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let directory = output.parent().filter(|p| !p.as_os_str().is_empty());
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

/// Moves the validated temporary file into its final place.
///
/// # Platform behaviour
///
/// The temporary file always lives in the destination's own directory, so
/// the move stays on one filesystem.
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
/// In both cases the destination is only touched after the replacement has
/// been fully written, synced, and validated.
fn persist(temp: tempfile::NamedTempFile, output: &Path, overwrite: bool) -> Result<()> {
    // Windows cannot rename onto an existing file; Unix can, and doing so
    // atomically is the whole point, so it must not be unlinked there.
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

        let temp = write_temporary(&output, b"new contents").unwrap();
        persist(temp, &output, true).unwrap();

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
            let _temp = write_temporary(&output, b"replacement that never lands").unwrap();
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

        let temp = write_temporary(&output, b"%PDF-1.7 test").unwrap();
        assert_eq!(temp.path().parent().unwrap(), dir.path());
        assert!(
            !output.exists(),
            "destination must not exist before persist"
        );

        persist(temp, &output, false).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"%PDF-1.7 test");
        assert!(leftover_temporary_files(dir.path()).is_empty());
    }

    #[test]
    fn dropping_the_temporary_file_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.pdf");
        {
            let _temp = write_temporary(&output, b"partial").unwrap();
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

        let temp = write_temporary(&output, b"new").unwrap();
        persist(temp, &output, true).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"new");
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
        };
        assert_eq!(report.reduction_percent(), Some(-50.0));
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
}
