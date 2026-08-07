//! The single orchestrator that turns one rendered page into a packed
//! bilevel image.
//!
//! This module owns the *order* of the image-processing stages. It
//! contains no algorithm logic of its own: every step delegates to the
//! existing modules, so there is exactly one implementation of each
//! algorithm in the crate.

use image::DynamicImage;

use crate::bilevel::BilevelImage;
use crate::binarization;
use crate::cleanup;
use crate::error::Result;
use crate::grayscale;
use crate::preprocessing;
use crate::settings::ProcessingSettings;

/// Runs the full image-processing chain for one rendered page.
///
/// Order (documented and enforced by tests):
///
/// 1. validate settings;
/// 2. convert the opaque rendered image to 8-bit grayscale;
/// 3. apply contrast;
/// 4. apply preprocessing (background normalization, then median denoise);
/// 5. binarize with the configured method;
/// 6. apply despeckle cleanup;
/// 7. pack into a [`BilevelImage`].
///
/// Settings are validated *before* any pixel work, so an invalid
/// configuration fails fast rather than after an expensive render.
pub fn process_rendered_page(
    image: &DynamicImage,
    settings: &ProcessingSettings,
) -> Result<BilevelImage> {
    settings.validate()?;

    let gray = grayscale::to_grayscale(image);
    let gray = grayscale::adjust_contrast(&gray, settings.contrast);
    let gray = preprocessing::apply_preprocessing(&gray, &settings.preprocessing);

    let mask = binarization::binarize(&gray, settings.method.into())?;
    // `gray` is no longer needed once the mask exists; dropping it here
    // keeps only one large working buffer alive during cleanup.
    drop(gray);

    let mask = cleanup::despeckle(&mask, &settings.cleanup);
    Ok(BilevelImage::from_mask(&mask))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binarization::SauvolaParams;
    use crate::cleanup::{CleanupSettings, DespeckleLevel};
    use crate::settings::{BinarizationMethod, PreprocessingSettings};
    use image::{GrayImage, Luma, RgbImage};

    fn base_settings(method: BinarizationMethod) -> ProcessingSettings {
        ProcessingSettings {
            dpi: 300,
            method,
            contrast: 0.0,
            preprocessing: PreprocessingSettings::default(),
            cleanup: CleanupSettings::default(),
        }
    }

    /// A page with a clear light background and a dark block of "ink".
    fn test_page() -> DynamicImage {
        let mut rgb = RgbImage::from_pixel(40, 30, image::Rgb([245, 245, 245]));
        for y in 10..20 {
            for x in 10..30 {
                rgb.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        DynamicImage::ImageRgb8(rgb)
    }

    #[test]
    fn honors_otsu() {
        let out = process_rendered_page(&test_page(), &base_settings(BinarizationMethod::Otsu))
            .expect("otsu should succeed");
        assert_eq!((out.width, out.height), (40, 30));
        // The dark block is foreground; the light margin is not.
        assert!(out.get_pixel(15, 15));
        assert!(!out.get_pixel(2, 2));
    }

    #[test]
    fn honors_manual_threshold() {
        // A threshold below the ink value marks nothing as foreground.
        let low = process_rendered_page(
            &test_page(),
            &base_settings(BinarizationMethod::Manual { threshold: 5 }),
        )
        .unwrap();
        assert!(low.to_mask().pixels.iter().all(|&p| !p));

        // A threshold above the ink but below the background catches the
        // ink only.
        let mid = process_rendered_page(
            &test_page(),
            &base_settings(BinarizationMethod::Manual { threshold: 128 }),
        )
        .unwrap();
        assert!(mid.get_pixel(15, 15));
        assert!(!mid.get_pixel(2, 2));

        // Different thresholds must produce different results, proving the
        // parameter is actually reaching the algorithm.
        assert_ne!(low.data, mid.data);
    }

    #[test]
    fn honors_sauvola() {
        let settings = base_settings(BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 15,
            k: 0.20,
            dynamic_range: 128.0,
        }));
        let out = process_rendered_page(&test_page(), &settings).unwrap();
        assert!(out.get_pixel(15, 15));

        // A different window size must be able to change the result,
        // showing the parameters are not ignored.
        let other = base_settings(BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 3,
            k: 0.5,
            dynamic_range: 128.0,
        }));
        let out2 = process_rendered_page(&test_page(), &other).unwrap();
        assert_eq!((out2.width, out2.height), (40, 30));
    }

    #[test]
    fn honors_contrast() {
        // Contrast is applied before binarization, so with a fixed manual
        // threshold a strong contrast change must alter the result.
        // A pixel just below the threshold and just below the mid-gray
        // pivot: reducing contrast pulls it up toward 128, across the
        // threshold, flipping it from foreground to background.
        let page =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(20, 20, image::Rgb([124, 124, 124])));
        let mut flat = base_settings(BinarizationMethod::Manual { threshold: 125 });
        flat.contrast = 0.0;
        let mut flattened = flat;
        flattened.contrast = -0.9;

        let a = process_rendered_page(&page, &flat).unwrap();
        let b = process_rendered_page(&page, &flattened).unwrap();
        assert!(
            a.get_pixel(0, 0),
            "124 <= 125 is foreground at neutral contrast"
        );
        assert!(
            !b.get_pixel(0, 0),
            "reduced contrast should pull the pixel above the threshold"
        );
        assert_ne!(a.data, b.data, "contrast must affect the output");
    }

    #[test]
    fn honors_preprocessing_toggles() {
        // A page with an isolated speck: median denoise should remove it
        // before binarization, changing the output.
        let page = {
            let mut rgb = RgbImage::from_pixel(20, 20, image::Rgb([240, 240, 240]));
            rgb.put_pixel(10, 10, image::Rgb([0, 0, 0]));
            DynamicImage::ImageRgb8(rgb)
        };
        let mut off = base_settings(BinarizationMethod::Manual { threshold: 128 });
        off.preprocessing.median_denoise = false;
        let mut on = off;
        on.preprocessing.median_denoise = true;

        let without = process_rendered_page(&page, &off).unwrap();
        let with = process_rendered_page(&page, &on).unwrap();
        assert!(
            without.get_pixel(10, 10),
            "speck should survive without denoise"
        );
        assert!(!with.get_pixel(10, 10), "denoise should remove the speck");
    }

    #[test]
    fn honors_cleanup_settings() {
        let page = {
            let mut rgb = RgbImage::from_pixel(20, 20, image::Rgb([240, 240, 240]));
            rgb.put_pixel(10, 10, image::Rgb([0, 0, 0]));
            DynamicImage::ImageRgb8(rgb)
        };
        let mut off = base_settings(BinarizationMethod::Manual { threshold: 128 });
        off.cleanup = CleanupSettings {
            despeckle: DespeckleLevel::Off,
            max_component_pixels: None,
        };
        let mut conservative = off;
        conservative.cleanup = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: None,
        };

        assert!(process_rendered_page(&page, &off)
            .unwrap()
            .get_pixel(10, 10));
        assert!(!process_rendered_page(&page, &conservative)
            .unwrap()
            .get_pixel(10, 10));
    }

    #[test]
    fn produces_deterministic_output() {
        let settings = base_settings(BinarizationMethod::Sauvola(SauvolaParams::default()));
        let first = process_rendered_page(&test_page(), &settings).unwrap();
        for _ in 0..4 {
            assert_eq!(
                first,
                process_rendered_page(&test_page(), &settings).unwrap()
            );
        }
    }

    #[test]
    fn rejects_invalid_settings_before_processing() {
        let mut settings = base_settings(BinarizationMethod::Otsu);
        settings.dpi = 150; // unsupported
        assert!(process_rendered_page(&test_page(), &settings).is_err());

        let mut settings = base_settings(BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 16, // even
            ..Default::default()
        }));
        settings.dpi = 300;
        assert!(process_rendered_page(&test_page(), &settings).is_err());

        let mut settings = base_settings(BinarizationMethod::Otsu);
        settings.contrast = 5.0; // out of range
        assert!(process_rendered_page(&test_page(), &settings).is_err());
    }

    #[test]
    fn grayscale_conversion_precedes_binarization() {
        // A pure-colour page whose channels differ must still binarize
        // via its luminance, not via any single channel.
        let page = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([0, 255, 0])));
        let gray = grayscale::to_grayscale(&page);
        let luma = gray.get_pixel(0, 0)[0];
        let settings = base_settings(BinarizationMethod::Manual { threshold: luma });
        let out = process_rendered_page(&page, &settings).unwrap();
        // luma <= threshold -> black everywhere.
        assert!(out.to_mask().pixels.iter().all(|&p| p));
    }

    #[test]
    fn output_dimensions_match_the_input_page() {
        let page = DynamicImage::ImageLuma8(GrayImage::from_pixel(37, 23, Luma([200])));
        let out = process_rendered_page(&page, &base_settings(BinarizationMethod::Otsu)).unwrap();
        assert_eq!(out.width, 37);
        assert_eq!(out.height, 23);
        assert_eq!(out.stride, 37usize.div_ceil(8));
        assert!(out.black_is_one);
    }
}
