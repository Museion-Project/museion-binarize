//! Loading benchmark input/ground-truth rasters from disk.
//!
//! Both benchmark images (the degraded input raster and its pixel-exact
//! ground truth) are required to be lossless PNG — never JPEG, whose
//! quantization artifacts would corrupt exactly the fine-detail pixels a
//! fidelity benchmark cares about most.
//!
//! Ground truth additionally has a much stricter contract than a normal
//! image: it must be **exactly** two values (no anti-aliasing, no soft
//! edges) so it can be losslessly interpreted as a [`BinaryMask`]. A
//! ground-truth PNG that turns out to have any other pixel value is
//! rejected with [`CoreError::InvalidGroundTruthPolarity`] rather than
//! silently thresholded — silently thresholding "ground truth" would
//! make it not actually ground truth.

use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::benchmark::limits::{MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS};
use crate::bilevel::BinaryMask;
use crate::error::{CoreError, Result};

/// Which raw pixel value a ground-truth PNG uses for foreground (black
/// ink) pixels. The project-internal convention (`true` = black) is
/// unaffected by this — it only controls how the two allowed pixel
/// values (`0` and `255`) map onto that convention when loading a file.
///
/// Defaults to [`GroundTruthPolarity::BlackIsZero`], matching the common
/// document-image-binarization-evaluation convention (e.g. DIBCO ground
/// truth: `0` = text/foreground, `255` = background).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundTruthPolarity {
    #[default]
    BlackIsZero,
    BlackIs255,
}

fn check_dimensions(width: u32, height: u32, context: &str) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidParameter(format!(
            "{context} has zero width or height"
        )));
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(CoreError::ResourceLimitExceeded(format!(
            "{context} is {width}x{height}, exceeding the maximum dimension of {MAX_IMAGE_DIMENSION}"
        )));
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_IMAGE_PIXELS {
        return Err(CoreError::ResourceLimitExceeded(format!(
            "{context} has {pixels} pixels, exceeding the maximum of {MAX_IMAGE_PIXELS}"
        )));
    }
    Ok(())
}

fn open_png(path: &Path) -> Result<DynamicImage> {
    let lower = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if lower != "png" {
        return Err(CoreError::InvalidParameter(format!(
            "benchmark images must be lossless PNG, got '{}' at {}",
            lower,
            path.display()
        )));
    }
    image::open(path).map_err(CoreError::Image)
}

/// Loads a benchmark **input** raster (the degraded/grayscale source
/// page). Any PNG color type is accepted here — unlike ground truth, the
/// input is meant to be real (possibly noisy, possibly color) content
/// that gets run through the production grayscale/binarization pipeline,
/// which already knows how to convert it.
pub fn load_input_raster(path: &Path) -> Result<DynamicImage> {
    let image = open_png(path)?;
    check_dimensions(image.width(), image.height(), "input raster")?;
    Ok(image)
}

