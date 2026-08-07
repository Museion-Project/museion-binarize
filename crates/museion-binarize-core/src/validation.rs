//! Output validation, kept separate from PDF construction so that a bug
//! in the writer cannot also silently define what "valid" means.
//!
//! Validation always *reopens* the finished file with PDFium and renders
//! from it. Inspecting the raw bytes for `/CCITTFaxDecode` is treated as a
//! supplementary structural assertion, never as proof of validity.

use std::path::Path;

use crate::error::{CoreError, Result};
use crate::pdfium_backend::PdfiumConfig;
use crate::renderer::{PdfOpenOptions, PdfRenderer};

/// How thoroughly a finished output PDF is checked before it is persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationMode {
    /// Reopen the output, confirm the page count and every page's visible
    /// dimensions, and render the first and last page.
    #[default]
    Structural,
    /// As `Structural`, but render *every* page. Slower; used by
    /// integration tests and available to users via `--validate render-all`.
    RenderAll,
}

/// Tolerance, in PDF points, when comparing an output page's visible
/// dimensions against the expected value.
///
/// Page sizes make a round trip through `f32` points, a rounded integer
/// pixel count, and back, so an exact comparison would be wrong. A tenth
/// of a point is about 0.035 mm — far below anything visible, but tight
/// enough to catch a genuine geometry bug.
pub const DIMENSION_TOLERANCE_POINTS: f32 = 0.1;

/// What the output is expected to contain.
#[derive(Debug, Clone)]
pub struct ExpectedOutput {
    pub page_count: u32,
    /// Expected visible (width, height) in points, per page, in order.
    pub page_dimensions: Vec<(f32, f32)>,
}

/// Validates a written output PDF by reopening it with PDFium.
///
/// `dpi` is used only to keep validation renders cheap; correctness of the
/// output does not depend on it.
pub fn validate_output(
    path: &Path,
    expected: &ExpectedOutput,
    mode: ValidationMode,
    pdfium: &PdfiumConfig,
) -> Result<()> {
    let options = PdfOpenOptions {
        password: None,
        pdfium: pdfium.clone(),
    };
    let renderer = PdfRenderer::open(path, &options).map_err(|e| {
        CoreError::OutputValidationFailed(format!("the written output could not be reopened: {e}"))
    })?;
    let info = renderer.info();

    if info.page_count != expected.page_count {
        return Err(CoreError::OutputValidationFailed(format!(
            "output has {} pages, expected {}",
            info.page_count, expected.page_count
        )));
    }

    for (index, expected_size) in expected.page_dimensions.iter().enumerate() {
        let page = info.pages.get(index).ok_or_else(|| {
            CoreError::OutputValidationFailed(format!("output is missing page {}", index + 1))
        })?;
        let actual = (page.geometry.width_points, page.geometry.height_points);
        let dw = (actual.0 - expected_size.0).abs();
        let dh = (actual.1 - expected_size.1).abs();
        if dw > DIMENSION_TOLERANCE_POINTS || dh > DIMENSION_TOLERANCE_POINTS {
            return Err(CoreError::OutputValidationFailed(format!(
                "page {} is {:.3}x{:.3} points, expected {:.3}x{:.3} (tolerance {})",
                index + 1,
                actual.0,
                actual.1,
                expected_size.0,
                expected_size.1,
                DIMENSION_TOLERANCE_POINTS
            )));
        }

        // Rebuilt pages are written upright with no /Rotate, because the
        // rasterized image already shows the page in its visible
        // orientation. A rotation here would mean the writer emitted one.
        if page.source_rotation != crate::page_geometry::PageRotation::None {
            return Err(CoreError::OutputValidationFailed(format!(
                "page {} unexpectedly carries a {} degree rotation; output pages must be upright",
                index + 1,
                page.source_rotation.degrees()
            )));
        }
    }

    // A low render DPI is enough to prove the page decodes; validation is
    // about structural soundness, not image quality.
    const VALIDATION_DPI: u16 = 72;
    let pages_to_render: Vec<u32> = match mode {
        ValidationMode::Structural => {
            if info.page_count == 1 {
                vec![0]
            } else {
                vec![0, info.page_count - 1]
            }
        }
        ValidationMode::RenderAll => (0..info.page_count).collect(),
    };

    for index in pages_to_render {
        renderer.render_page(index, VALIDATION_DPI).map_err(|e| {
            CoreError::OutputValidationFailed(format!(
                "output page {} could not be rendered: {e}",
                index + 1
            ))
        })?;
    }

    Ok(())
}

/// Supplementary structural assertion over the raw output bytes.
///
/// This exists so a regression that swapped CCITT for some other filter
/// would be caught loudly. It is *not* a substitute for reopening and
/// rendering the file.
pub fn assert_bilevel_ccitt_structure(bytes: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(bytes);
    for needle in ["/CCITTFaxDecode", "/BitsPerComponent 1", "/K -1"] {
        if !text.contains(needle) {
            return Err(CoreError::OutputValidationFailed(format!(
                "the output does not contain the expected bilevel marker {needle}"
            )));
        }
    }
    for forbidden in ["/DCTDecode", "/JBIG2Decode", "/JPXDecode"] {
        if text.contains(forbidden) {
            return Err(CoreError::OutputValidationFailed(format!(
                "the output unexpectedly contains {forbidden}; pages must be CCITT Group 4"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_check_accepts_a_bilevel_ccitt_document() {
        let bytes = b"<< /Filter /CCITTFaxDecode /BitsPerComponent 1 /DecodeParms << /K -1 >> >>";
        assert!(assert_bilevel_ccitt_structure(bytes).is_ok());
    }

    #[test]
    fn structure_check_rejects_a_missing_ccitt_filter() {
        let bytes = b"<< /Filter /FlateDecode /BitsPerComponent 8 >>";
        assert!(assert_bilevel_ccitt_structure(bytes).is_err());
    }

    #[test]
    fn structure_check_rejects_lossy_filters() {
        let bytes =
            b"<< /Filter /CCITTFaxDecode /BitsPerComponent 1 /K -1 >> << /Filter /DCTDecode >>";
        let err = assert_bilevel_ccitt_structure(bytes).unwrap_err();
        assert!(err.to_string().contains("DCTDecode"));
    }

    #[test]
    fn structure_check_rejects_jbig2() {
        let bytes = b"/CCITTFaxDecode /BitsPerComponent 1 /K -1 /JBIG2Decode";
        assert!(assert_bilevel_ccitt_structure(bytes).is_err());
    }

    #[test]
    fn default_validation_mode_is_structural() {
        assert_eq!(ValidationMode::default(), ValidationMode::Structural);
    }

    #[test]
    fn validating_a_nonexistent_file_is_an_error_not_a_panic() {
        let expected = ExpectedOutput {
            page_count: 1,
            page_dimensions: vec![(595.0, 842.0)],
        };
        let result = validate_output(
            Path::new("/nonexistent/output.pdf"),
            &expected,
            ValidationMode::Structural,
            &PdfiumConfig::default(),
        );
        assert!(result.is_err());
    }
}
