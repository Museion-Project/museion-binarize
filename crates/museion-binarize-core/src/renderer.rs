//! Museion-owned PDF rendering abstraction.
//!
//! [`PdfRenderer`] renders pages sequentially, releasing each page's
//! bitmap as soon as the caller is done with it. The rest of the pipeline
//! talks to this type, never to PDFium directly.
//!
//! Rendering is deliberately sequential: PDFium's thread-safety story is
//! not something this milestone measures, so no parallelism is introduced
//! (see `docs/adr/0001-pdfium-runtime-binding.md`).
//!
//! # Known limitation: the input file is reopened per page operation
//!
//! `PdfDocument` borrows from the `Pdfium` session, so holding one inside
//! this struct would make it self-referential. As a result
//! [`PdfRenderer::render_page`] reopens and reparses the source file on
//! every call, which costs one document parse per page and leaves a
//! time-of-check/time-of-use gap: a file mutated mid-run would be picked
//! up partway through. A persistent document session is deliberately out
//! of scope for this milestone — see the Milestone 3 entry in
//! `docs/roadmap.md` for the scoped follow-up.

use std::path::{Path, PathBuf};

use image::DynamicImage;
use pdfium_render::prelude::*;

use crate::document::{PdfDocumentInfo, PdfMetadata, PdfPageInfo};
use crate::error::{CoreError, Result};
use crate::page_geometry::{PageGeometry, PageRotation};
use crate::pdfium_backend::{self, PdfiumConfig, ResolvedLibrary};

/// Options for opening a source document.
#[derive(Debug, Clone, Default)]
pub struct PdfOpenOptions {
    /// Password for an encrypted document. Never logged or reported.
    pub password: Option<String>,
    pub pdfium: PdfiumConfig,
}

/// An open PDF document, bound to a live PDFium session.
///
/// The PDFium bindings are owned by this struct, so no global mutable
/// state is involved and two renderers can exist independently.
pub struct PdfRenderer {
    /// Borrowed from the process-wide PDFium session; PDFium can only be
    /// initialized once per process (see `pdfium_backend`).
    pdfium: &'static Pdfium,
    path: PathBuf,
    password: Option<String>,
    resolved_library: ResolvedLibrary,
    info: PdfDocumentInfo,
}

impl PdfRenderer {
    /// Opens `path`, binding PDFium according to `options`.
    pub fn open(path: &Path, options: &PdfOpenOptions) -> Result<Self> {
        let session = pdfium_backend::session(&options.pdfium)?;
        let pdfium = &session.pdfium;
        let resolved_library = session.resolved.clone();

        let source_bytes = std::fs::metadata(path)
            .map_err(|e| CoreError::io(path, e))?
            .len();

        let info = {
            let document = Self::load_document(pdfium, path, options.password.as_deref())?;
            Self::describe(&document, source_bytes)?
        };

        Ok(Self {
            pdfium,
            path: path.to_path_buf(),
            password: options.password.clone(),
            resolved_library,
            info,
        })
    }

