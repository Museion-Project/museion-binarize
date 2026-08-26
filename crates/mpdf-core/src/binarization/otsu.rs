//! Global Otsu thresholding.
//!
//! Implemented in-house with 64-bit accumulators. An earlier version
//! delegated to `imageproc::contrast::otsu_level`, but that function
//! accumulates `t * histogram[t]` in `u32`: a full-page scan at 600 DPI
//! (roughly 35 megapixels) overflows it and panics, which is unacceptable
//! for user-supplied documents. The wrapper that existed for exactly this
//! reason made the swap a local change.

use image::GrayImage;

use super::manual::binarize_manual;
use crate::bilevel::BinaryMask;

/// Computes the Otsu threshold for `image`.
///
/// Maximizes between-class variance over the 256-bin grayscale histogram.
/// When several thresholds tie (as happens for a purely bimodal image),
/// the lowest is returned.
pub fn otsu_threshold(image: &GrayImage) -> u8 {
    let mut histogram = [0u64; 256];
    for pixel in image.pixels() {
        histogram[pixel[0] as usize] += 1;
    }

    let total_weight: u64 = histogram.iter().sum();
    if total_weight == 0 {
        return 0;
    }

    // u64 throughout: the largest possible value here is 255 * pixel
    // count, which stays far below u64::MAX for any page this crate will
    // ever render (see page_geometry::MAX_RENDERED_PIXELS).
    let total_sum: u64 = histogram
        .iter()
        .enumerate()
        .map(|(t, count)| t as u64 * count)
        .sum();

    let mut background_weight: u64 = 0;
    let mut background_sum: u64 = 0;
    let mut largest_variance = 0f64;
    let mut best_threshold = 0u8;

    for (threshold, &count) in histogram.iter().enumerate() {
        background_weight += count;
        if background_weight == 0 {
            continue;
        }
        let foreground_weight = total_weight - background_weight;
        if foreground_weight == 0 {
            break;
        }
        background_sum += threshold as u64 * count;
        let foreground_sum = total_sum - background_sum;

        let background_mean = background_sum as f64 / background_weight as f64;
        let foreground_mean = foreground_sum as f64 / foreground_weight as f64;
        let mean_difference = background_mean - foreground_mean;
        let between_class_variance =
            background_weight as f64 * foreground_weight as f64 * mean_difference * mean_difference;

        if between_class_variance > largest_variance {
            largest_variance = between_class_variance;
            best_threshold = threshold as u8;
        }
    }

    best_threshold
}

/// Binarizes `image` using global Otsu thresholding.
///
/// Returns the mask along with the threshold that was selected, so callers
/// (CLI `analyze`, desktop UI) can display it.
pub fn binarize_otsu(image: &GrayImage) -> (BinaryMask, u8) {
    let threshold = otsu_threshold(image);
    (binarize_manual(image, threshold), threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn image_from_rows(rows: &[&[u8]]) -> GrayImage {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let mut img = GrayImage::new(width, height);
        for (y, row) in rows.iter().enumerate() {
            for (x, &v) in row.iter().enumerate() {
                img.put_pixel(x as u32, y as u32, Luma([v]));
            }
        }
        img
    }

    #[test]
    fn two_clearly_separated_regions_split_between_them() {
        // Left half near-black, right half near-white.
        let mut img = GrayImage::new(20, 10);
        for y in 0..10 {
            for x in 0..20 {
                let v = if x < 10 { 10 } else { 240 };
                img.put_pixel(x, y, Luma([v]));
            }
        }
        let (mask, threshold) = binarize_otsu(&img);
        // For a purely two-valued histogram, intra-class variance is flat
        // across the whole gap between the clusters; imageproc's
        // implementation keeps the first (lowest) threshold that attains
        // the maximum, so `threshold` lands at the low cluster's value
        // rather than strictly between the clusters. What actually
        // matters is that the two clusters end up on opposite sides.
        assert!((10..240).contains(&threshold));
        for y in 0..10 {
            assert!(mask.get(0, y));
            assert!(!mask.get(19, y));
        }
    }

    #[test]
    fn does_not_overflow_on_a_large_uniform_page() {
        // Regression: the previous imageproc-based implementation
        // accumulated `t * count` in u32 and panicked with "attempt to
        // multiply with overflow" once a single histogram bin held more
        // than u32::MAX / 255 pixels (~16.8 M). A 5000x4000 all-white page
        // (20 M pixels) reproduces it.
        let img = GrayImage::from_pixel(5000, 4000, Luma([255]));
        let _ = otsu_threshold(&img);
    }

    #[test]
    fn uniform_image_does_not_panic_and_returns_a_threshold() {
        let img = GrayImage::from_pixel(8, 8, Luma([128]));
        let (_mask, _threshold) = binarize_otsu(&img);
    }

    #[test]
    fn noisy_bimodal_image_still_separates_clusters() {
        // Deterministic pseudo-noise around two clusters (30 and 220).
        let mut img = GrayImage::new(16, 16);
        let mut state: u32 = 12345;
        let mut next = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            ((state >> 16) & 0x1F) as i32 // 0..=31 jitter
        };
        for y in 0..16 {
            for x in 0..16 {
                let base = if (x + y) % 2 == 0 { 30 } else { 220 };
                let jitter = next() - 15;
                let v = (base + jitter).clamp(0, 255) as u8;
                img.put_pixel(x, y, Luma([v]));
            }
        }
        let (mask, threshold) = binarize_otsu(&img);
        // As above: the plateau's first threshold is picked, which lands
        // near the top of the low cluster rather than the midpoint. What
        // matters is that it still separates the two clusters correctly.
        assert!(threshold > 15 && threshold < 205, "threshold={threshold}");
        for y in 0..16 {
            for x in 0..16 {
                let expect_black = (x + y) % 2 == 0;
                assert_eq!(
                    mask.get(x, y),
                    expect_black,
                    "mismatch at ({x},{y}) with threshold={threshold}"
                );
            }
        }
    }

    #[test]
    fn black_text_on_white_background() {
        let img = image_from_rows(&[
            &[255, 255, 255, 255, 255],
            &[255, 0, 0, 0, 255],
            &[255, 255, 255, 255, 255],
        ]);
        let (mask, _t) = binarize_otsu(&img);
        assert!(mask.get(1, 1));
        assert!(mask.get(2, 1));
        assert!(mask.get(3, 1));
        assert!(!mask.get(0, 0));
    }

    #[test]
    fn white_text_on_black_background_still_separates() {
        let img = image_from_rows(&[&[0, 0, 0, 0, 0], &[0, 255, 255, 255, 0], &[0, 0, 0, 0, 0]]);
        let (mask, threshold) = binarize_otsu(&img);
        // Otsu just finds the split; with "black" convention (g <= t ->
        // black) the bright stroke ends up white (unset) and the dark
        // background ends up black (set). With only two distinct values
        // present, the variance plateau's first threshold is 0.
        assert!(threshold < 255);
        assert!(mask.get(0, 0));
        assert!(!mask.get(1, 1));
    }
}
