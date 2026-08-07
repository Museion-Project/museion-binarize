//! The single orchestrator that turns one rendered page into a packed
//! bilevel image, plus the measurements `analyze` and `process` reports
//! are built from.
//!
//! This module owns the *order* of the image-processing stages. It
//! contains no algorithm logic of its own: every step delegates to the
//! existing modules, so there is exactly one implementation of each
//! algorithm in the crate. It also owns the *one* place binarization
//! actually runs, so a report can describe what happened without ever
//! recomputing a page.

use image::{DynamicImage, GrayImage};
use serde::{Deserialize, Serialize};

use crate::bilevel::BilevelImage;
use crate::binarization::{self, SauvolaParams};
use crate::cleanup;
use crate::error::{CoreError, Result};
use crate::grayscale;
use crate::preprocessing;
use crate::settings::{BinarizationMethod, ProcessingSettings};
use crate::timing::{timed, StageDurations};

/// Which threshold was used for a page, and its value where one exists.
///
/// Otsu and manual thresholding have one meaningful scalar value; Sauvola
/// does not — it computes a different threshold per pixel from the local
/// window, so this variant reports its *configuration* instead of
/// inventing a single number that would misrepresent an adaptive method
/// as if it were global. The variant name itself (serialized as `method`)
/// is what tells a report reader whether `threshold` means anything.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ThresholdInfo {
    /// The threshold Otsu's method actually selected for this page.
    Otsu { threshold: u8 },
    /// The threshold the caller configured.
    Manual { threshold: u8 },
    /// Sauvola is local/adaptive: there is no single threshold value.
    /// These are the configuration parameters that were applied.
    Sauvola {
        window_size: u32,
        k: f32,
        dynamic_range: f32,
    },
}

/// Grayscale sample statistics for one page, computed once (immediately
/// after grayscale conversion, contrast, and preprocessing) and reused by
/// both the pipeline and any report — the page is never re-scanned to
/// produce these numbers.
///
/// `mean`/`std_dev` use numerically safe 64-bit accumulation: a 400
/// megapixel page would overflow a 32-bit accumulator.
/// `std_dev` is the **population** standard deviation (divide by N, not
/// N-1), which is the appropriate convention for a full pixel population
/// rather than a sample drawn from one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GrayscaleStats {
    pub min: u8,
    pub max: u8,
    pub mean: f64,
    pub std_dev: f64,
    pub pixel_count: u64,
}

impl GrayscaleStats {
    /// Computes statistics over every pixel of `image`. Returns a
    /// structured error for a zero-pixel image rather than dividing by
    /// zero or fabricating a mean.
    pub fn compute(image: &GrayImage) -> Result<Self> {
        let pixel_count = u64::from(image.width()) * u64::from(image.height());
        if pixel_count == 0 {
            return Err(CoreError::InvalidParameter(
                "cannot compute grayscale statistics for a zero-pixel image".to_string(),
            ));
        }

        let mut min = u8::MAX;
        let mut max = 0u8;
        let mut sum: u64 = 0;
        let mut sum_of_squares: f64 = 0.0;
        for pixel in image.pixels() {
            let value = pixel[0];
            min = min.min(value);
            max = max.max(value);
            sum += u64::from(value);
            let v = f64::from(value);
            sum_of_squares += v * v;
        }

        let n = pixel_count as f64;
        let mean = sum as f64 / n;
        // Population variance via E[X^2] - (E[X])^2. Clamp at zero: a
        // uniform image should be exactly 0.0, but the two-term formula
        // can land a hair below zero due to floating-point cancellation.
        let variance = (sum_of_squares / n - mean * mean).max(0.0);
        let std_dev = variance.sqrt();

        Ok(Self {
            min,
            max,
            mean,
            std_dev,
            pixel_count,
        })
    }
}

/// Pixel-count measurements of the final bilevel page.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InkStats {
    pub black_pixels: u64,
    pub white_pixels: u64,
    /// `black_pixels / (black_pixels + white_pixels)`. Never `NaN`: an
    /// empty image is rejected earlier, at [`GrayscaleStats::compute`],
    /// before this can be computed from zero pixels.
    pub black_pixel_ratio: f64,
}

