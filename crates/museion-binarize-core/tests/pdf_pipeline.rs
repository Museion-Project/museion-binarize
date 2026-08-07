//! End-to-end PDF pipeline integration tests.
//!
//! These tests need a real PDFium dynamic library. Point
//! `MUSEION_PDFIUM_LIBRARY` at one (see `docs/pdfium.md`); without it every
//! test here reports that it was skipped rather than failing, so
//! `cargo test --workspace` stays green on a machine with no PDFium.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use museion_binarize_core::binarization::SauvolaParams;
use museion_binarize_core::cleanup::CleanupSettings;
use museion_binarize_core::error::CoreError;
use museion_binarize_core::pdfium_backend::PdfiumConfig;
use museion_binarize_core::pipeline::{
    self, leftover_temporary_files, PdfProcessingOptions, ProcessingReport,
};
use museion_binarize_core::progress::{ProgressEvent, ProgressReporter};
use museion_binarize_core::renderer::{PdfOpenOptions, PdfRenderer};
use museion_binarize_core::settings::{
    BinarizationMethod, PreprocessingSettings, ProcessingSettings,
};
use museion_binarize_core::test_fixtures;
use museion_binarize_core::validation::ValidationMode;

/// Returns the PDFium configuration, or `None` when no library is
/// configured for this run.
fn pdfium() -> Option<PdfiumConfig> {
    let path = std::env::var_os("MUSEION_PDFIUM_LIBRARY")?;
    let path = PathBuf::from(path);
    if !path.is_file() {
        return None;
    }
    Some(PdfiumConfig {
        library_path: Some(path),
        allow_system_library: false,
    })
}

/// Serializes PDFium use across tests.
///
/// PDFium is initialized once per process and this project deliberately
/// drives it sequentially (see `docs/adr/0001-pdfium-runtime-binding.md`);
/// exercising documents from several test threads at once crashes inside
/// the C++ library. `cargo test` runs test functions in parallel by
/// default, so every PDFium-touching test takes this lock.
static PDFIUM_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn pdfium_guard() -> std::sync::MutexGuard<'static, ()> {
    PDFIUM_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Skips the calling test (printing why) when PDFium is unavailable.
macro_rules! require_pdfium {
    () => {{
        match pdfium() {
            Some(config) => {
                // Held for the rest of the test body.
                let guard = pdfium_guard();
                (config, guard)
            }
            None => {
                eprintln!(
                    "SKIPPED: set MUSEION_PDFIUM_LIBRARY to a PDFium dynamic library to run this test"
                );
                return;
            }
        }
    }};
}

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn settings(method: BinarizationMethod, dpi: u16) -> ProcessingSettings {
    ProcessingSettings {
        dpi,
        method,
        contrast: 0.0,
        preprocessing: PreprocessingSettings::default(),
        cleanup: CleanupSettings::default(),
    }
}

fn options(pdfium: PdfiumConfig, validation: ValidationMode) -> PdfProcessingOptions {
    PdfProcessingOptions {
        password: None,
        overwrite: false,
        validation,
        pdfium,
    }
}

/// Collects every progress event, and can cancel after N page starts.
struct RecordingProgress {
    events: std::sync::Mutex<Vec<ProgressEvent>>,
    cancel_after_pages: Option<u32>,
    pages_started: AtomicU32,
}

impl RecordingProgress {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
            cancel_after_pages: None,
            pages_started: AtomicU32::new(0),
        }
    }

    fn cancelling_after(pages: u32) -> Self {
        Self {
            cancel_after_pages: Some(pages),
            ..Self::new()
        }
    }

    fn events(&self) -> Vec<ProgressEvent> {
        self.events.lock().expect("progress lock").clone()
    }
}

impl ProgressReporter for RecordingProgress {
    fn report(&self, event: ProgressEvent) {
        if matches!(event, ProgressEvent::PageStarted { .. }) {
            self.pages_started.fetch_add(1, Ordering::SeqCst);
        }
        self.events.lock().expect("progress lock").push(event);
    }

