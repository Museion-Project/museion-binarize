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
    let background = box_filter(image, radius);
    let (width, height) = image.dimensions();
    ImageBuffer::from_fn(width, height, |x, y| {
        let p = image.get_pixel(x, y)[0] as i32;
        let b = background.get_pixel(x, y)[0] as i32;
        let corrected = p - b + 255;
        Luma([corrected.clamp(0, 255) as u8])
    })
}

/// Mean filter over a `(2 * radius + 1)` square window, computed with a
/// 64-bit summed-area table.
///
/// Implemented here rather than via `imageproc::filter::box_filter`,
/// which builds its integral image with `u32` accumulators and therefore
/// overflows on a full page at 400-600 DPI (255 * 35 M pixels far exceeds
/// `u32::MAX`). Windows are clipped at the image border, so edge pixels
/// average over the available neighbourhood rather than wrapping or
/// reading out of bounds.
fn box_filter(image: &GrayImage, radius: u32) -> GrayImage {
    let (width, height) = image.dimensions();
    let stride = width as usize + 1;
    let mut integral = vec![0u64; stride * (height as usize + 1)];

    for y in 0..height {
        let mut row_sum = 0u64;
        for x in 0..width {
            row_sum += u64::from(image.get_pixel(x, y)[0]);
            let idx = (y as usize + 1) * stride + (x as usize + 1);
            integral[idx] = integral[idx - stride] + row_sum;
        }
    }

    let radius = radius as i64;
    ImageBuffer::from_fn(width, height, |x, y| {
        let x0 = (x as i64 - radius).max(0) as usize;
        let y0 = (y as i64 - radius).max(0) as usize;
        let x1 = (x as i64 + radius).min(width as i64 - 1) as usize;
        let y1 = (y as i64 + radius).min(height as i64 - 1) as usize;

        let a = integral[y0 * stride + x0];
        let b = integral[y0 * stride + x1 + 1];
        let c = integral[(y1 + 1) * stride + x0];
        let d = integral[(y1 + 1) * stride + x1 + 1];
        let sum = d + a - b - c;
        let count = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;

        Luma([(sum / count) as u8])
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
    fn box_filter_averages_and_handles_borders() {
        let mut img = GrayImage::from_pixel(5, 5, Luma([0]));
        img.put_pixel(2, 2, Luma([250]));
        let out = box_filter(&img, 1);
        // Centre window is 3x3 = 9 px containing one 250 -> 27.
        assert_eq!(out.get_pixel(2, 2)[0], 27);
        // A corner clips to a 2x2 window with no bright pixel -> 0.
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        // A uniform image is unchanged.
        let flat = GrayImage::from_pixel(6, 6, Luma([80]));
        assert_eq!(box_filter(&flat, 2), flat);
    }

    #[test]
    fn box_filter_does_not_overflow_on_a_large_bright_page() {
        // Regression: imageproc's box_filter builds a u32 integral image,
        // which overflows past ~16.8 M bright pixels. This is the same
        // class of bug as the Otsu overflow.
        let img = GrayImage::from_pixel(5000, 4000, Luma([255]));
        let out = box_filter(&img, 3);
        assert_eq!(out.get_pixel(2500, 2000)[0], 255);
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