/// Loads a benchmark **ground-truth** raster and normalizes it to a
/// [`BinaryMask`] using this crate's `true = black` convention.
///
/// The PNG must decode to a grayscale color type (`L8`/`L16`/`La8`/`La16`
/// — no RGB/RGBA, since "is this color pixel foreground or background"
/// has no defined conversion here) with a fully opaque alpha channel if
/// one is present, and every pixel's grayscale value must be exactly `0`
/// or `255` — anything else is rejected rather than thresholded, per this
/// module's documentation above.
pub fn load_ground_truth_mask(path: &Path, polarity: GroundTruthPolarity) -> Result<BinaryMask> {
    let image = open_png(path)?;
    check_dimensions(image.width(), image.height(), "ground truth")?;

    match &image {
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_) => {}
        DynamicImage::ImageLumaA8(buf) => {
            if buf.pixels().any(|p| p[1] != 255) {
                return Err(CoreError::InvalidGroundTruthPolarity(format!(
                    "{} has a non-opaque alpha channel, which is ambiguous for ground truth",
                    path.display()
                )));
            }
        }
        DynamicImage::ImageLumaA16(buf) => {
            if buf.pixels().any(|p| p[1] != u16::MAX) {
                return Err(CoreError::InvalidGroundTruthPolarity(format!(
                    "{} has a non-opaque alpha channel, which is ambiguous for ground truth",
                    path.display()
                )));
            }
        }
        _ => {
            return Err(CoreError::InvalidGroundTruthPolarity(format!(
                "{} is a color image; ground truth must be grayscale with no ambiguous \
                 color-to-polarity conversion",
                path.display()
            )));
        }
    }

    let gray = image.to_luma8();
    let (width, height) = gray.dimensions();
    let mut mask = BinaryMask::new(width, height);
    for (x, y, pixel) in gray.enumerate_pixels() {
        let value = pixel[0];
        let is_black_pixel_value = match value {
            0 => true,
            255 => false,
            other => {
                return Err(CoreError::InvalidGroundTruthPolarity(format!(
                    "{} contains pixel value {other} at ({x},{y}); ground truth must be \
                     exactly binary (only 0 and 255)",
                    path.display()
                )));
            }
        };
        let is_black = match polarity {
            GroundTruthPolarity::BlackIsZero => is_black_pixel_value,
            GroundTruthPolarity::BlackIs255 => !is_black_pixel_value,
        };
        mask.set(x, y, is_black);
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    fn write_gray_png(
        dir: &Path,
        name: &str,
        values: &[[u8; 1]],
        width: u32,
        height: u32,
    ) -> std::path::PathBuf {
        let mut img = GrayImage::new(width, height);
        for (i, v) in values.iter().enumerate() {
            img.put_pixel((i as u32) % width, (i as u32) / width, Luma(*v));
        }
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn loads_a_strictly_binary_ground_truth() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gray_png(dir.path(), "gt.png", &[[0], [255], [255], [0]], 2, 2);
        let mask = load_ground_truth_mask(&path, GroundTruthPolarity::BlackIsZero).unwrap();
        assert!(mask.get(0, 0));
        assert!(!mask.get(1, 0));
        assert!(!mask.get(0, 1));
        assert!(mask.get(1, 1));
    }

    #[test]
    fn reversed_polarity_flips_black_and_white() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gray_png(dir.path(), "gt.png", &[[0], [255]], 2, 1);
        let default_polarity =
            load_ground_truth_mask(&path, GroundTruthPolarity::BlackIsZero).unwrap();
        let reversed = load_ground_truth_mask(&path, GroundTruthPolarity::BlackIs255).unwrap();
        assert_ne!(default_polarity.pixels, reversed.pixels);
        assert!(default_polarity.get(0, 0));
        assert!(!reversed.get(0, 0));
    }

    #[test]
    fn rejects_a_non_binary_pixel_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gray_png(dir.path(), "gt.png", &[[0], [128]], 2, 1);
        let err = load_ground_truth_mask(&path, GroundTruthPolarity::BlackIsZero).unwrap_err();
        assert!(matches!(err, CoreError::InvalidGroundTruthPolarity(_)));
    }

    #[test]
    fn rejects_a_non_png_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gt.jpg");
        std::fs::write(&path, b"not a real jpeg").unwrap();
        let err = load_ground_truth_mask(&path, GroundTruthPolarity::BlackIsZero).unwrap_err();
        assert!(matches!(err, CoreError::InvalidParameter(_)));
    }

    #[test]
    fn rejects_a_color_ground_truth() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = image::RgbImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = image::Rgb([10, 20, 30]);
        }
        let path = dir.path().join("gt.png");
        img.save(&path).unwrap();
        let err = load_ground_truth_mask(&path, GroundTruthPolarity::BlackIsZero).unwrap_err();
        assert!(matches!(err, CoreError::InvalidGroundTruthPolarity(_)));
    }

    #[test]
    fn load_input_raster_accepts_a_normal_grayscale_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gray_png(dir.path(), "in.png", &[[10], [200], [128], [64]], 2, 2);
        let image = load_input_raster(&path).unwrap();
        assert_eq!((image.width(), image.height()), (2, 2));
    }

    #[test]
    fn dimension_limits_reject_an_oversized_declared_image() {
        // We can't actually allocate a 20001x20001 PNG in a fast test, so
        // this exercises the pure dimension-check helper directly.
        assert!(check_dimensions(MAX_IMAGE_DIMENSION + 1, 10, "test").is_err());
        assert!(check_dimensions(10, 10, "test").is_ok());
    }
}
