//! The persistent PDF document session.
//!
//! # Source snapshot policy: open-bytes snapshot
//!
//! [`PdfDocumentSession::open`] reads the entire source file into memory
//! **once**, then loads PDFium from that owned buffer via
//! [`Pdfium::load_pdf_from_byte_vec`], which takes ownership of the bytes
//! and keeps them alive for as long as the resulting document. Every page
//! rendered afterwards is therefore served from the *same* in-memory
//! snapshot:
//!
//! * a modification to the file at `path` made after `open` returns
//!   cannot affect an in-progress `inspect`, `analyze`, `preview`, or
//!   `process` operation — there is no time-of-check/time-of-use gap,
//!   because there is no second check of the filesystem;
//! * the source is read from disk exactly **once** per operation, not
//!   once per page, which is what made a long book expensive before.
//!
//! Output validation (`crate::validation`) opens the *completed output* as
//! its own, separate document session. That is a different file and a
//! different operation, so it is not a violation of "one session per
//! operation" — it is a second operation.
//!
//! ## Why this is safe without `unsafe` code
//!
//! `pdfium_render::Pdfium::load_pdf_from_byte_vec(&self, bytes: Vec<u8>, ..)
//! -> Result<PdfDocument<'_>>` ties the returned document's lifetime to
//! the **`Pdfium` binding**, not to the byte buffer (which the document
//! takes ownership of internally, via `set_source_byte_buffer`) and not to
//! any local variable in the caller. This crate's `Pdfium` binding is a
//! process-wide `&'static Pdfium` (see `crate::pdfium_backend`), so the
//! elided lifetime on `PdfDocument<'_>` resolves to `'static`:
//! `PdfDocument<'static>` can be stored directly as an ordinary, movable
//! struct field. No self-referential struct, no raw pointers, no lifetime
//! transmutation, and no `unsafe` block are needed anywhere in this
//! module.
//!
//! ## Memory model
//!
//! Because the whole source file is held in memory for the session's
//! lifetime, the honest bound during a `process` or `analyze` run is:
//!
//! > source PDF bytes (held once, for the whole operation)
//! > + one uncompressed working raster page
//! > + algorithm working buffers
//! > + the growing compressed output PDF (for `process`)
//!
//! This is **not** O(1) in source file size: baseline memory now grows
//! with the *input* size as well as the output. Milestone 2's claim of a
//! single bounded working page was correct for that milestone's per-page
//! reopen design; it no longer describes this crate and is corrected in
//! `docs/pdf-pipeline-session.md` and `docs/limitations.md`.

use std::path::Path;

use image::DynamicImage;
use pdfium_render::prelude::*;

use crate::document::{PdfDocumentInfo, PdfMetadata, PdfPageInfo};
use crate::error::{CoreError, Result};
use crate::page_geometry::{PageGeometry, PageRotation};
use crate::pdfium_backend::{self, PdfiumConfig, ResolvedLibrary};
use crate::source_identity::SourceIdentity;

/// Options for opening a source document.
#[derive(Debug, Clone, Default)]
pub struct PdfOpenOptions {
    /// Password for an encrypted document. Never logged or reported.
    pub password: Option<String>,
    pub pdfium: PdfiumConfig,
    /// Whether to compute [`SourceIdentity::content_sha256`] for this
    /// open. Off by default: hashing a large scanned book is not free,
    /// and most operations don't need it. See
    /// [`crate::source_identity::SourceIdentity`].
    pub compute_source_hash: bool,
}

/// Text recovered from a PDF's native text layer.  This is deliberately an
/// owned, backend-neutral value: PDFium handles never cross the session
/// boundary or enter a persisted MDP record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeTextPage {
    pub text: String,
}

/// A project-owned seam between the pipeline and the concrete PDF backend.
///
/// Exists so `process`/`analyze` can be tested with an in-memory mock
/// document — proving the pipeline calls `render_page` on the *one*
/// session it was given, rather than reopening the source — without
/// requiring a real PDFium library in ordinary (non-`#[ignore]`) tests.
/// No `pdfium-render` type appears in this trait's signature.
pub trait DocumentSession {
    fn info(&self) -> &PdfDocumentInfo;
    fn source_identity(&self) -> &SourceIdentity;
    /// A human-readable description of the PDFium library this session
    /// is using, for reports and progress output. A mock session used in
    /// tests may return any descriptive string; it never has to
    /// construct a real [`ResolvedLibrary`].
    fn pdfium_library_description(&self) -> String;
    /// Renders one zero-indexed page at `dpi`. Implementations must serve
    /// this from the single session opened for the whole operation —
    /// never by reopening the source document.
    fn render_page(&self, index: u32, dpi: u16) -> Result<DynamicImage>;

    /// Extracts the native text layer for one page.  Test sessions may use
    /// the default empty result; an empty or suspicious result is routed to
    /// the local OCR provider by the M3 pipeline.
    fn native_text(&self, _index: u32) -> Result<NativeTextPage> {
        Ok(NativeTextPage::default())
    }
}

