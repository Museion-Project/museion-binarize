//! Local Sauvola thresholding, implemented in-house using integral images
//! so that the local mean and standard deviation for every pixel's window
//! are computed in amortized constant time.
//!
//! `T(x,y) = m(x,y) * (1 + k * (s(x,y) / R - 1))`

use image::GrayImage;

use crate::bilevel::BinaryMask;
use crate::error::{CoreError, Result};

/// Parameters for Sauvola thresholding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SauvolaParams {
    /// Side length of the (square) local window. Must be odd.
    pub window_size: u32,
    /// Sensitivity parameter, typically in `[0.2, 0.5]`.
    pub k: f32,
    /// Dynamic range of the standard deviation, typically `128` for 8-bit
    /// grayscale input.
    pub dynamic_range: f32,
}

impl Default for SauvolaParams {
    fn default() -> Self {
        Self {
            window_size: 33,
            k: 0.20,
            dynamic_range: 128.0,
        }
    }
}

/// Derives a reasonable odd window size from the output DPI:
/// `nearest_odd(clamp(round(dpi * 0.08), 15, 75))`.
pub fn sauvola_window_for_dpi(dpi: u16) -> u32 {
    let raw = (dpi as f32 * 0.08).round();
    let clamped = raw.clamp(15.0, 75.0) as u32;
    if clamped % 2 == 0 {
        clamped + 1
    } else {
        clamped
    }
}

/// Binarizes `image` using Sauvola local thresholding.
pub fn binarize_sauvola(image: &GrayImage, params: &SauvolaParams) -> Result<BinaryMask> {
    if params.window_size == 0 || params.window_size % 2 == 0 {
        return Err(CoreError::InvalidParameter(format!(
            "Sauvola window_size must be a positive odd number, got {}",
            params.window_size
        )));
    }
    if params.dynamic_range <= 0.0 {
        return Err(CoreError::InvalidParameter(format!(
            "Sauvola dynamic_range must be positive, got {}",
            params.dynamic_range
        )));
    }

    let (width, height) = image.dimensions();
    let (sum, sq_sum) = integral_images(image);
    let half = (params.window_size / 2) as i64;

    let mut mask = BinaryMask::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let x0 = (x as i64 - half).max(0) as u32;
            let y0 = (y as i64 - half).max(0) as u32;
            let x1 = (x as i64 + half).min(width as i64 - 1) as u32;
            let y1 = (y as i64 + half).min(height as i64 - 1) as u32;

            let count = (x1 - x0 + 1) as u64 * (y1 - y0 + 1) as u64;
            let s = region_sum(&sum, width, x0, y0, x1, y1) as f64;
            let sq = region_sum(&sq_sum, width, x0, y0, x1, y1) as f64;

            let mean = s / count as f64;
            let variance = (sq / count as f64 - mean * mean).max(0.0);
            let stddev = variance.sqrt();

            let threshold =
                mean * (1.0 + params.k as f64 * (stddev / params.dynamic_range as f64 - 1.0));

            let p = image.get_pixel(x, y)[0] as f64;
            mask.set(x, y, p <= threshold);
        }
    }
    Ok(mask)
}

/// Builds summed-area tables (integral images) for `image` and its
/// pixel-squared values, using 64-bit accumulators to avoid overflow: the
/// largest supported page (see `docs/architecture.md`) has far fewer than
/// `u64::MAX / 255^2` pixels.
fn integral_images(image: &GrayImage) -> (Vec<u64>, Vec<u64>) {
    let (width, height) = image.dimensions();
    let stride = width as usize + 1;
    let mut sum = vec![0u64; stride * (height as usize + 1)];
    let mut sq_sum = vec![0u64; stride * (height as usize + 1)];

    for y in 0..height {
        let mut row_sum = 0u64;
        let mut row_sq = 0u64;
        for x in 0..width {
            let p = image.get_pixel(x, y)[0] as u64;
            row_sum += p;
            row_sq += p * p;
            let idx = (y as usize + 1) * stride + (x as usize + 1);
            let above = idx - stride;
            sum[idx] = sum[above] + row_sum;
            sq_sum[idx] = sq_sum[above] + row_sq;
        }
    }
    (sum, sq_sum)
}

