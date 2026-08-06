//! Error types shared across the processing core.

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
}

/// Convenience alias for results returned by the processing core.
pub type Result<T> = std::result::Result<T, CoreError>;