    fn is_cancelled(&self) -> bool {
        match self.cancel_after_pages {
            Some(limit) => self.pages_started.load(Ordering::SeqCst) > limit,
            None => false,
        }
    }
}

/// Renders one page of a PDF to grayscale for pixel assertions.
fn render_gray(path: &Path, index: u32, dpi: u16, pdfium: &PdfiumConfig) -> image::GrayImage {
    let renderer = PdfRenderer::open(
        path,
        &PdfOpenOptions {
            password: None,
            pdfium: pdfium.clone(),
        },
    )
    .expect("open for rendering");
    renderer.render_page(index, dpi).expect("render").to_luma8()
}

/// Mean luminance of a rectangular region given in fractions of the image.
fn region_mean(image: &image::GrayImage, x0: f32, y0: f32, x1: f32, y1: f32) -> f64 {
    let (w, h) = (image.width() as f32, image.height() as f32);
    let (px0, py0) = ((x0 * w) as u32, (y0 * h) as u32);
    let (px1, py1) = ((x1 * w) as u32, (y1 * h) as u32);
    let mut total = 0u64;
    let mut count = 0u64;
    for y in py0..py1.min(image.height()) {
        for x in px0..px1.min(image.width()) {
            total += u64::from(image.get_pixel(x, y)[0]);
            count += 1;
        }
    }
    assert!(count > 0, "empty sample region");
    total as f64 / count as f64
}

#[test]
fn loads_the_pinned_pdfium_library() {
    let (config, _pdfium_guard) = require_pdfium!();
    let description =
        pipeline::describe_pdfium_library(&config).expect("should resolve the library");
    assert!(description.contains("explicit path"));
}

#[test]
fn opens_and_inspects_a_generated_single_page_pdf() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );

    let info = pipeline::inspect_pdf(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config,
        },
    )
    .expect("inspect");

    assert_eq!(info.page_count, 1);
    assert_eq!(info.pages.len(), 1);
    assert_eq!(info.pages[0].index, 0);
    assert_eq!(info.pages[0].page_number(), 1);
    let g = &info.pages[0].geometry;
    assert!((g.display_width_points() - test_fixtures::A4_PORTRAIT.0).abs() < 0.1);
    assert!((g.display_height_points() - test_fixtures::A4_PORTRAIT.1).abs() < 0.1);
    assert!(info.source_bytes > 0);
}

#[test]
fn renders_at_every_supported_dpi() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );
    let renderer = PdfRenderer::open(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config,
        },
    )
    .expect("open");

    let mut last = 0;
    for dpi in [300u16, 400, 600] {
        let image = renderer.render_page(0, dpi).expect("render");
        assert!(image.width() > last, "higher DPI must yield more pixels");
        last = image.width();
        // 595 pt at 300 DPI = 2479 px.
        let expected = (595.0 * f64::from(dpi) / 72.0).round() as u32;
        assert!(
            image.width().abs_diff(expected) <= 1,
            "at {dpi} DPI expected ~{expected} px wide, got {}",
            image.width()
        );
    }
}

