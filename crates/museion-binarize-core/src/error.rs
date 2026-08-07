//! Error types shared across the processing core.
//!
//! Errors never carry document content or passwords: a variant may name a
//! *path* the user themselves supplied, but never page text, pixel data,
//! or credentials.

use std::path::PathBuf;

use thiserror::Error;

/// Errors that can occur while processing an image or document.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A caller-supplied parameter (threshold, window size, DPI, ...) was
    /// outside the range the algorithm can safely operate on.
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),

    /// A lower-level image-crate operation failed (decode, encode, ...).
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    /// Encoding or decoding a CCITT Group 4 bitstream failed.
    #[error("CCITT Group 4 error: {0}")]
    Ccitt(String),

    /// A filesystem operation failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// No PDFium dynamic library could be found. Lists the candidate
    /// locations that were tried so the user can act on it.
    #[error(
        "could not find a PDFium dynamic library. Tried: {}. \
         See docs/pdfium.md for how to provide one.",
        .attempted.join(", ")
    )]
    PdfiumNotFound { attempted: Vec<String> },

    /// A PDFium library was found but could not be loaded (wrong
    /// architecture, missing symbols, incompatible build, ...).
    #[error("failed to load the PDFium library at {path}: {reason}")]
    PdfiumLoadFailed { path: PathBuf, reason: String },

    /// The input PDF could not be opened.
    #[error("could not open the PDF at {path}: {reason}")]
    PdfOpenFailed { path: PathBuf, reason: String },

    /// The input PDF is encrypted and needs a password that was not
    /// supplied, or the supplied password was rejected. The password
    /// itself is never included.
    #[error("the PDF at {path} is password-protected and the supplied password was missing or incorrect")]
    PasswordRequired { path: PathBuf },

    /// The document opened but is structurally unusable (e.g. zero pages).
    #[error("invalid PDF document: {0}")]
    InvalidDocument(String),

    /// A page's physical geometry is missing, degenerate, or out of range.
    #[error("invalid page geometry: {0}")]
    InvalidPageGeometry(String),

    /// PDFium failed to rasterize a page.
    #[error("failed to render page {page_number}: {reason}")]
    RenderFailed { page_number: u32, reason: String },

    /// An operation would have exceeded a documented safety limit.
    #[error("resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),

    /// Assembling the output PDF failed.
    #[error("failed to construct the output PDF: {0}")]
    PdfConstructionFailed(String),

    /// Creating, writing, or persisting the temporary output file failed.
    #[error("temporary output file error: {0}")]
    TemporaryFile(String),

    /// The requested destination cannot be written to safely.
    #[error("destination conflict: {0}")]
    DestinationConflict(String),

    /// The completed output failed validation and was therefore discarded.
    #[error("output validation failed: {0}")]
    OutputValidationFailed(String),

    /// Processing was cancelled through the [`crate::progress::ProgressReporter`].
    #[error("processing was cancelled")]
    Cancelled,
}

impl CoreError {
    /// Convenience constructor for [`CoreError::Io`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        CoreError::Io {
            path: path.into(),
            source,
        }
    }
}

/// Convenience alias for results returned by the processing core.
pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdfium_not_found_lists_attempted_paths() {
        let err = CoreError::PdfiumNotFound {
            attempted: vec!["/a/libpdfium.dylib".into(), "/b/libpdfium.dylib".into()],
        };
        let message = err.to_string();
        assert!(message.contains("/a/libpdfium.dylib"));
        assert!(message.contains("/b/libpdfium.dylib"));
    }

    #[test]
    fn password_error_never_contains_the_password() {
        let err = CoreError::PasswordRequired {
            path: PathBuf::from("/tmp/secret.pdf"),
        };
        let message = err.to_string();
        assert!(!message.to_lowercase().contains("hunter2"));
        assert!(message.contains("password-protected"));
    }

    #[test]
    fn cancelled_is_distinguishable() {
        assert!(matches!(CoreError::Cancelled, CoreError::Cancelled));
    }
}
