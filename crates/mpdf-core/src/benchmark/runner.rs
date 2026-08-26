//! Orchestrates a benchmark run: dataset x profile -> per-page metrics ->
//! aggregate -> report.
//!
//! [`run_raster_benchmark`] is the primary (Level A) benchmark: it runs
//! [`crate::image_pipeline::process_rendered_page`] — the exact function
//! `process`/`analyze`/`estimate` already use — directly on each page's
//! degraded input raster, with no PDFium involved at all. This isolates
//! binarization-pipeline fidelity from PDF-rendering behavior; see
//! `docs/benchmark-metrics.md`, "Benchmark levels," for why that
//! separation matters and for the PDFium-backed Level B counterpart
//! (`crates/mpdf-core/tests/pdf_pipeline.rs`, ignored by
//! default like every other PDFium-dependent test).
//!
//! Execution is single-threaded and strictly ordered (dataset order,
//! then profile order) — no `--jobs` beyond 1 in this milestone, so a
//! benchmark report's page ordering and content are reproducible without
//! having to reason about scheduling.

use std::collections::BTreeMap;

use crate::benchmark::dataset::LoadedDataset;
use crate::benchmark::mask_io;
use crate::benchmark::metrics::{self, ConfusionMatrix, Psnr};
use crate::benchmark::profile::LoadedProfile;
use crate::benchmark::report::{
    AggregateMetrics, BenchmarkLevel, BenchmarkReport, DatasetInfo, Environment, FMeasureReport,
    PageBenchmarkResult, PageExtreme, ProfileInfo, RoiResult, RunReport, WorstPages,
    METRICS_SCHEMA_VERSION,
};
use crate::ccitt;
use crate::error::Result;
use crate::image_pipeline::process_rendered_page;
use crate::settings::ProcessingSettings;
use crate::timing::timed;

/// Runs every `[[runs]]` in `profile` against every page in `dataset`,
/// using the raster (Level A) benchmark. `pdfium_library` is always
/// `None` here (the raster level never touches PDFium); it exists so the
/// same [`Environment`] shape is reused verbatim by a future PDFium-backed
/// Level B runner rather than duplicating the type.
pub fn run_raster_benchmark(
    dataset: &LoadedDataset,
    profile: &LoadedProfile,
) -> Result<BenchmarkReport> {
    let mut runs = Vec::with_capacity(profile.manifest.runs.len());
    for run_config in &profile.manifest.runs {
        let settings = run_config.to_processing_settings()?;
        let method_name = method_label(&settings);
        let pages = run_one_page_set(dataset, &settings)?;
        let aggregate = aggregate_pages(pages.iter().collect::<Vec<_>>().as_slice());
        let category_aggregates = aggregate_by_category(&pages);
        let roi_tag_aggregates = aggregate_by_roi_tag(&pages);
        let worst_pages = worst_pages(&pages);

        runs.push(RunReport {
            run_id: run_config.id.clone(),
            dpi: settings.dpi,
            method: method_name,
            pages,
            aggregate,
            category_aggregates,
            roi_tag_aggregates,
            worst_pages,
        });
    }

    Ok(BenchmarkReport {
        metrics_schema_version: METRICS_SCHEMA_VERSION.to_string(),
        dataset: DatasetInfo {
            id: dataset.manifest.id.clone(),
            title: dataset.manifest.title.clone(),
            manifest_digest: dataset.manifest_digest(),
            page_count: dataset.manifest.pages.len(),
        },
        profile: ProfileInfo {
            id: profile.manifest.id.clone(),
            digest: profile.digest(),
        },
        environment: Environment {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            pdfium_library: None,
            benchmark_level: BenchmarkLevel::Raster,
        },
        runs,
    })
}

fn method_label(settings: &ProcessingSettings) -> String {
    use crate::settings::BinarizationMethod;
    match settings.method {
        BinarizationMethod::Otsu => "otsu".to_string(),
        BinarizationMethod::Sauvola(_) => "sauvola".to_string(),
        BinarizationMethod::Manual { .. } => "manual".to_string(),
    }
}

