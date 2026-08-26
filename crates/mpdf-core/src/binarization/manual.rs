//! Manual (fixed) global threshold.

use image::GrayImage;

use crate::bilevel::BinaryMask;

/// Binarizes `image` against a single fixed threshold `t`.
///
/// For grayscale value `g`: `g <= t` becomes black, `g > t` becomes white.
/// This assumes dark text on a light background; there is no public
/// inversion toggle in Phase 1.
pub fn binarize_manual(image: &GrayImage, threshold: u8) -> BinaryMask {
    let (width, height) = image.dimensions();
    let mut mask = BinaryMask::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels() {
        mask.set(x, y, pixel[0] <= threshold);
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[test]
    fn threshold_boundary_is_inclusive_on_black_side() {
        let mut img = GrayImage::new(3, 1);
        img.put_pixel(0, 0, Luma([99]));
        img.put_pixel(1, 0, Luma([100]));
        img.put_pixel(2, 0, Luma([101]));
        let mask = binarize_manual(&img, 100);
        assert!(mask.get(0, 0));
        assert!(mask.get(1, 0));
        assert!(!mask.get(2, 0));
    }

    #[test]
    fn black_text_on_white_background() {
        let mut img = GrayImage::new(2, 1);
        img.put_pixel(0, 0, Luma([250])); // background
        img.put_pixel(1, 0, Luma([10])); // ink
        let mask = binarize_manual(&img, 128);
        assert!(!mask.get(0, 0));
        assert!(mask.get(1, 0));
    }

    #[test]
    fn uniform_image_all_one_color() {
        let img = GrayImage::from_pixel(4, 4, Luma([200]));
        let mask = binarize_manual(&img, 128);
        assert!(mask.pixels.iter().all(|&p| !p));
    }
}