/// The central correctness test: convert the asymmetric fixture, reopen
/// the output, and confirm polarity and orientation survived.
#[test]
fn end_to_end_conversion_preserves_polarity_and_orientation() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );
    let output = dir.path().join("out.pdf");

    let report: ProcessingReport = pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Manual { threshold: 128 }, 300),
        &options(config.clone(), ValidationMode::RenderAll),
        &RecordingProgress::new(),
    )
    .expect("conversion should succeed");

    assert_eq!(report.pages_processed, 1);
    assert!(output.is_file(), "output must exist");
    assert!(report.output_bytes > 0);

    // The output must really be bilevel CCITT Group 4.
    let bytes = std::fs::read(&output).unwrap();
    museion_binarize_core::validation::assert_bilevel_ccitt_structure(&bytes)
        .expect("output must be CCITT Group 4 bilevel");

    // Reopen and render the OUTPUT, then compare marker regions.
    let rendered = render_gray(&output, 0, 150, &config);

    // Fixture layout (visual): large black square top-left, small black
    // square bottom-right, mostly-white centre-right.
    // Sample regions are derived from the fixture's own geometry (see
    // test_fixtures::orientation_markers) rather than guessed, and are
    // inset slightly so antialiasing at the marker edges cannot skew them.
    let (pw, ph) = test_fixtures::A4_PORTRAIT;
    let unit = pw.min(ph);
    let large = unit * 0.20;
    let small = unit * 0.10;
    let margin = unit * 0.05;
    let inset = 0.15; // fraction of the marker to trim from each edge

    // Visual top-left marker.
    let top_left = region_mean(
        &rendered,
        (margin + large * inset) / pw,
        (margin + large * inset) / ph,
        (margin + large * (1.0 - inset)) / pw,
        (margin + large * (1.0 - inset)) / ph,
    );
    // Visual bottom-right marker: PDF y is measured from the bottom, so
    // its distance from the visual top is (height - margin - small).
    let br_top = (ph - margin - small) / ph;
    let br_bottom = (ph - margin) / ph;
    let bottom_right = region_mean(
        &rendered,
        (pw - margin - small * (1.0 - inset)) / pw,
        br_top + (br_bottom - br_top) * inset,
        (pw - margin - small * inset) / pw,
        br_bottom - (br_bottom - br_top) * inset,
    );
    // Open areas that must stay white.
    let centre_right = region_mean(&rendered, 0.55, 0.55, 0.85, 0.75);
    let top_right = region_mean(&rendered, 0.85, 0.04, 0.98, 0.12);

    assert!(
        top_left < 64.0,
        "top-left marker must be BLACK (mean {top_left}); a bright value means the page is inverted or flipped"
    );
    assert!(
        bottom_right < 64.0,
        "bottom-right marker must be BLACK (mean {bottom_right})"
    );
    assert!(
        centre_right > 192.0,
        "the open centre-right area must be WHITE (mean {centre_right}); a dark value means the page is inverted"
    );
    assert!(
        top_right > 192.0,
        "top-right must be WHITE (mean {top_right}); a dark value means the page is mirrored horizontally"
    );

    // Explicit anti-flip / anti-mirror assertions: the large marker is in
    // the top-left, so the top-left must be darker than both the
    // bottom-left (vertical flip) and the top-right (horizontal mirror).
    let bottom_left = region_mean(&rendered, 0.06, 0.86, 0.18, 0.95);
    assert!(
        top_left < bottom_left,
        "top-left ({top_left}) must be darker than bottom-left ({bottom_left}); otherwise the page is flipped vertically"
    );
    assert!(
        top_left < top_right,
        "top-left ({top_left}) must be darker than top-right ({top_right}); otherwise the page is mirrored horizontally"
    );
}

#[test]
fn preserves_page_count_and_mixed_page_sizes() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());
    let output = dir.path().join("mixed-out.pdf");

    let report = pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &options(config.clone(), ValidationMode::RenderAll),
        &RecordingProgress::new(),
    )
    .expect("conversion");
    assert_eq!(report.pages_processed, 3);

    let info = pipeline::inspect_pdf(
        &output,
        &PdfOpenOptions {
            password: None,
            pdfium: config,
        },
    )
    .expect("reopen output");
    assert_eq!(info.page_count, 3);

    let expected = [
        test_fixtures::A4_PORTRAIT,
        test_fixtures::LANDSCAPE,
        test_fixtures::SMALL,
    ];
    for (page, (ew, eh)) in info.pages.iter().zip(expected) {
        let (aw, ah) = (
            page.geometry.display_width_points(),
            page.geometry.display_height_points(),
        );
        assert!(
            (aw - ew).abs() < 0.1 && (ah - eh).abs() < 0.1,
            "page {} is {aw}x{ah}, expected {ew}x{eh}",
            page.page_number()
        );
    }
}