fn run_one_page_set(
    dataset: &LoadedDataset,
    settings: &ProcessingSettings,
) -> Result<Vec<PageBenchmarkResult>> {
    let mut results = Vec::with_capacity(dataset.manifest.pages.len());
    for (page, (input_path, gt_path)) in dataset.manifest.pages.iter().zip(&dataset.resolved_paths)
    {
        let input = mask_io::load_input_raster(input_path)?;
        let ground_truth =
            mask_io::load_ground_truth_mask(gt_path, dataset.manifest.ground_truth_polarity)?;

        let processed = process_rendered_page(&input, settings)?;
        let output_mask = processed.bilevel.to_mask();

        let (ccitt_bytes_vec, ccitt_encode_us) = timed(|| ccitt::encode_g4(&processed.bilevel));
        let ccitt_bytes = ccitt_bytes_vec.len() as u64;

        let confusion = metrics::confusion_matrix(&output_mask, &ground_truth)?;
        let f = metrics::f_measure(&confusion);
        let psnr_value = metrics::psnr(&output_mask, &ground_truth)?;
        let drd_value = metrics::drd(&output_mask, &ground_truth)?;

        let pixel_count = u64::from(output_mask.width) * u64::from(output_mask.height);
        let bytes_per_megapixel = ccitt_bytes as f64 / (pixel_count as f64 / 1_000_000.0);

        let black_pixel_ratio_ground_truth = {
            let black = ground_truth.pixels.iter().filter(|&&p| p).count() as f64;
            black / ground_truth.pixels.len() as f64
        };

        let processing_duration_us = processed.stage_durations.grayscale_prep_us
            + processed.stage_durations.binarization_us
            + processed.stage_durations.cleanup_us
            + ccitt_encode_us;

        let roi_results = page
            .rois
            .iter()
            .map(|roi| -> Result<RoiResult> {
                roi.validate(output_mask.width, output_mask.height)?;
                let cropped_output = roi.crop(&output_mask);
                let cropped_gt = roi.crop(&ground_truth);
                let roi_confusion = metrics::confusion_matrix(&cropped_output, &cropped_gt)?;
                let roi_f = metrics::f_measure(&roi_confusion);
                let roi_psnr = metrics::psnr(&cropped_output, &cropped_gt)?;
                let roi_drd = metrics::drd(&cropped_output, &cropped_gt)?;
                Ok(RoiResult {
                    roi_id: roi.id.clone(),
                    tag: roi.tag.clone(),
                    f_measure: roi_f.into(),
                    psnr: roi_psnr,
                    drd: roi_drd,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        results.push(PageBenchmarkResult {
            page_id: page.id.clone(),
            category: page.category.clone(),
            tags: page.tags.clone(),
            width: output_mask.width,
            height: output_mask.height,
            pixel_count,
            f_measure: f.into(),
            psnr: psnr_value,
            drd: drd_value,
            black_pixel_ratio_output: processed.ink_stats.black_pixel_ratio,
            black_pixel_ratio_ground_truth,
            ccitt_bytes,
            bytes_per_megapixel,
            render_duration_us: None, // no PDFium render stage at this level.
            processing_duration_us,
            roi_results,
        });
    }
    Ok(results)
}

fn aggregate_pages(pages: &[&PageBenchmarkResult]) -> AggregateMetrics {
    let page_count = pages.len();
    if page_count == 0 {
        return AggregateMetrics {
            page_count: 0,
            macro_f1: 0.0,
            micro_f1: 0.0,
            mean_precision: 0.0,
            mean_recall: 0.0,
            mean_psnr_db: None,
            perfect_psnr_page_count: 0,
            mean_drd: 0.0,
            median_drd: 0.0,
            max_drd: 0.0,
            mean_bytes_per_megapixel: Some(0.0),
            median_processing_duration_us: None,
            total_processing_duration_us: None,
        };
    }

    let macro_f1 = mean(pages.iter().map(|p| p.f_measure.f1));
    let mean_precision = mean(pages.iter().map(|p| p.f_measure.precision));
    let mean_recall = mean(pages.iter().map(|p| p.f_measure.recall));

    let mut total_confusion = ConfusionMatrix {
        true_positive: 0,
        false_positive: 0,
        false_negative: 0,
        true_negative: 0,
    };
    for p in pages {
        // Saturating, not wrapping: cross-page confusion totals stay
        // bounded well under u64::MAX given this crate's dataset/image
        // size limits (see `benchmark::limits`), but a silent wraparound
        // would be a worse failure mode than an inflated-but-safe
        // saturated total.
        total_confusion.true_positive = total_confusion
            .true_positive
            .saturating_add(p.f_measure.confusion.true_positive);
        total_confusion.false_positive = total_confusion
            .false_positive
            .saturating_add(p.f_measure.confusion.false_positive);
        total_confusion.false_negative = total_confusion
            .false_negative
            .saturating_add(p.f_measure.confusion.false_negative);
        total_confusion.true_negative = total_confusion
            .true_negative
            .saturating_add(p.f_measure.confusion.true_negative);
    }
    let micro_f1 = metrics::f_measure(&total_confusion).f1;

    let finite_psnr: Vec<f64> = pages.iter().filter_map(|p| p.psnr.db()).collect();
    let perfect_psnr_page_count = pages.iter().filter(|p| p.psnr.is_perfect_match()).count();
    let mean_psnr_db = if finite_psnr.is_empty() {
        None
    } else {
        Some(mean(finite_psnr.iter().copied()))
    };

    let mut drd_values: Vec<f64> = pages.iter().map(|p| p.drd).collect();
    let mean_drd = mean(drd_values.iter().copied());
    drd_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_drd = median_of_sorted(&drd_values);
    let max_drd = drd_values.last().copied().unwrap_or(0.0);

    let mean_bytes_per_megapixel = mean(pages.iter().map(|p| p.bytes_per_megapixel));

    let mut durations: Vec<u64> = pages.iter().map(|p| p.processing_duration_us).collect();
    durations.sort_unstable();
    let median_processing_duration_us = Some(median_of_sorted_u64(&durations));
    let total_processing_duration_us = Some(durations.iter().sum());

    AggregateMetrics {
        page_count,
        macro_f1,
        micro_f1,
        mean_precision,
        mean_recall,
        mean_psnr_db,
        perfect_psnr_page_count,
        mean_drd,
        median_drd,
        max_drd,
        mean_bytes_per_megapixel: Some(mean_bytes_per_megapixel),
        median_processing_duration_us,
        total_processing_duration_us,
    }
}

fn aggregate_by_category(pages: &[PageBenchmarkResult]) -> BTreeMap<String, AggregateMetrics> {
    let mut grouped: BTreeMap<String, Vec<&PageBenchmarkResult>> = BTreeMap::new();
    for page in pages {
        grouped.entry(page.category.clone()).or_default().push(page);
    }
    grouped
        .into_iter()
        .map(|(category, group)| (category, aggregate_pages(&group)))
        .collect()
}

fn aggregate_by_roi_tag(pages: &[PageBenchmarkResult]) -> BTreeMap<String, AggregateMetrics> {
    let mut grouped: BTreeMap<String, Vec<(FMeasureReport, Psnr, f64)>> = BTreeMap::new();
    for page in pages {
        for roi in &page.roi_results {
            if let Some(tag) = &roi.tag {
                grouped
                    .entry(tag.clone())
                    .or_default()
                    .push((roi.f_measure, roi.psnr, roi.drd));
            }
        }
    }
    grouped
        .into_iter()
        .map(|(tag, items)| (tag, aggregate_fmeasure_triples(&items)))
        .collect()
}

fn aggregate_fmeasure_triples(items: &[(FMeasureReport, Psnr, f64)]) -> AggregateMetrics {
    let page_count = items.len();
    if page_count == 0 {
        return AggregateMetrics {
            page_count: 0,
            macro_f1: 0.0,
            micro_f1: 0.0,
            mean_precision: 0.0,
            mean_recall: 0.0,
            mean_psnr_db: None,
            perfect_psnr_page_count: 0,
            mean_drd: 0.0,
            median_drd: 0.0,
            max_drd: 0.0,
            mean_bytes_per_megapixel: None,
            median_processing_duration_us: None,
            total_processing_duration_us: None,
        };
    }

    let macro_f1 = mean(items.iter().map(|(f, _, _)| f.f1));
    let mean_precision = mean(items.iter().map(|(f, _, _)| f.precision));
    let mean_recall = mean(items.iter().map(|(f, _, _)| f.recall));

    let mut total_confusion = ConfusionMatrix {
        true_positive: 0,
        false_positive: 0,
        false_negative: 0,
        true_negative: 0,
    };
    for (f, _, _) in items {
        total_confusion.true_positive = total_confusion
            .true_positive
            .saturating_add(f.confusion.true_positive);
        total_confusion.false_positive = total_confusion
            .false_positive
            .saturating_add(f.confusion.false_positive);
        total_confusion.false_negative = total_confusion
            .false_negative
            .saturating_add(f.confusion.false_negative);
        total_confusion.true_negative = total_confusion
            .true_negative
            .saturating_add(f.confusion.true_negative);
    }
    let micro_f1 = metrics::f_measure(&total_confusion).f1;

    let finite_psnr: Vec<f64> = items.iter().filter_map(|(_, p, _)| p.db()).collect();
    let perfect_psnr_page_count = items
        .iter()
        .filter(|(_, p, _)| p.is_perfect_match())
        .count();
    let mean_psnr_db = if finite_psnr.is_empty() {
        None
    } else {
        Some(mean(finite_psnr.iter().copied()))
    };

    let mut drd_values: Vec<f64> = items.iter().map(|(_, _, d)| *d).collect();
    let mean_drd = mean(drd_values.iter().copied());
    drd_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_drd = median_of_sorted(&drd_values);
    let max_drd = drd_values.last().copied().unwrap_or(0.0);

    AggregateMetrics {
        page_count,
        macro_f1,
        micro_f1,
        mean_precision,
        mean_recall,
        mean_psnr_db,
        perfect_psnr_page_count,
        mean_drd,
        median_drd,
        max_drd,
        mean_bytes_per_megapixel: None,
        median_processing_duration_us: None,
        total_processing_duration_us: None,
    }
}

fn worst_pages(pages: &[PageBenchmarkResult]) -> WorstPages {
    let lowest_f1 = pages
        .iter()
        .min_by(|a, b| a.f_measure.f1.partial_cmp(&b.f_measure.f1).unwrap())
        .map(|p| PageExtreme {
            page_id: p.page_id.clone(),
            value: p.f_measure.f1,
        });
    let highest_drd = pages
        .iter()
        .max_by(|a, b| a.drd.partial_cmp(&b.drd).unwrap())
        .map(|p| PageExtreme {
            page_id: p.page_id.clone(),
            value: p.drd,
        });
    let largest_compressed_page = pages
        .iter()
        .max_by_key(|p| p.ccitt_bytes)
        .map(|p| PageExtreme {
            page_id: p.page_id.clone(),
            value: p.ccitt_bytes as f64,
        });
    let slowest_page = pages
        .iter()
        .max_by_key(|p| p.processing_duration_us)
        .map(|p| PageExtreme {
            page_id: p.page_id.clone(),
            value: p.processing_duration_us as f64,
        });

    WorstPages {
        lowest_f1,
        highest_drd,
        largest_compressed_page,
        slowest_page,
    }
}

fn mean(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for v in values {
        sum += v;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn median_of_sorted_u64(sorted: &[u64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0
    } else {
        sorted[mid] as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::dataset::{DatasetManifest, LoadedDataset, PageManifest};
    use crate::benchmark::mask_io::GroundTruthPolarity;
    use crate::benchmark::profile::{LoadedProfile, MethodConfig, ProfileManifest, RunConfig};
    use crate::benchmark::roi::Roi;
    use image::{GrayImage, Luma};

    fn write_gray_png(path: &std::path::Path, values: &[u8], width: u32, height: u32) {
        let mut img = GrayImage::new(width, height);
        for (i, &v) in values.iter().enumerate() {
            img.put_pixel((i as u32) % width, (i as u32) / width, Luma([v]));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        img.save(path).unwrap();
    }

    /// Builds a tiny two-page synthetic dataset directly (not via TOML
    /// parsing — that path is covered by `dataset.rs`'s own tests) so
    /// this module can test the runner/aggregation logic in isolation.
    fn tiny_dataset(dir: &std::path::Path) -> LoadedDataset {
        // Page 1: a clean 8x8 block with a black square, easy to
        // binarize correctly (manual threshold well-separated).
        write_gray_png(
            &dir.join("input/page-1.png"),
            &{
                let mut v = vec![250u8; 64];
                for y in 2..6 {
                    for x in 2..6 {
                        v[y * 8 + x] = 10;
                    }
                }
                v
            },
            8,
            8,
        );
        write_gray_png(
            &dir.join("ground-truth/page-1.png"),
            &{
                let mut v = vec![255u8; 64];
                for y in 2..6 {
                    for x in 2..6 {
                        v[y * 8 + x] = 0;
                    }
                }
                v
            },
            8,
            8,
        );

        // Page 2: identical shape, different category, with one ROI.
        write_gray_png(
            &dir.join("input/page-2.png"),
            &{
                let mut v = vec![250u8; 64];
                for y in 2..6 {
                    for x in 2..6 {
                        v[y * 8 + x] = 10;
                    }
                }
                v
            },
            8,
            8,
        );
        write_gray_png(
            &dir.join("ground-truth/page-2.png"),
            &{
                let mut v = vec![255u8; 64];
                for y in 2..6 {
                    for x in 2..6 {
                        v[y * 8 + x] = 0;
                    }
                }
                v
            },
            8,
            8,
        );

        let manifest = DatasetManifest {
            schema: crate::benchmark::dataset::DATASET_SCHEMA.to_string(),
            schema_version: crate::benchmark::dataset::DATASET_SCHEMA_VERSION.to_string(),
            id: "tiny-test-dataset".to_string(),
            title: "Tiny test dataset".to_string(),
            description: String::new(),
            license: "CC0-1.0".to_string(),
            provenance: "unit test".to_string(),
            ground_truth_method: "synthetic".to_string(),
            ground_truth_polarity: GroundTruthPolarity::BlackIsZero,
            ground_truth_creator: None,
            ground_truth_review_status: None,
            ground_truth_review_method: None,
            pages: vec![
                PageManifest {
                    id: "page-1".to_string(),
                    input: "input/page-1.png".to_string(),
                    ground_truth: "ground-truth/page-1.png".to_string(),
                    category: "clean_text".to_string(),
                    tags: vec![],
                    notes: None,
                    source_url: None,
                    rights_holder: None,
                    expected_width: Some(8),
                    expected_height: Some(8),
                    rois: vec![],
                },
                PageManifest {
                    id: "page-2".to_string(),
                    input: "input/page-2.png".to_string(),
                    ground_truth: "ground-truth/page-2.png".to_string(),
                    category: "faint_text".to_string(),
                    tags: vec![],
                    notes: None,
                    source_url: None,
                    rights_holder: None,
                    expected_width: Some(8),
                    expected_height: Some(8),
                    rois: vec![Roi {
                        id: "roi-1".to_string(),
                        x: 2,
                        y: 2,
                        width: 4,
                        height: 4,
                        tag: Some("diacritic".to_string()),
                    }],
                },
            ],
        };
        let manifest_toml = toml::to_string(&manifest).unwrap();
        std::fs::write(dir.join("dataset.toml"), manifest_toml).unwrap();
        LoadedDataset::load(&dir.join("dataset.toml")).unwrap()
    }

    fn otsu_profile() -> LoadedProfile {
        let manifest = ProfileManifest {
            schema: crate::benchmark::profile::PROFILE_SCHEMA.to_string(),
            schema_version: crate::benchmark::profile::PROFILE_SCHEMA_VERSION.to_string(),
            id: "test-profile".to_string(),
            description: String::new(),
            runs: vec![RunConfig {
                id: "otsu-300".to_string(),
                dpi: 300,
                method: MethodConfig::Otsu,
                threshold: None,
                k: None,
                window_size: None,
                dynamic_range: None,
                contrast: 0.0,
                median_denoise: false,
                background_normalization: false,
                background_radius: None,
                despeckle: None,
            }],
        };
        let text = toml::to_string(&manifest).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        std::fs::write(&path, text).unwrap();
        // Leak the tempdir so the file outlives this function; acceptable
        // in a test.
        std::mem::forget(dir);
        LoadedProfile::load(&path).unwrap()
    }

    #[test]
    fn runs_a_tiny_raster_benchmark_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = tiny_dataset(dir.path());
        let profile = otsu_profile();

        let report = run_raster_benchmark(&dataset, &profile).unwrap();
        assert_eq!(report.runs.len(), 1);
        let run = &report.runs[0];
        assert_eq!(run.pages.len(), 2);
        assert_eq!(run.method, "otsu");
        assert_eq!(run.aggregate.page_count, 2);
        // Otsu on a well-separated block should recover it near-perfectly.
        assert!(run.aggregate.macro_f1 > 0.9);
        assert_eq!(run.category_aggregates.len(), 2);
        assert!(run.category_aggregates.contains_key("clean_text"));
        assert!(run.category_aggregates.contains_key("faint_text"));
        assert!(run.roi_tag_aggregates.contains_key("diacritic"));
        assert!(run.worst_pages.lowest_f1.is_some());
    }

    #[test]
    fn every_page_has_no_render_duration_at_the_raster_level() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = tiny_dataset(dir.path());
        let profile = otsu_profile();
        let report = run_raster_benchmark(&dataset, &profile).unwrap();
        for page in &report.runs[0].pages {
            assert_eq!(page.render_duration_us, None);
        }
    }

    #[test]
    fn environment_records_no_pdfium_library_at_the_raster_level() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = tiny_dataset(dir.path());
        let profile = otsu_profile();
        let report = run_raster_benchmark(&dataset, &profile).unwrap();
        assert_eq!(report.environment.pdfium_library, None);
        assert_eq!(report.environment.benchmark_level, BenchmarkLevel::Raster);
    }

    #[test]
    fn report_serializes_without_nan_or_infinity() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = tiny_dataset(dir.path());
        let profile = otsu_profile();
        let report = run_raster_benchmark(&dataset, &profile).unwrap();
        let json = serde_json::to_string(&report);
        assert!(json.is_ok(), "report must serialize: {json:?}");
    }

    #[test]
    fn running_twice_produces_identical_content_fields() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = tiny_dataset(dir.path());
        let profile = otsu_profile();
        let a = run_raster_benchmark(&dataset, &profile).unwrap();
        let b = run_raster_benchmark(&dataset, &profile).unwrap();
        assert_eq!(a.dataset, b.dataset);
        assert_eq!(a.profile, b.profile);
        assert_eq!(a.runs.len(), b.runs.len());
        for (ra, rb) in a.runs.iter().zip(&b.runs) {
            assert_eq!(ra.pages.len(), rb.pages.len());
            for (pa, pb) in ra.pages.iter().zip(&rb.pages) {
                assert_eq!(pa.f_measure, pb.f_measure);
                assert_eq!(pa.psnr, pb.psnr);
                assert_eq!(pa.drd, pb.drd);
                assert_eq!(pa.ccitt_bytes, pb.ccitt_bytes);
                // processing_duration_us is runtime-dependent and
                // deliberately excluded from this comparison.
            }
        }
    }
}
