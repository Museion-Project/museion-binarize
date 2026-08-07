//! Grayscale conversion and contrast adjustment.

use image::{DynamicImage, GrayImage, ImageBuffer, Luma};

/// Converts a rendered page image to 8-bit grayscale.
///
/// Uses `image`'s standard luminance conversion. Rendering must have
/// already composited the page onto an opaque white background (see
/// `docs/architecture.md`), so this function never has to reason about
/// alpha.
pub fn to_grayscale(image: &DynamicImage) -> GrayImage {
    image.to_luma8()
}

/// Adjusts contrast around the mid-gray point.
///
/// `contrast` is clamped to `[-1.0, 1.0]`: `0.0` leaves the image
/// unchanged, `1.0` is maximum contrast stretch, `-1.0` flattens the image
/// toward mid-gray. This is the standard brightness-preserving contrast
/// formula (pivoted at 128), applied per pixel.
pub fn adjust_contrast(image: &GrayImage, contrast: f32) -> GrayImage {
    let contrast = contrast.clamp(-1.0, 1.0);
    if contrast == 0.0 {
        return image.clone();
    }

    // Classic contrast-correction factor, with `contrast` remapped from
    // `[-1.0, 1.0]` to the `[-255, 255]` range the formula expects.
    let c = contrast * 255.0;
    let factor = (259.0 * (c + 255.0)) / (255.0 * (259.0 - c));

    ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        let p = image.get_pixel(x, y)[0] as f32;
        let v = factor * (p - 128.0) + 128.0;
        Luma([v.round().clamp(0.0, 255.0) as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[test]
    fn to_grayscale_converts_rgb() {
        let mut rgb = RgbImage::new(2, 1);
        rgb.put_pixel(0, 0, image::Rgb([255, 255, 255]));
        rgb.put_pixel(1, 0, image::Rgb([0, 0, 0]));
        let gray = to_grayscale(&DynamicImage::ImageRgb8(rgb));
        assert_eq!(gray.get_pixel(0, 0)[0], 255);
        assert_eq!(gray.get_pixel(1, 0)[0], 0);
    }

    #[test]
    fn zero_contrast_is_identity() {
        let mut img = GrayImage::new(2, 2);
        img.put_pixel(0, 0, Luma([10]));
        img.put_pixel(1, 0, Luma([200]));
        let out = adjust_contrast(&img, 0.0);
        assert_eq!(img, out);
    }

    #[test]
    fn positive_contrast_pushes_values_away_from_mid_gray() {
        let mut img = GrayImage::new(1, 1);
        img.put_pixel(0, 0, Luma([160]));
        let out = adjust_contrast(&img, 0.5);
        assert!(out.get_pixel(0, 0)[0] > 160);
    }

    #[test]
    fn negative_contrast_pulls_values_toward_mid_gray() {
        let mut img = GrayImage::new(1, 1);
        img.put_pixel(0, 0, Luma([200]));
        let out = adjust_contrast(&img, -0.5);
        let v = out.get_pixel(0, 0)[0];
        assert!(v < 200 && v > 128);
    }

    #[test]
    fn contrast_is_clamped_and_never_panics() {
        let mut img = GrayImage::new(1, 1);
        img.put_pixel(0, 0, Luma([0]));
        let _ = adjust_contrast(&img, 5.0);
        let _ = adjust_contrast(&img, -5.0);
    }
}