#[test]
fn preserves_visible_geometry_for_rotated_pages() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "rot.pdf", &test_fixtures::page_rotations());
    let output = dir.path().join("rot-out.pdf");

    let report = pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Manual { threshold: 128 }, 300),
        &options(config.clone(), ValidationMode::RenderAll),
        &RecordingProgress::new(),
    )
    .expect("conversion");
    assert_eq!(report.pages_processed, 4);

    let source = pipeline::inspect_pdf(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
        },
    )
    .unwrap();
    let out = pipeline::inspect_pdf(
        &output,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
        },
    )
    .unwrap();
    assert_eq!(out.page_count, 4);

    for (src, dst) in source.pages.iter().zip(out.pages.iter()) {
        // The visible rectangle must survive, whatever the source /Rotate.
        assert!(
            (src.geometry.display_width_points() - dst.geometry.display_width_points()).abs() < 0.1,
            "page {} visible width changed",
            src.page_number()
        );
        assert!(
            (src.geometry.display_height_points() - dst.geometry.display_height_points()).abs()
                < 0.1,
            "page {} visible height changed",
            src.page_number()
        );
    }

    // The real rotation assertion: for every page, the OUTPUT's visible
    // rendering must match the SOURCE's visible rendering. A page with
    // /Rotate 90 legitimately shows its markers in a different corner
    // than the unrotated page, so comparing against a fixed corner would
    // be wrong; comparing source against output catches any *extra*
    // rotation, flip, or mirror introduced by the pipeline.
    for index in 0..4 {
        let source_render = render_gray(&input, index, 120, &config);
        let output_render = render_gray(&output, index, 120, &config);

        let source_grid = coarse_grid(&source_render);
        let output_grid = coarse_grid(&output_render);

        // Binarization hardens the antialiased edges, so cells are
        // compared as "mostly dark" vs "mostly light" rather than by
        // exact luminance.
        let mismatches = source_grid
            .iter()
            .zip(output_grid.iter())
            .filter(|(a, b)| (**a < 128.0) != (**b < 128.0))
            .count();
        assert!(
            mismatches <= 2,
            "page {} (rotation {}°): output layout differs from the source in {mismatches} of {} \
             cells; the pipeline introduced an unexpected rotation, flip, or mirror",
            index + 1,
            source.pages[index as usize].geometry.rotation.degrees(),
            source_grid.len()
        );

        // And the page must not be blank or fully inked either way.
        let dark_cells = output_grid.iter().filter(|v| **v < 128.0).count();
        assert!(
            dark_cells > 0 && dark_cells < output_grid.len(),
            "page {} is uniformly blank or uniformly black",
            index + 1
        );
    }
}

/// Reduces an image to a 6x6 grid of mean luminances, so two renderings
/// can be compared structurally without being pixel-exact.
fn coarse_grid(image: &image::GrayImage) -> Vec<f64> {
    const N: usize = 6;
    let mut cells = Vec::with_capacity(N * N);
    for row in 0..N {
        for col in 0..N {
            cells.push(region_mean(
                image,
                col as f32 / N as f32,
                row as f32 / N as f32,
                (col + 1) as f32 / N as f32,
                (row + 1) as f32 / N as f32,
            ));
        }
    }
    cells
}

#[test]
fn cancellation_leaves_no_output_and_no_temporary_files() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());
    let output = dir.path().join("cancelled.pdf");

    let progress = RecordingProgress::cancelling_after(1);
    let err = pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &options(config, ValidationMode::Structural),
        &progress,
    )
    .expect_err("cancellation must surface as an error");

    assert!(matches!(err, CoreError::Cancelled));
    assert!(!output.exists(), "no destination file may be left behind");
    assert!(
        leftover_temporary_files(dir.path()).is_empty(),
        "no partial temporary file may be left behind"
    );

    let events = progress.events();
    assert!(events.contains(&ProgressEvent::Cancelled));
    assert!(!events.contains(&ProgressEvent::Finished));
    // The source is untouched.
    assert!(input.is_file());
}

