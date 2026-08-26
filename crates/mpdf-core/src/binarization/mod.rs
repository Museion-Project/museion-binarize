//! Binarization algorithms: global Otsu, manual threshold, and local
//! Sauvola thresholding.
//!
//! Each algorithm converts an 8-bit grayscale page into a
//! [`crate::bilevel::BinaryMask`]. All three share the same convention:
//! dark text on a light background, i.e. a pixel becomes black (`true`)
//! when its grayscale value is at or below the threshold computed for it.

pub mod manual;
pub mod otsu;
pub mod sauvola;

pub use manual::binarize_manual;
pub use otsu::{binarize_otsu, otsu_threshold};
pub use sauvola::{binarize_sauvola, sauvola_window_for_dpi, SauvolaParams};

/// Which binarization algorithm to run, and its parameters.
///
/// Mirrors [`crate::settings::ProcessingSettings::method`]; kept as a
/// separate, minimal enum here so the binarization module has no
/// dependency on the rest of the settings surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinarizationMethod {
    Otsu,
    Sauvola(SauvolaParams),
    Manual { threshold: u8 },
}

/// Runs the selected binarization method against a grayscale image.
pub fn binarize(
    image: &image::GrayImage,
    method: BinarizationMethod,
) -> crate::error::Result<crate::bilevel::BinaryMask> {
    match method {
        BinarizationMethod::Otsu => Ok(binarize_otsu(image).0),
        BinarizationMethod::Manual { threshold } => Ok(binarize_manual(image, threshold)),
        BinarizationMethod::Sauvola(params) => binarize_sauvola(image, &params),
    }
}