/// Sums the inclusive rectangle `[x0, x1] x [y0, y1]` (0-indexed original
/// image coordinates) using a summed-area table built by
/// [`integral_images`].
fn region_sum(integral: &[u64], width: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> u64 {
    let stride = width as usize + 1;
    let a = integral[y0 as usize * stride + x0 as usize];
    let b = integral[y0 as usize * stride + (x1 as usize + 1)];
    let c = integral[(y1 as usize + 1) * stride + x0 as usize];
    let d = integral[(y1 as usize + 1) * stride + (x1 as usize + 1)];
    d + a - b - c
}

/// Reference implementation: recomputes the local mean/stddev directly for
/// every pixel with no integral-image optimization. Used only in tests to
/// cross-check [`binarize_sauvola`]'s output.
#[cfg(test)]
fn binarize_sauvola_reference(image: &GrayImage, params: &SauvolaParams) -> BinaryMask {
    let (width, height) = image.dimensions();
    let half = (params.window_size / 2) as i64;
    let mut mask = BinaryMask::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let x0 = (x as i64 - half).max(0);
            let y0 = (y as i64 - half).max(0);
            let x1 = (x as i64 + half).min(width as i64 - 1);
            let y1 = (y as i64 + half).min(height as i64 - 1);

            let mut sum = 0f64;
            let mut sq_sum = 0f64;
            let mut count = 0f64;
            for wy in y0..=y1 {
                for wx in x0..=x1 {
                    let p = image.get_pixel(wx as u32, wy as u32)[0] as f64;
                    sum += p;
                    sq_sum += p * p;
                    count += 1.0;
                }
            }
            let mean = sum / count;
            let variance = (sq_sum / count - mean * mean).max(0.0);
            let stddev = variance.sqrt();
            let threshold =
                mean * (1.0 + params.k as f64 * (stddev / params.dynamic_range as f64 - 1.0));
            let p = image.get_pixel(x, y)[0] as f64;
            mask.set(x, y, p <= threshold);
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[test]
    fn rejects_even_window_size() {
        let img = GrayImage::from_pixel(10, 10, Luma([128]));
        let params = SauvolaParams {
            window_size: 10,
            ..Default::default()
        };
        assert!(binarize_sauvola(&img, &params).is_err());
    }

    #[test]
    fn rejects_zero_dynamic_range() {
        let img = GrayImage::from_pixel(10, 10, Luma([128]));
        let params = SauvolaParams {
            dynamic_range: 0.0,
            ..Default::default()
        };
        assert!(binarize_sauvola(&img, &params).is_err());
    }

    #[test]
    fn optimized_matches_reference_implementation() {
        // Deterministic pseudo-random grayscale image, including edges
        // where the window is clipped by the image boundary.
        let (w, h) = (37, 29);
        let mut img = GrayImage::new(w, h);
        let mut state: u32 = 987654321;
        for y in 0..h {
            for x in 0..w {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                let v = ((state >> 16) & 0xFF) as u8;
                img.put_pixel(x, y, Luma([v]));
            }
        }

        for window_size in [3, 15, 31] {
            let params = SauvolaParams {
                window_size,
                ..Default::default()
            };
            let fast = binarize_sauvola(&img, &params).unwrap();
            let reference = binarize_sauvola_reference(&img, &params);
            assert_eq!(fast, reference, "mismatch for window_size={window_size}");
        }
    }

    #[test]
    fn is_deterministic_across_repeated_runs() {
        let mut img = GrayImage::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                img.put_pixel(x, y, Luma([((x * 7 + y * 13) % 256) as u8]));
            }
        }
        let params = SauvolaParams::default();
        let first = binarize_sauvola(&img, &params).unwrap();
        for _ in 0..5 {
            assert_eq!(first, binarize_sauvola(&img, &params).unwrap());
        }
    }

    #[test]
    fn uniform_image_produces_no_foreground() {
        let img = GrayImage::from_pixel(16, 16, Luma([128]));
        let mask = binarize_sauvola(&img, &SauvolaParams::default()).unwrap();
        // Zero local standard deviation everywhere -> threshold == mean *
        // (1 - k) < mean, and every pixel equals the mean, so nothing
        // crosses the threshold.
        assert!(mask.pixels.iter().all(|&p| !p));
    }

    #[test]
    fn window_for_dpi_is_always_odd_and_in_range() {
        for dpi in [72u16, 150, 300, 400, 600, 1200] {
            let w = sauvola_window_for_dpi(dpi);
            assert!(w % 2 == 1, "dpi={dpi} produced even window {w}");
            assert!(
                (15..=75).contains(&w),
                "dpi={dpi} produced out-of-range window {w}"
            );
        }
    }
}
