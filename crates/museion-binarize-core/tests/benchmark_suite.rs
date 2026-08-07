//! Integration tests against the committed `test-data/benchmark/` suite.
//!
//! Every test here runs without PDFium — the raster benchmark level
//! never touches it — so this file (unlike `pdf_pipeline.rs`) has no
//! `#[ignore]` tests and always runs in hosted CI.

use std::path::{Path, PathBuf};

use museion_binarize_core::benchmark::dataset::LoadedDataset;
use museion_binarize_core::benchmark::profile::LoadedProfile;
use museion_binarize_core::benchmark::runner::run_raster_benchmark;
use museion_binarize_core::benchmark_fixtures::{self, ALL_CATEGORIES};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/museion-binarize-core; the repo root
    // is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn synthetic_v1_dataset_path() -> PathBuf {
    repo_root().join("test-data/benchmark/synthetic-v1/dataset.toml")
}

fn baseline_profile_path() -> PathBuf {
    repo_root().join("test-data/benchmark/profiles/baseline.toml")
}

#[test]
fn committed_synthetic_v1_dataset_loads_and_validates() {
    let dataset = LoadedDataset::load(&synthetic_v1_dataset_path()).unwrap();
    assert_eq!(dataset.manifest.id, "synthetic-document-v1");
    assert_eq!(dataset.manifest.pages.len(), ALL_CATEGORIES.len());
    assert_eq!(dataset.manifest.license, "CC0-1.0");
}

#[test]
fn committed_baseline_profile_loads_and_validates() {
    let profile = LoadedProfile::load(&baseline_profile_path()).unwrap();
    assert_eq!(profile.manifest.runs.len(), 3);
}

#[test]
fn benchmark_fixture_suite_regenerates_byte_identical_pngs() {
    // Regenerates every fixture into a fresh temp directory using the
    // exact same generator this repository's committed suite came from,
    // and compares bytes directly against what is actually committed —
    // real evidence of reproducibility, not merely "the generator ran
    // without panicking."
    use image::{GrayImage, Luma};
    use museion_binarize_core::bilevel::BinaryMask;

    fn ground_truth_png(mask: &BinaryMask) -> GrayImage {
        let mut img = GrayImage::new(mask.width, mask.height);
        for y in 0..mask.height {
            for x in 0..mask.width {
                let value = if mask.get(x, y) { 0u8 } else { 255u8 };
                img.put_pixel(x, y, Luma([value]));
            }
        }
        img
    }

    let committed_root = repo_root().join("test-data/benchmark/synthetic-v1");
    let temp_dir = tempfile::tempdir().unwrap();

    for category in ALL_CATEGORIES {
        let fixture = benchmark_fixtures::generate(category);
        let id = category.id();

        let regenerated_input_path = temp_dir.path().join(format!("{id}-input.png"));
        fixture.input.save(&regenerated_input_path).unwrap();
        let regenerated_input_bytes = std::fs::read(&regenerated_input_path).unwrap();
        let committed_input_bytes =
            std::fs::read(committed_root.join("input").join(format!("{id}.png"))).unwrap();
        assert_eq!(
            regenerated_input_bytes, committed_input_bytes,
            "input PNG for '{id}' is not byte-identical to the committed fixture"
        );

        let regenerated_gt_path = temp_dir.path().join(format!("{id}-gt.png"));
        ground_truth_png(&fixture.ground_truth)
            .save(&regenerated_gt_path)
            .unwrap();
        let regenerated_gt_bytes = std::fs::read(&regenerated_gt_path).unwrap();
        let committed_gt_bytes = std::fs::read(
            committed_root
                .join("ground-truth")
                .join(format!("{id}.png")),
        )
        .unwrap();
        assert_eq!(
            regenerated_gt_bytes, committed_gt_bytes,
            "ground-truth PNG for '{id}' is not byte-identical to the committed fixture"
        );
    }
}

#[test]
fn synthetic_v1_baseline_benchmark_runs_and_produces_plausible_scores() {
    let dataset = LoadedDataset::load(&synthetic_v1_dataset_path()).unwrap();
    let profile = LoadedProfile::load(&baseline_profile_path()).unwrap();

    let report = run_raster_benchmark(&dataset, &profile).unwrap();
    assert_eq!(report.runs.len(), 3);
    for run in &report.runs {
        assert_eq!(run.pages.len(), ALL_CATEGORIES.len());
        assert!(run.aggregate.macro_f1.is_finite());
        assert!((0.0..=1.0).contains(&run.aggregate.macro_f1));
        // "clean_text" should be recovered well by every method on this
        // deterministic, high-contrast synthetic fixture.
        let clean_text = run
            .pages
            .iter()
            .find(|p| p.page_id == "clean_text")
            .expect("clean_text page must be present");
        assert!(
            clean_text.f_measure.f1 > 0.8,
            "run '{}' scored {} F1 on clean_text, expected a well-recovered easy case",
            run.run_id,
            clean_text.f_measure.f1
        );
    }
    // The report itself must be serializable (no NaN/Infinity anywhere).
    assert!(serde_json::to_string(&report).is_ok());
}

#[test]
fn synthetic_v1_diacritic_roi_aggregate_is_present() {
    let dataset = LoadedDataset::load(&synthetic_v1_dataset_path()).unwrap();
    let profile = LoadedProfile::load(&baseline_profile_path()).unwrap();
    let report = run_raster_benchmark(&dataset, &profile).unwrap();
    for run in &report.runs {
        assert!(
            run.roi_tag_aggregates.contains_key("diacritic"),
            "run '{}' is missing the diacritic ROI aggregate",
            run.run_id
        );
        assert!(
            run.roi_tag_aggregates.contains_key("apparatus"),
            "run '{}' is missing the apparatus ROI aggregate",
            run.run_id
        );
    }
}

#[test]
fn running_the_committed_suite_twice_is_deterministic_in_content_fields() {
    let dataset = LoadedDataset::load(&synthetic_v1_dataset_path()).unwrap();
    let profile = LoadedProfile::load(&baseline_profile_path()).unwrap();
    let a = run_raster_benchmark(&dataset, &profile).unwrap();
    let b = run_raster_benchmark(&dataset, &profile).unwrap();
    assert_eq!(a.dataset, b.dataset);
    assert_eq!(a.profile, b.profile);
    for (ra, rb) in a.runs.iter().zip(&b.runs) {
        for (pa, pb) in ra.pages.iter().zip(&rb.pages) {
            assert_eq!(pa.f_measure, pb.f_measure);
            assert_eq!(pa.psnr, pb.psnr);
            assert_eq!(pa.drd, pb.drd);
            assert_eq!(pa.ccitt_bytes, pb.ccitt_bytes);
        }
    }
}
