//! Conservative, deterministic preprocessing applied before binarization.
//!
//! Every operation here is independently switchable and defaults to off
//! (or its most conservative setting). None of these operations may
//! synthesize or redraw strokes — only adjust illumination and remove
//! pixel-level sensor noise.

use image::{GrayImage, ImageBuffer, Luma};

use crate::settings::PreprocessingSettings;

/// Applies the preprocessing steps enabled in `settings`, in a fixed,
/// documented order: background normalization, then median denoise.
/// Background normalization runs first so that denoising operates on an
/// already-flattened image.
pub fn apply_preprocessing(image: &GrayImage, settings: &PreprocessingSettings) -> GrayImage {
    let mut out = image.clone();
    if settings.background_normalization {
        out = normalize_background(&out, settings.background_radius);
    }
    if settings.median_denoise {
        out = median_denoise_3x3(&out);
    }
    out
}

/// A 3x3 median filter. Removes isolated single-pixel sensor noise while
/// preserving edges far better than a mean/box filter. Border pixels (for
/// which a full 3x3 window is unavailable) are left unchanged.
pub fn median_denoise_3x3(image: &GrayImage) -> GrayImage {
    let (width, height) = image.dimensions();
    let mut out = image.clone();
    if width < 3 || height < 3 {
        return out;
    }

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let mut window = [0u8; 9];
            let mut i = 0;
            for dy in [-1i32, 0, 1] {
                for dx in [-1i32, 0, 1] {
                    let px = (x as i32 + dx) as u32;
                    let py = (y as i32 + dy) as u32;
                    window[i] = image.get_pixel(px, py)[0];
                    i += 1;
                }
            }
            window.sort_unstable();
            out.put_pixel(x, y, Luma([window[4]]));
        }
    }
    out
}

/// Estimates a slowly varying background via a large box filter, then
/// corrects local illumination by subtracting it back out toward a flat
/// mid-tone. This corrects uneven scanner lighting or gentle bleed-through
/// gradients without touching stroke-scale detail, since `radius` is
/// expected to be much larger than any single glyph.
pub fn normalize_background(image: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return image.clone();
    }
    let background = imageproc::filter::box_filter(image, radius, radius);
    let (width, height) = image.dimensions();
    ImageBuffer::from_fn(width, height, |x, y| {
        let p = image.get_pixel(x, y)[0] as i32;
        let b = background.get_pixel(x, y)[0] as i32;
        let corrected = p - b + 255;
        Luma([corrected.clamp(0, 255) as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_denoise_removes_single_pixel_outlier() {
        let mut img = GrayImage::from_pixel(5, 5, Luma([200]));
        img.put_pixel(2, 2, Luma([0])); // isolated dark speck
        let out = median_denoise_3x3(&img);
        assert_eq!(out.get_pixel(2, 2)[0], 200);
    }

    #[test]
    fn median_denoise_preserves_flat_regions() {
        let img = GrayImage::from_pixel(6, 6, Luma([77]));
        let out = median_denoise_3x3(&img);
        assert_eq!(img, out);
    }

    #[test]
    fn median_denoise_leaves_tiny_images_unchanged() {
        let img = GrayImage::from_pixel(2, 2, Luma([100]));
        let out = median_denoise_3x3(&img);
        assert_eq!(img, out);
    }

    #[test]
    fn background_normalization_flattens_a_gradient() {
        let width = 40;
        let height = 40;
        let mut img = GrayImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                // Slowly varying background from 150 to 220 left to right.
                let bg = 150 + (x * 70 / width);
                img.put_pixel(x, y, Luma([bg as u8]));
            }
        }
        let out = normalize_background(&img, 15);
        let left = out.get_pixel(2, 20)[0] as i32;
        let right = out.get_pixel(width - 3, 20)[0] as i32;
        assert!(
            (left - right).abs() < 20,
            "expected flattened background, got left={left} right={right}"
        );
    }

    #[test]
    fn zero_radius_is_identity() {
        let img = GrayImage::from_pixel(10, 10, Luma([90]));
        let out = normalize_background(&img, 0);
        assert_eq!(img, out);
    }

    #[test]
    fn apply_preprocessing_is_identity_when_everything_disabled() {
        let settings = PreprocessingSettings {
            median_denoise: false,
            background_normalization: false,
            ..Default::default()
        };
        let img = GrayImage::from_pixel(8, 8, Luma([50]));
        let out = apply_preprocessing(&img, &settings);
        assert_eq!(img, out);
    }
}