/// Everything one call to [`process_rendered_page`] produces: the packed
/// bilevel image plus the measurements taken while producing it, so
/// `analyze` and `process` reporting never need to recompute or
/// re-inspect intermediate buffers the CLI has no business reaching into.
///
/// `stage_durations` covers only the stages this module measures
/// (grayscale/contrast/preprocessing combined, binarization, cleanup);
/// the caller fills in `render_us`, `ccitt_encode_us`, and `total_us`,
/// which happen outside this function.
#[derive(Debug, Clone, PartialEq)]
pub struct PageProcessingResult {
    pub bilevel: BilevelImage,
    pub grayscale_stats: GrayscaleStats,
    pub threshold: ThresholdInfo,
    pub ink_stats: InkStats,
    pub stage_durations: StageDurations,
}

/// Runs the full image-processing chain for one rendered page.
///
/// Order (documented and enforced by tests):
///
/// 1. validate settings;
/// 2. convert the opaque rendered image to 8-bit grayscale;
/// 3. apply contrast;
/// 4. apply preprocessing (background normalization, then median denoise);
/// 5. compute grayscale statistics (on exactly the buffer binarization
///    will read);
/// 6. binarize with the configured method, capturing threshold
///    information as a side effect rather than a second pass;
/// 7. apply despeckle cleanup;
/// 8. pack into a [`BilevelImage`] and count ink pixels.
///
/// Settings are validated *before* any pixel work, so an invalid
/// configuration fails fast rather than after an expensive render.
pub fn process_rendered_page(
    image: &DynamicImage,
    settings: &ProcessingSettings,
) -> Result<PageProcessingResult> {
    settings.validate()?;

    let (gray, grayscale_prep_us) = timed(|| {
        let gray = grayscale::to_grayscale(image);
        let gray = grayscale::adjust_contrast(&gray, settings.contrast);
        preprocessing::apply_preprocessing(&gray, &settings.preprocessing)
    });

    let grayscale_stats = GrayscaleStats::compute(&gray)?;

    let (binarized, binarization_us) = timed(|| binarize_with_info(&gray, settings.method));
    let (mask, threshold) = binarized?;
    // `gray` is no longer needed once the mask exists; dropping it here
    // keeps only one large working buffer alive during cleanup.
    drop(gray);

    let (mask, cleanup_us) = timed(|| cleanup::despeckle(&mask, &settings.cleanup));

    let black_pixels = mask.pixels.iter().filter(|&&p| p).count() as u64;
    let white_pixels = mask.pixels.len() as u64 - black_pixels;
    let black_pixel_ratio = black_pixels as f64 / mask.pixels.len() as f64;

    let bilevel = BilevelImage::from_mask(&mask);

    Ok(PageProcessingResult {
        bilevel,
        grayscale_stats,
        threshold,
        ink_stats: InkStats {
            black_pixels,
            white_pixels,
            black_pixel_ratio,
        },
        stage_durations: StageDurations {
            grayscale_prep_us,
            binarization_us,
            cleanup_us,
            ..Default::default()
        },
    })
}