/// One PDFium document, opened once from an in-memory snapshot of the
/// source file (see the module documentation for the snapshot policy),
/// and reused for every page rendered during the operation.
pub struct PdfDocumentSession {
    document: PdfDocument<'static>,
    info: PdfDocumentInfo,
    resolved_library: ResolvedLibrary,
    source_identity: SourceIdentity,
}

impl PdfDocumentSession {
    /// Opens `path`: reads it into memory once, binds PDFium according to
    /// `options`, and loads the document from that owned snapshot. The
    /// document, its page geometry, and its metadata are all read exactly
    /// once here; nothing about `path` is consulted again afterwards.
    pub fn open(path: &Path, options: &PdfOpenOptions) -> Result<Self> {
        let session = pdfium_backend::session(&options.pdfium)?;
        let pdfium: &'static Pdfium = &session.pdfium;
        let resolved_library = session.resolved.clone();

        let bytes = std::fs::read(path).map_err(|e| CoreError::io(path, e))?;
        let source_identity = SourceIdentity::capture(path, &bytes, options.compute_source_hash)?;

        let document = pdfium
            .load_pdf_from_byte_vec(bytes, options.password.as_deref())
            .map_err(|e| {
                // Distinguish a password problem from a generic open
                // failure where PDFium tells us enough to do so.
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
            })?;

        let info = describe(&document, source_identity.byte_len)?;

        Ok(Self {
            document,
            info,
            resolved_library,
            source_identity,
        })
    }

    /// Document information gathered when the file was opened.
    pub fn info(&self) -> &PdfDocumentInfo {
        &self.info
    }

    /// Which PDFium library this session bound to.
    pub fn resolved_library(&self) -> &ResolvedLibrary {
        &self.resolved_library
    }

    /// Identity of the exact byte snapshot this session opened.
    pub fn source_identity(&self) -> &SourceIdentity {
        &self.source_identity
    }

    /// Renders one zero-indexed page at `dpi` onto an opaque white
    /// background, in its **visible** orientation (page rotation already
    /// applied by PDFium). Served directly from the document opened by
    /// [`PdfDocumentSession::open`] — this never reopens or reparses the
    /// source, no matter how many times it is called.
    ///
    /// The returned image is an ordinary `image::DynamicImage`; PDFium's
    /// per-page resources are released before this function returns.
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

        let pages = self.document.pages();
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

impl DocumentSession for PdfDocumentSession {
    fn info(&self) -> &PdfDocumentInfo {
        PdfDocumentSession::info(self)
    }

    fn source_identity(&self) -> &SourceIdentity {
        PdfDocumentSession::source_identity(self)
    }

    fn pdfium_library_description(&self) -> String {
        pdfium_backend::describe_resolved(self.resolved_library())
    }

    fn render_page(&self, index: u32, dpi: u16) -> Result<DynamicImage> {
        PdfDocumentSession::render_page(self, index, dpi)
    }

    fn native_text(&self, index: u32) -> Result<NativeTextPage> {
        let page =
            self.document.pages().get(index as i32).map_err(|e| {
                CoreError::InvalidDocument(format!("could not access page text: {e}"))
            })?;
        let text = page
            .text()
            .map_err(|e| CoreError::InvalidDocument(format!("could not extract page text: {e}")))?
            .all();
        Ok(NativeTextPage { text })
    }
}

/// Collects page geometry and metadata for the whole document. Called
/// exactly once, from [`PdfDocumentSession::open`].
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

        // Page box policy: PDFium's `width()`/`height()` already report
        // the effective visible box (CropBox when present and valid,
        // otherwise MediaBox). See docs/pdf-output.md.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_options_default_to_no_password_no_system_library_no_hash() {
        let options = PdfOpenOptions::default();
        assert!(options.password.is_none());
        assert!(options.pdfium.library_path.is_none());
        assert!(!options.pdfium.allow_system_library);
        assert!(!options.compute_source_hash);
    }

    #[test]
    fn opening_a_missing_file_reports_a_clear_error_without_panicking() {
        // With no PDFium available this fails at library resolution; with
        // one available it fails at file access. Either way it must be a
        // structured error, never a panic.
        let options = PdfOpenOptions::default();
        let result = PdfDocumentSession::open(Path::new("/nonexistent/never.pdf"), &options);
        assert!(result.is_err());
    }

    /// A `PdfDocumentSession` boxed as `dyn DocumentSession` must still
    /// type-check and be object-safe: the pipeline depends on the trait,
    /// not on the concrete PDFium-backed type.
    #[test]
    fn document_session_trait_is_object_safe() {
        fn assert_object_safe(_: &dyn DocumentSession) {}
        // Compile-time check only; not calling it with a real value here
        // avoids requiring PDFium in an ordinary test.
        let _ = assert_object_safe;
    }
}