    fn load_document<'a>(
        pdfium: &'a Pdfium,
        path: &Path,
        password: Option<&str>,
    ) -> Result<PdfDocument<'a>> {
        pdfium.load_pdf_from_file(path, password).map_err(|e| {
            // Distinguish a password problem from a generic open failure
            // where PDFium tells us enough to do so.
            if matches!(
                e,
                PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)
            ) {
                CoreError::PasswordRequired {
                    path: path.to_path_buf(),
                }
            } else {
                CoreError::PdfOpenFailed {
                    path: path.to_path_buf(),
                    reason: e.to_string(),
                }
            }
        })
    }

    /// Collects page geometry and metadata for the whole document.
    fn describe(document: &PdfDocument, source_bytes: u64) -> Result<PdfDocumentInfo> {
        let pages = document.pages();
        let page_count = pages.len() as u32;
        if page_count == 0 {
            return Err(CoreError::InvalidDocument(
                "the document contains no pages".to_string(),
            ));
        }

        let mut page_infos = Vec::with_capacity(page_count as usize);
        for index in 0..page_count {
            let page = pages
                .get(index as i32)
                .map_err(|e| CoreError::RenderFailed {
                    page_number: index + 1,
                    reason: format!("could not access page: {e}"),
                })?;

            // Page box policy: PDFium's `width()`/`height()` already
            // report the effective visible box (CropBox when present and
            // valid, otherwise MediaBox). See docs/pdf-output.md.
            let width_points = page.width().value;
            let height_points = page.height().value;
            let rotation = match page.rotation() {
                Ok(PdfPageRenderRotation::None) => PageRotation::None,
                Ok(PdfPageRenderRotation::Degrees90) => PageRotation::Degrees90,
                Ok(PdfPageRenderRotation::Degrees180) => PageRotation::Degrees180,
                Ok(PdfPageRenderRotation::Degrees270) => PageRotation::Degrees270,
                Err(e) => {
                    return Err(CoreError::InvalidPageGeometry(format!(
                        "could not read rotation of page {}: {e}",
                        index + 1
                    )))
                }
            };

            // PDFium's width()/height() already report the page's visible,
            // post-rotation dimensions, which is exactly the convention
            // PageGeometry stores. The source /Rotate is kept alongside as
            // informational metadata only: applying it here as well would
            // swap the axes a second time and transpose every rotated page.
            let geometry = PageGeometry::new(width_points, height_points)
                .map_err(|e| CoreError::InvalidPageGeometry(format!("page {}: {e}", index + 1)))?;

            page_infos.push(PdfPageInfo {
                index,
                geometry,
                source_rotation: rotation,
            });
        }

        let metadata = PdfMetadata {
            title: document
                .metadata()
                .get(PdfDocumentMetadataTagType::Title)
                .map(|t| t.value().to_string()),
            author: document
                .metadata()
                .get(PdfDocumentMetadataTagType::Author)
                .map(|t| t.value().to_string()),
            subject: document
                .metadata()
                .get(PdfDocumentMetadataTagType::Subject)
                .map(|t| t.value().to_string()),
            keywords: document
                .metadata()
                .get(PdfDocumentMetadataTagType::Keywords)
                .map(|t| t.value().to_string()),
        }
        .sanitized();

        Ok(PdfDocumentInfo {
            page_count,
            pages: page_infos,
            metadata,
            source_bytes,
        })
    }

    /// Document information gathered when the file was opened.
    pub fn info(&self) -> &PdfDocumentInfo {
        &self.info
    }

    /// Which PDFium library this renderer bound to.
    pub fn resolved_library(&self) -> &ResolvedLibrary {
        &self.resolved_library
    }

    /// Renders one zero-indexed page at `dpi` onto an opaque white
    /// background, in its **visible** orientation (page rotation applied).
    ///
    /// The returned image is an ordinary `image::DynamicImage`; PDFium
    /// resources for the page are released before this function returns.
    pub fn render_page(&self, index: u32, dpi: u16) -> Result<DynamicImage> {
        let page_number = index + 1;
        let page_info = self.info.pages.get(index as usize).ok_or_else(|| {
            CoreError::InvalidParameter(format!(
                "page {page_number} is out of range; the document has {} pages",
                self.info.page_count
            ))
        })?;

        // Compute and range-check the target size before asking PDFium to
        // allocate anything.
        let (width, height) = page_info.geometry.pixel_size(dpi)?;

        let document = Self::load_document(self.pdfium, &self.path, self.password.as_deref())?;
        let pages = document.pages();
        let page = pages
            .get(index as i32)
            .map_err(|e| CoreError::RenderFailed {
                page_number,
                reason: format!("could not access page: {e}"),
            })?;

        let config = PdfRenderConfig::new()
            .set_target_size(width as i32, height as i32)
            // Render the page in its visible orientation. PDFium applies
            // the page's own /Rotate; no additional rotation is requested.
            .render_form_data(false);

        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| CoreError::RenderFailed {
                page_number,
                reason: e.to_string(),
            })?;

        let rendered = bitmap
            .as_image()
            .map_err(|e| CoreError::RenderFailed {
                page_number,
                reason: format!("could not convert the rendered bitmap: {e}"),
            })?
            .into_rgb8();

        // Composite is unnecessary: PDFium renders onto an opaque white
        // background by default and `into_rgb8` drops any alpha channel,
        // so the result is already opaque. Converting here (rather than
        // later) also releases the PDFium bitmap at the end of this scope.
        Ok(DynamicImage::ImageRgb8(rendered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_options_default_to_no_password_and_no_system_library() {
        let options = PdfOpenOptions::default();
        assert!(options.password.is_none());
        assert!(options.pdfium.library_path.is_none());
        assert!(!options.pdfium.allow_system_library);
    }

    #[test]
    fn opening_a_missing_file_reports_a_clear_error_without_panicking() {
        // With no PDFium available this fails at library resolution; with
        // one available it fails at file access. Either way it must be a
        // structured error, never a panic.
        let options = PdfOpenOptions::default();
        let result = PdfRenderer::open(Path::new("/nonexistent/never.pdf"), &options);
        assert!(result.is_err());
    }
}
