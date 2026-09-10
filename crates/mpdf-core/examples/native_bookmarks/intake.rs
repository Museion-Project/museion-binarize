//! Native-text-only intake for bookmarks. No provider, raster, or network.
//!
//! Reuses the existing native-text record contract as a compatibility view;
//! this is not an OCR run over scanned pages. All pages are inspected, but
//! empty or unusable text layers remain empty and are reported separately.
//! Line/word boxes are deliberately approximate, as in the existing native
//! route. They must never be used to recover indentation or precise font size.

use std::path::Path;

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use mpdf_core::derived::DerivedDocument;
use mpdf_core::document_package::{DocumentPackage, CANONICAL_MASTER_DPI};
use mpdf_core::document_session::{DocumentSession, PdfDocumentSession, PdfOpenOptions};
use mpdf_core::error::{CoreError, Result};
use mpdf_core::ocr::{
    OcrBlock, OcrBox, OcrLine, OcrPage, OcrRoute, OcrRun, OcrWord, OCR_PROTOCOL,
    OCR_PROTOCOL_VERSION,
};

use mpdf_core::bookmarks::AutoBookmarkInputs;

#[derive(Debug, Clone, Serialize)]
pub struct NativeBookmarkDiagnostics {
    pub text_pages: usize,
    /// Zero-based PDF page indices. Empty extraction does not prove a blank page.
    pub pages_without_text: Vec<u32>,
    pub pages_with_unusable_text: Vec<u32>,
    /// Native extraction anomalies retained verbatim, e.g. PDFium's hyphen marker.
    pub pages_with_text_warnings: Vec<u32>,
    pub existing_outline: bool,
    pub geometry: &'static str,
}

/// Opens and hashes one source snapshot, then extracts actual native text.
/// Existing outlines take priority and do not require a text scan.
pub fn load_native_bookmark_inputs(
    source: &Path,
    options: &PdfOpenOptions,
    cancelled: &dyn Fn() -> bool,
) -> Result<(AutoBookmarkInputs, NativeBookmarkDiagnostics)> {
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    let session = PdfDocumentSession::open(
        source,
        &PdfOpenOptions {
            compute_source_hash: true,
            ..options.clone()
        },
    )?;
    let package = DocumentPackage::create_from_session(
        &session,
        source.file_name().map(|s| s.to_string_lossy().into_owned()),
    )?;
    let mut diagnostics = NativeBookmarkDiagnostics {
        text_pages: 0,
        pages_without_text: vec![],
        pages_with_unusable_text: vec![],
        pages_with_text_warnings: vec![],
        existing_outline: package
            .pages
            .iter()
            .any(|p| !p.existing_outline_evidence.is_empty()),
        geometry: "approximate_native_text",
    };
    if diagnostics.existing_outline {
        return Ok((
            AutoBookmarkInputs {
                package,
                ocr: None,
                derived: None,
            },
            diagnostics,
        ));
    }
    let mut run = OcrRun {
        protocol: OCR_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        pages: vec![],
        errors: vec![],
    };
    for page in &session.info().pages {
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let native = session.native_text(page.index)?;
        if native.text.contains('\u{fffe}')
            || native
                .text
                .chars()
                .any(|c| c.is_control() && !c.is_whitespace())
        {
            diagnostics.pages_with_text_warnings.push(page.index);
        }
        if native.text.len() > 4_000_000 {
            return Err(CoreError::InvalidDocument(
                "native page exceeds bookmark text budget".into(),
            ));
        }
        let text = if unusable(&native.text) {
            diagnostics.pages_with_unusable_text.push(page.index);
            ""
        } else if native.text.trim().is_empty() {
            diagnostics.pages_without_text.push(page.index);
            ""
        } else {
            diagnostics.text_pages += 1;
            &native.text
        };
        let (width, height) = page.geometry.pixel_size(CANONICAL_MASTER_DPI)?;
        run.pages
            .push(native_record(page.index, text, width, height));
    }
    run.validate()
        .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
    let derived = DerivedDocument::from_package(&package, Some(&run))?;
    Ok((
        AutoBookmarkInputs {
            package,
            ocr: Some(run),
            derived: Some(derived),
        },
        diagnostics,
    ))
}

fn unusable(text: &str) -> bool {
    // Same garbling criterion as the current native route. U+FFFE can be a
    // PDFium layout marker for a hyphen, so retain it with a warning rather
    // than discarding every other line on that page or guessing a replacement.
    text.contains('\u{fffd}')
        || text.contains('\u{ffff}')
        || text
            .chars()
            .filter(|c| c.is_control() && !c.is_whitespace())
            .count()
            > 2
}

fn native_record(page_index: u32, text: &str, width: u32, height: u32) -> OcrPage {
    let lines: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let line_height = (height.saturating_sub(1) as f32 / lines.len().max(1) as f32).min(80.0);
    let blocks = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let tokens: Vec<_> = line.split_whitespace().collect();
            let word_width = width.saturating_sub(1) as f32 / tokens.len().max(1) as f32;
            let bbox = OcrBox {
                x: 0.0,
                y: i as f32 * line_height,
                width: width as f32,
                height: line_height,
            };
            let words = tokens
                .iter()
                .enumerate()
                .map(|(j, token)| OcrWord {
                    text: (*token).into(),
                    normalized_text: token.nfc().collect(),
                    bbox: OcrBox {
                        x: j as f32 * word_width,
                        y: bbox.y,
                        width: word_width,
                        height: line_height,
                    },
                    // Native extraction confidence, not semantic correctness of an old OCR layer.
                    confidence: 1.0,
                    reading_order: j as u32,
                })
                .collect();
            OcrBlock {
                bbox: bbox.clone(),
                confidence: 1.0,
                reading_order: i as u32,
                lines: vec![OcrLine {
                    bbox,
                    confidence: 1.0,
                    reading_order: 0,
                    words,
                }],
            }
        })
        .collect();
    OcrPage {
        page_index,
        route: OcrRoute::NativeText,
        width,
        height,
        blocks,
        revisions: vec![],
        provider_provenance: None,
        provider_raw_artifact: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_native_page_is_not_filled_with_recognition() {
        let page = native_record(4, " \n", 600, 800);
        assert!(page.blocks.is_empty());
        assert!(matches!(page.route, OcrRoute::NativeText));
        assert!(page.provider_provenance.is_none());
    }
    #[test]
    fn unicode_is_preserved_and_garbled_text_is_flagged() {
        let page = native_record(0, "1. ψυχή\ne\u{301}", 600, 800);
        let word = &page.blocks[1].lines[0].words[0];
        assert_eq!(word.text, "e\u{301}");
        assert_eq!(word.normalized_text, "é");
        assert!(!unusable("ψυχή\n目录\t1"));
        assert!(unusable("bad\u{fffd}"));
        assert!(!unusable("hyphen\u{fffe}marker"));
        assert!(unusable("bad\u{0001}\u{0002}\u{0003}"));
    }
}