/// Runs the configured binarization method once, returning both the mask
/// and a description of the threshold that was used — never running the
/// algorithm twice to additionally produce a report.
fn binarize_with_info(
    gray: &GrayImage,
    method: BinarizationMethod,
) -> Result<(crate::bilevel::BinaryMask, ThresholdInfo)> {
    match method {
        BinarizationMethod::Otsu => {
            let (mask, threshold) = binarization::binarize_otsu(gray);
            Ok((mask, ThresholdInfo::Otsu { threshold }))
        }
        BinarizationMethod::Manual { threshold } => Ok((
            binarization::binarize_manual(gray, threshold),
            ThresholdInfo::Manual { threshold },
        )),
        BinarizationMethod::Sauvola(SauvolaParams {
            window_size,
            k,
            dynamic_range,
        }) => {
            let mask = binarization::binarize_sauvola(
                gray,
                &SauvolaParams {
                    window_size,
                    k,
                    dynamic_range,
                },
            )?;
            Ok((
                mask,
                ThresholdInfo::Sauvola {
                    window_size,
                    k,
                    dynamic_range,
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::{CleanupSettings, DespeckleLevel};
    use crate::settings::PreprocessingSettings;
    use image::{Luma, Rgb, RgbImage};

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
        let mut rgb = RgbImage::from_pixel(40, 30, Rgb([245, 245, 245]));
        for y in 10..20 {
            for x in 10..30 {
                rgb.put_pixel(x, y, Rgb([20, 20, 20]));
            }
        }
        DynamicImage::ImageRgb8(rgb)
    }

    #[test]
    fn honors_otsu() {
        let out = process_rendered_page(&test_page(), &base_settings(BinarizationMethod::Otsu))
            .expect("otsu should succeed");
        assert_eq!((out.bilevel.width, out.bilevel.height), (40, 30));
        assert!(out.bilevel.get_pixel(15, 15));
        assert!(!out.bilevel.get_pixel(2, 2));
        assert!(matches!(out.threshold, ThresholdInfo::Otsu { .. }));
    }

    #[test]
    fn honors_manual_threshold() {
        let low = process_rendered_page(
            &test_page(),
            &base_settings(BinarizationMethod::Manual { threshold: 5 }),
        )
        .unwrap();
        assert!(low.bilevel.to_mask().pixels.iter().all(|&p| !p));

        let mid = process_rendered_page(
            &test_page(),
            &base_settings(BinarizationMethod::Manual { threshold: 128 }),
        )
        .unwrap();
        assert!(mid.bilevel.get_pixel(15, 15));
        assert!(!mid.bilevel.get_pixel(2, 2));
        assert_eq!(mid.threshold, ThresholdInfo::Manual { threshold: 128 });

        assert_ne!(low.bilevel.data, mid.bilevel.data);
    }

    #[test]
    fn honors_sauvola() {
        let settings = base_settings(BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 15,
            k: 0.20,
            dynamic_range: 128.0,
        }));
        let out = process_rendered_page(&test_page(), &settings).unwrap();
        assert!(out.bilevel.get_pixel(15, 15));
        assert_eq!(
            out.threshold,
            ThresholdInfo::Sauvola {
                window_size: 15,
                k: 0.20,
                dynamic_range: 128.0
            }
        );

        let other = base_settings(BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 3,
            k: 0.5,
            dynamic_range: 128.0,
        }));
        let out2 = process_rendered_page(&test_page(), &other).unwrap();
        assert_eq!((out2.bilevel.width, out2.bilevel.height), (40, 30));
    }

    #[test]
    fn honors_contrast() {
        let page = DynamicImage::ImageRgb8(RgbImage::from_pixel(20, 20, Rgb([124, 124, 124])));
        let flat = base_settings(BinarizationMethod::Manual { threshold: 125 });
        let mut flattened = flat;
        flattened.contrast = -0.9;

        let a = process_rendered_page(&page, &flat).unwrap();
        let b = process_rendered_page(&page, &flattened).unwrap();
        assert!(a.bilevel.get_pixel(0, 0));
        assert!(!b.bilevel.get_pixel(0, 0));
        assert_ne!(a.bilevel.data, b.bilevel.data);
    }

    #[test]
    fn honors_preprocessing_toggles() {
        let page = {
            let mut rgb = RgbImage::from_pixel(20, 20, Rgb([240, 240, 240]));
            rgb.put_pixel(10, 10, Rgb([0, 0, 0]));
            DynamicImage::ImageRgb8(rgb)
        };
        let mut off = base_settings(BinarizationMethod::Manual { threshold: 128 });
        off.preprocessing.median_denoise = false;
        let mut on = off;
        on.preprocessing.median_denoise = true;

        let without = process_rendered_page(&page, &off).unwrap();
        let with = process_rendered_page(&page, &on).unwrap();
        assert!(without.bilevel.get_pixel(10, 10));
        assert!(!with.bilevel.get_pixel(10, 10));
    }

    #[test]
    fn honors_cleanup_settings() {
        let page = {
            let mut rgb = RgbImage::from_pixel(20, 20, Rgb([240, 240, 240]));
            rgb.put_pixel(10, 10, Rgb([0, 0, 0]));
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
            .bilevel
            .get_pixel(10, 10));
        assert!(!process_rendered_page(&page, &conservative)
            .unwrap()
            .bilevel
            .get_pixel(10, 10));
    }

    #[test]
    fn produces_deterministic_output() {
        let settings = base_settings(BinarizationMethod::Sauvola(SauvolaParams::default()));
        let first = process_rendered_page(&test_page(), &settings).unwrap();
        for _ in 0..4 {
            let again = process_rendered_page(&test_page(), &settings).unwrap();
            assert_eq!(first.bilevel, again.bilevel);
            assert_eq!(first.threshold, again.threshold);
            assert_eq!(first.ink_stats, again.ink_stats);
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
        let page = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, Rgb([0, 255, 0])));
        let gray = grayscale::to_grayscale(&page);
        let luma = gray.get_pixel(0, 0)[0];
        let settings = base_settings(BinarizationMethod::Manual { threshold: luma });
        let out = process_rendered_page(&page, &settings).unwrap();
        assert!(out.bilevel.to_mask().pixels.iter().all(|&p| p));
    }

    #[test]
    fn output_dimensions_match_the_input_page() {
        let page = DynamicImage::ImageLuma8(GrayImage::from_pixel(37, 23, Luma([200])));
        let out = process_rendered_page(&page, &base_settings(BinarizationMethod::Otsu)).unwrap();
        assert_eq!(out.bilevel.width, 37);
        assert_eq!(out.bilevel.height, 23);
        assert_eq!(out.bilevel.stride, 37usize.div_ceil(8));
        assert!(out.bilevel.black_is_one);
    }

    #[test]
    fn ink_stats_count_matches_bilevel_pixels_and_ratio_is_consistent() {
        let out =
            process_rendered_page(&test_page(), &base_settings(BinarizationMethod::Otsu)).unwrap();
        let total = out.ink_stats.black_pixels + out.ink_stats.white_pixels;
        assert_eq!(
            total,
            u64::from(out.bilevel.width) * u64::from(out.bilevel.height)
        );
        assert!((0.0..=1.0).contains(&out.ink_stats.black_pixel_ratio));
        let expected_ratio = out.ink_stats.black_pixels as f64 / total as f64;
        assert!((out.ink_stats.black_pixel_ratio - expected_ratio).abs() < 1e-12);
    }

    #[test]
    fn grayscale_stats_are_computed_over_the_actual_page() {
        let page = DynamicImage::ImageLuma8(GrayImage::from_pixel(10, 10, Luma([100])));
        let stats = GrayscaleStats::compute(&page.to_luma8()).unwrap();
        assert_eq!(stats.min, 100);
        assert_eq!(stats.max, 100);
        assert_eq!(stats.mean, 100.0);
        assert_eq!(stats.std_dev, 0.0, "uniform image has zero std dev");
        assert_eq!(stats.pixel_count, 100);
    }

    #[test]
    fn grayscale_stats_reject_a_zero_pixel_image() {
        let empty = GrayImage::new(0, 0);
        assert!(GrayscaleStats::compute(&empty).is_err());
    }

    #[test]
    fn grayscale_stats_std_dev_matches_a_direct_two_pass_computation() {
        let mut img = GrayImage::new(4, 4);
        let values = [
            10u8, 250, 10, 250, 10, 250, 10, 250, 10, 250, 10, 250, 10, 250, 10, 250,
        ];
        for (i, v) in values.iter().enumerate() {
            img.put_pixel((i % 4) as u32, (i / 4) as u32, Luma([*v]));
        }
        let stats = GrayscaleStats::compute(&img).unwrap();
        let n = values.len() as f64;
        let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let variance = values
            .iter()
            .map(|&v| (f64::from(v) - mean).powi(2))
            .sum::<f64>()
            / n;
        assert!((stats.mean - mean).abs() < 1e-9);
        assert!((stats.std_dev - variance.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn stage_durations_leave_render_and_ccitt_fields_for_the_caller() {
        let out =
            process_rendered_page(&test_page(), &base_settings(BinarizationMethod::Otsu)).unwrap();
        assert_eq!(out.stage_durations.render_us, 0);
        assert_eq!(out.stage_durations.ccitt_encode_us, None);
        assert_eq!(out.stage_durations.total_us, 0);
    }
}