#[test]
fn repeated_conversions_are_byte_for_byte_identical() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());

    let mut outputs = Vec::new();
    for run in 0..2 {
        let output = dir.path().join(format!("out-{run}.pdf"));
        pipeline::process_pdf(
            &input,
            &output,
            &settings(BinarizationMethod::Sauvola(SauvolaParams::default()), 300),
            &options(config.clone(), ValidationMode::Structural),
            &RecordingProgress::new(),
        )
        .expect("conversion");
        outputs.push(std::fs::read(&output).unwrap());
    }
    assert_eq!(
        outputs[0], outputs[1],
        "the same input and settings must produce byte-identical output"
    );
}

#[test]
fn refuses_an_existing_destination_without_overwrite_and_honours_it_with() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );
    let output = dir.path().join("out.pdf");
    std::fs::write(&output, b"pre-existing").unwrap();

    let mut opts = options(config.clone(), ValidationMode::Structural);
    let err = pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &opts,
        &RecordingProgress::new(),
    )
    .expect_err("must refuse to overwrite by default");
    assert!(matches!(err, CoreError::DestinationConflict(_)));
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"pre-existing",
        "the existing file must be untouched"
    );

    opts.overwrite = true;
    pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &opts,
        &RecordingProgress::new(),
    )
    .expect("overwrite should succeed");
    assert_ne!(std::fs::read(&output).unwrap(), b"pre-existing");
    assert!(leftover_temporary_files(dir.path()).is_empty());
}

#[test]
fn progress_events_arrive_in_the_documented_order() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());
    let output = dir.path().join("out.pdf");

    let progress = RecordingProgress::new();
    pipeline::process_pdf(
        &input,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &options(config, ValidationMode::Structural),
        &progress,
    )
    .expect("conversion");

    let events = progress.events();
    assert!(matches!(
        events.first(),
        Some(ProgressEvent::Started { total_pages: 3 })
    ));
    assert_eq!(events.last(), Some(&ProgressEvent::Finished));
    assert!(events.contains(&ProgressEvent::Validating));
    for page in 1..=3u32 {
        assert!(events.contains(&ProgressEvent::PageStarted { page }));
        assert!(events
            .iter()
            .any(|e| matches!(e, ProgressEvent::PageFinished { page: p, .. } if *p == page)));
    }
    assert!(!events.contains(&ProgressEvent::Cancelled));
}

#[test]
fn the_thresholding_algorithms_actually_run_on_grayscale_content() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "gray.pdf", &test_fixtures::threshold_patterns());

    // A very low manual threshold keeps only the near-black band; a high
    // one also captures the mid grays. If the algorithm were bypassed, the
    // two outputs would be identical.
    let mut sizes = Vec::new();
    for (run, threshold) in [(0u32, 20u8), (1, 200)] {
        let output = dir.path().join(format!("gray-{run}.pdf"));
        pipeline::process_pdf(
            &input,
            &output,
            &settings(BinarizationMethod::Manual { threshold }, 300),
            &options(config.clone(), ValidationMode::Structural),
            &RecordingProgress::new(),
        )
        .expect("conversion");
        sizes.push(std::fs::read(&output).unwrap());
    }
    assert_ne!(
        sizes[0], sizes[1],
        "different thresholds must produce different output"
    );
}

#[test]
fn refuses_to_write_over_the_input_document() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );
    let original = std::fs::read(&input).unwrap();

    let mut opts = options(config, ValidationMode::Structural);
    opts.overwrite = true;
    let err = pipeline::process_pdf(
        &input,
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &opts,
        &RecordingProgress::new(),
    )
    .expect_err("must refuse input == output");
    assert!(matches!(err, CoreError::DestinationConflict(_)));
    assert_eq!(std::fs::read(&input).unwrap(), original);
}
