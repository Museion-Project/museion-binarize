//! The end-to-end PDF conversion pipeline.
//!
//! ```text
//! input.pdf
//!   -> PDFium rasterization        (renderer)
//!   -> image-processing core       (image_pipeline)
//!   -> packed bilevel image        (bilevel)
//!   -> CCITT Group 4               (ccitt, via pdf_writer::EncodedPage)
//!   -> rebuilt 1-bit PDF           (pdf_writer)
//!   -> temporary file, validated, then atomically persisted
//! ```
//!
//! ## Memory behaviour
//!
//! Pages are processed strictly one at a time: the rendered bitmap, the
//! grayscale buffer, and the binary mask for page N are all dropped before
//! page N+1 is rendered. Only the *compressed* CCITT stream of each page
//! is retained, because the PDF writer assembles the document in memory.
//!
//! The honest bound is therefore:
//!
//! > approximately one uncompressed working page
//! > + algorithm working buffers
//! > + the growing compressed output PDF
//!
//! This is **not** O(1) with respect to document length — the compressed
//! output grows — and the documentation does not claim otherwise.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::document::PdfDocumentInfo;
use crate::error::{CoreError, Result};
use crate::image_pipeline::process_rendered_page;
use crate::pdf_writer::{BilevelPdfBuilder, EncodedPage};
use crate::pdfium_backend::{self, PdfiumConfig};
use crate::progress::{ProcessingStage, ProgressEvent, ProgressReporter};
use crate::renderer::{PdfOpenOptions, PdfRenderer};
use crate::settings::ProcessingSettings;
use crate::validation::{self, ExpectedOutput, ValidationMode};

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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessingReport {
    pub pages_processed: u32,
    pub original_bytes: u64,
    pub output_bytes: u64,
    pub elapsed: Duration,
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
pub fn inspect_pdf(input: &Path, options: &PdfOpenOptions) -> Result<PdfDocumentInfo> {
    let renderer = PdfRenderer::open(input, options)?;
    Ok(renderer.info().clone())
}

/// Describes which PDFium library a given configuration would use, without
/// opening any document.
pub fn describe_pdfium_library(config: &PdfiumConfig) -> Result<String> {
    let resolved = pdfium_backend::resolve_library(config)?;
    Ok(pdfium_backend::describe_resolved(&resolved))
}

/// Renders and processes a single page, returning the processed bilevel
/// image as a grayscale preview image. Used by the CLI `preview` command.
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
    let renderer = PdfRenderer::open(input, options)?;
    let index = page_number - 1;
    if index >= renderer.info().page_count {
        return Err(CoreError::InvalidParameter(format!(
            "page {page_number} is out of range; the document has {} pages",
            renderer.info().page_count
        )));
    }

    let rendered = renderer.render_page(index, settings.dpi)?;
    let bilevel = process_rendered_page(&rendered, settings)?;
    Ok(bilevel_to_gray(&bilevel))
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

    // Cancellation check: before opening.
    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    let open_options = PdfOpenOptions {
        password: options.password.clone(),
        pdfium: options.pdfium.clone(),
    };
    let renderer = PdfRenderer::open(input, &open_options)?;
    let info = renderer.info().clone();
    let pdfium_library = pdfium_backend::describe_resolved(renderer.resolved_library());

    // Cancellation check: after opening.
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

        // Cancellation check: before every page.
        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }
        progress.report(ProgressEvent::PageStarted { page: page_number });

        // Rotation is normalized into the geometry: PDFium renders the
        // page in its visible orientation, so the rebuilt page uses the
        // visible rectangle and is written upright.
        let width_points = page_info.geometry.display_width_points();
        let height_points = page_info.geometry.display_height_points();

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Rendering,
        });
        let rendered = renderer.render_page(page_info.index, settings.dpi)?;

        // Cancellation check: after render.
        if progress.is_cancelled() {
            progress.report(ProgressEvent::Cancelled);
            return Err(CoreError::Cancelled);
        }

        progress.report(ProgressEvent::StageChanged {
            page: page_number,
            stage: ProcessingStage::Binarization,
        });
        let bilevel = process_rendered_page(&rendered, settings)?;
        // The rendered RGB page is the largest buffer in flight; release
        // it as soon as the bilevel image exists.
        drop(rendered);

        // Cancellation check: after image processing.
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
        // Release the uncompressed bilevel page; only the compressed
        // stream inside `encoded` is retained from here on.
        drop(bilevel);

        // Cancellation check: after CCITT encoding.
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

    // Cancellation check: before final persistence.
    if progress.is_cancelled() {
        progress.report(ProgressEvent::Cancelled);
        return Err(CoreError::Cancelled);
    }

    // Structural sanity check on the bytes we are about to write. Cheap,
    // and catches a catastrophic writer regression before touching disk.
    validation::assert_bilevel_ccitt_structure(&bytes)?;

    let temp = write_temporary(output, &bytes)?;

    // Cancellation check: before validation.
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
    // On failure `temp` is dropped on the way out, which removes the
    // invalid file and leaves the destination untouched.
    validation::validate_output(temp.path(), &expected, options.validation, &options.pdfium)?;

    let output_bytes = bytes.len() as u64;
    persist(temp, output, options.overwrite)?;

    progress.report(ProgressEvent::Finished);
    Ok(ProcessingReport {
        pages_processed: info.page_count,
        original_bytes: info.source_bytes,
        output_bytes,
        elapsed: started.elapsed(),
        page_reports,
        pdfium_library,
    })
}

/// Rejects unsafe destinations before any work begins.
fn check_destination(input: &Path, output: &Path, overwrite: bool) -> Result<()> {
    if paths_refer_to_same_file(input, output) {
        return Err(CoreError::DestinationConflict(
            "the output path is the same file as the input; the source is never overwritten"
                .to_string(),
        ));
    }
    if output.exists() && !overwrite {
        return Err(CoreError::DestinationConflict(format!(
            "{} already exists; pass the overwrite option to replace it",
            output.display()
        )));
    }
    if output.is_dir() {
        return Err(CoreError::DestinationConflict(format!(
            "{} is a directory",
            output.display()
        )));
    }
    Ok(())
}

/// Compares two paths, using filesystem identity when both exist so that
/// symlinks and `./` differences are handled correctly.
fn paths_refer_to_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
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
/// With `overwrite` the existing destination is only removed once the
/// replacement is complete and validated, so a failure can never leave the
/// user with neither file. On Windows a rename onto an existing path
/// fails, so the old file is unlinked immediately before the rename; that
/// leaves a very small window in which neither name exists, which is
/// documented in `docs/pdf-output.md`.
fn persist(temp: tempfile::NamedTempFile, output: &Path, overwrite: bool) -> Result<()> {
    if overwrite && output.exists() {
        std::fs::remove_file(output).map_err(|e| CoreError::io(output, e))?;
    }
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
            elapsed: Duration::from_secs(1),
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
            elapsed: Duration::ZERO,
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
            elapsed: Duration::ZERO,
            page_reports: Vec::new(),
            pdfium_library: "test".to_string(),
        };
        assert_eq!(report.reduction_percent(), Some(-50.0));
    }

    #[test]
    fn preview_rejects_page_zero_before_touching_pdfium() {
        let settings = ProcessingSettings {
            dpi: 300,
            method: crate::settings::BinarizationMethod::Otsu,
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
}
