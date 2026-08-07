//! Global Otsu thresholding.
//!
//! The threshold search itself is delegated to `imageproc`, but wrapped
//! behind this project-owned function so the rest of the pipeline never
//! references `imageproc` directly and the implementation can be swapped
//! later without touching callers.

use image::GrayImage;

use super::manual::binarize_manual;
use crate::bilevel::BinaryMask;

/// Computes the Otsu threshold for `image`.
pub fn otsu_threshold(image: &GrayImage) -> u8 {
    imageproc::contrast::otsu_level(image)
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
