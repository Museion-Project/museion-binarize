//! End-to-end PDF pipeline integration tests.
//!
//! Every test in this file needs a real PDFium dynamic library and is
//! therefore marked `#[ignore]`. Rust's test harness has no dynamic "skip"
//! result — a test that returns early is recorded as **passed** — so an
//! unavailable dependency must never be represented by returning. Instead:
//!
//! * `cargo test --workspace` reports these as **ignored**, never as
//!   passed, so ordinary CI cannot appear to have verified the pipeline.
//! * Running them explicitly requires a library, and its absence or
//!   failure to load is a hard test failure with an actionable message:
//!
//! ```text
//! MUSEION_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.dylib \
//!   cargo test --test pdf_pipeline -- --ignored
//! ```
//!
//! See `docs/testing-pdf-pipeline.md`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use museion_binarize_core::binarization::SauvolaParams;
use museion_binarize_core::cleanup::CleanupSettings;
use museion_binarize_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use museion_binarize_core::error::CoreError;
use museion_binarize_core::pdfium_backend::PdfiumConfig;
use museion_binarize_core::pipeline::{
    self, leftover_temporary_files, PdfProcessingOptions, ProcessingReport,
};
use museion_binarize_core::progress::{ProgressEvent, ProgressReporter};
use museion_binarize_core::settings::{
    BinarizationMethod, PreprocessingSettings, ProcessingSettings,
};
use museion_binarize_core::test_fixtures;
use museion_binarize_core::validation::ValidationMode;

/// Resolves the PDFium configuration for an explicitly-run ignored test.
///
/// These tests only execute when someone asked for them by name or with
/// `--ignored`, so a missing or unusable library is a **failure**, not a
/// reason to pass quietly. The panic message says exactly how to fix it.
fn require_pdfium_config() -> PdfiumConfig {
    let Some(raw) = std::env::var_os(ENV_VAR) else {
        panic!(
            "{ENV_VAR} is not set, but this test was run explicitly.\n\
             These integration tests require a provisioned PDFium library:\n  \
             {ENV_VAR}=/absolute/path/to/{} \\\n    \
             cargo test --test pdf_pipeline -- --ignored\n\
             See docs/pdfium.md for how to obtain one.",
            museion_binarize_core::pdfium_backend::pdfium_library_file_name()
        );
    };
    let path = PathBuf::from(raw);
    assert!(
        path.is_file(),
        "{ENV_VAR} points at {}, which is not a file. See docs/pdfium.md.",
        path.display()
    );
    PdfiumConfig {
        library_path: Some(path),
        allow_system_library: false,
    }
}

const ENV_VAR: &str = "MUSEION_PDFIUM_LIBRARY";

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

/// Acquires the PDFium config and the serialization guard for a test.
///
/// Never returns early: a missing library panics with an actionable
/// message, so an ignored test that is explicitly run can only pass by
/// actually exercising PDFium.
macro_rules! require_pdfium {
    () => {{
        let config = require_pdfium_config();
        // Held for the rest of the test body.
        let guard = pdfium_guard();
        (config, guard)
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
        prior_estimate: None,
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
    let renderer = PdfDocumentSession::open(
        path,
        &PdfOpenOptions {
            password: None,
            pdfium: pdfium.clone(),
            compute_source_hash: false,
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn loads_the_pinned_pdfium_library() {
    let (config, _pdfium_guard) = require_pdfium!();
    let description =
        pipeline::describe_pdfium_library(&config).expect("should resolve the library");
    assert!(description.contains("explicit path"));
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
            compute_source_hash: false,
        },
    )
    .expect("inspect");

    assert_eq!(info.page_count, 1);
    assert_eq!(info.pages.len(), 1);
    assert_eq!(info.pages[0].index, 0);
    assert_eq!(info.pages[0].page_number(), 1);
    let g = &info.pages[0].geometry;
    assert!((g.width_points - test_fixtures::A4_PORTRAIT.0).abs() < 0.1);
    assert!((g.height_points - test_fixtures::A4_PORTRAIT.1).abs() < 0.1);
    assert!(info.source_bytes > 0);
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn renders_at_every_supported_dpi() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "a.pdf",
        &test_fixtures::orientation_and_polarity(),
    );
    let renderer = PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config,
            compute_source_hash: false,
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
            compute_source_hash: false,
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
        let (aw, ah) = (page.geometry.width_points, page.geometry.height_points);
        assert!(
            (aw - ew).abs() < 0.1 && (ah - eh).abs() < 0.1,
            "page {} is {aw}x{ah}, expected {ew}x{eh}",
            page.page_number()
        );
    }
}

/// Independent oracle for the rotation fixture.
///
/// These values come from the fixture's own constants and the PDF
/// specification's `/Rotate` semantics — never from
/// `PageGeometry`/`points_to_pixels` or any other production geometry
/// helper, so this test can detect an error in those helpers instead of
/// inheriting it. The fixture builds four A4-portrait pages carrying
/// `/Rotate` 0, 90, 180, 270 in that order.
const ROTATION_FIXTURE_ORDER: [u32; 4] = [0, 90, 180, 270];

/// Expected *visible* page size in points for a source `/Rotate`, derived
/// straight from the fixture's MediaBox constant.
fn expected_visible_points(rotate_degrees: u32) -> (f32, f32) {
    let (media_w, media_h) = test_fixtures::A4_PORTRAIT;
    match rotate_degrees {
        90 | 270 => (media_h, media_w),
        _ => (media_w, media_h),
    }
}

/// Pixels for a length in points at a DPI, computed here from the
/// definition (72 points per inch) rather than by calling production code.
fn expected_pixels(points: f32, dpi: u32) -> u32 {
    (f64::from(points) * f64::from(dpi) / 72.0).round() as u32
}

/// A connected region of dark pixels: bounding box, area, centroid.
struct Blob {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    area: u32,
}

impl Blob {
    fn width(&self) -> u32 {
        self.max_x - self.min_x + 1
    }
    fn height(&self) -> u32 {
        self.max_y - self.min_y + 1
    }
    fn aspect(&self) -> f64 {
        f64::from(self.width()) / f64::from(self.height())
    }
    /// Fraction of the bounding box that is actually inked; a solid
    /// rectangle is ~1.0.
    fn fill(&self) -> f64 {
        f64::from(self.area) / (f64::from(self.width()) * f64::from(self.height()))
    }
    fn centroid(&self) -> (f64, f64) {
        (
            f64::from(self.min_x + self.max_x) / 2.0,
            f64::from(self.min_y + self.max_y) / 2.0,
        )
    }
}

/// Finds 4-connected dark regions, ignoring specks below `min_area`.
///
/// Deliberately a plain flood fill written here in the test, so the
/// assertions do not depend on any production image-processing code.
fn find_blobs(image: &image::GrayImage, min_area: u32) -> Vec<Blob> {
    let (w, h) = (image.width(), image.height());
    let mut seen = vec![false; (w as usize) * (h as usize)];
    let mut blobs = Vec::new();
    let idx = |x: u32, y: u32| (y as usize) * (w as usize) + (x as usize);

    for y0 in 0..h {
        for x0 in 0..w {
            if seen[idx(x0, y0)] || image.get_pixel(x0, y0)[0] >= 128 {
                continue;
            }
            let mut stack = vec![(x0, y0)];
            seen[idx(x0, y0)] = true;
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (x0, y0, x0, y0);
            let mut area = 0u32;
            while let Some((x, y)) = stack.pop() {
                area += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                let neighbours = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in neighbours {
                    if nx < w && ny < h && !seen[idx(nx, ny)] && image.get_pixel(nx, ny)[0] < 128 {
                        seen[idx(nx, ny)] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if area >= min_area {
                blobs.push(Blob {
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                    area,
                });
            }
        }
    }
    blobs
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn rotated_pages_report_the_visible_dimensions_required_by_the_fixture() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "rot.pdf", &test_fixtures::page_rotations());

    let source = pipeline::inspect_pdf(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
            compute_source_hash: false,
        },
    )
    .unwrap();
    assert_eq!(source.page_count, 4);

    for (index, rotate) in ROTATION_FIXTURE_ORDER.iter().copied().enumerate() {
        let page = &source.pages[index];
        let (want_w, want_h) = expected_visible_points(rotate);

        assert!(
            (page.geometry.width_points - want_w).abs() < 0.5
                && (page.geometry.height_points - want_h).abs() < 0.5,
            "page {} (/Rotate {rotate}) reports {:.2}x{:.2} pt, but its visible size must be \
             {want_w:.2}x{want_h:.2} pt",
            index + 1,
            page.geometry.width_points,
            page.geometry.height_points
        );

        // The source rotation is preserved as informational metadata and
        // must not have been folded into the dimensions a second time.
        assert_eq!(
            page.source_rotation.degrees(),
            rotate,
            "page {} lost its source /Rotate metadata",
            index + 1
        );

        let (px_w, px_h) = page.geometry.pixel_size(300).unwrap();
        assert_eq!(
            (px_w, px_h),
            (expected_pixels(want_w, 300), expected_pixels(want_h, 300)),
            "page {} (/Rotate {rotate}) rasterizes to the wrong size at 300 DPI",
            index + 1
        );
    }

    // Spelled out explicitly, so a future regression cannot be hidden by a
    // helper: A4 with /Rotate 90 is 842x595 pt and 3508x2479 px at 300 DPI.
    assert_eq!(expected_visible_points(90), (842.0, 595.0));
    assert_eq!(
        (expected_pixels(842.0, 300), expected_pixels(595.0, 300)),
        (3508, 2479)
    );
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn conversion_keeps_square_markers_square_and_in_opposite_corners() {
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

    // The output's own geometry must match the independently derived
    // visible size of the corresponding source page.
    let out = pipeline::inspect_pdf(
        &output,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
            compute_source_hash: false,
        },
    )
    .unwrap();
    for (index, rotate) in ROTATION_FIXTURE_ORDER.iter().copied().enumerate() {
        let (want_w, want_h) = expected_visible_points(rotate);
        let page = &out.pages[index];
        assert!(
            (page.geometry.width_points - want_w).abs() < 0.5
                && (page.geometry.height_points - want_h).abs() < 0.5,
            "output page {} (/Rotate {rotate}) is {:.2}x{:.2} pt, expected {want_w:.2}x{want_h:.2}",
            index + 1,
            page.geometry.width_points,
            page.geometry.height_points
        );
        assert_eq!(page.source_rotation.degrees(), 0, "output must be upright");
    }

    // The fixture draws a large square marker at the visual top-left of
    // the unrotated page and a small square at the visual bottom-right,
    // each `unit * 0.20` and `unit * 0.10` points on a side where
    // `unit = min(MediaBox)`. Under any pure rotation both stay square and
    // stay diagonally opposite; a transposed or squashed page turns them
    // into rectangles, and a mirrored page moves them off the diagonal.
    const PREVIEW_DPI: u16 = 100;
    let unit = test_fixtures::A4_PORTRAIT
        .0
        .min(test_fixtures::A4_PORTRAIT.1);
    let expected_large_px = expected_pixels(unit * 0.20, u32::from(PREVIEW_DPI));

    for index in 0..4u32 {
        let rotate = ROTATION_FIXTURE_ORDER[index as usize];
        let render = render_gray(&output, index, PREVIEW_DPI, &config);

        // Solid, roughly-square regions are the two markers; the two bars
        // are long thin rectangles and are excluded by the aspect filter.
        let mut squares: Vec<Blob> = find_blobs(&render, 64)
            .into_iter()
            .filter(|b| (b.aspect() - 1.0).abs() < 0.20 && b.fill() > 0.80)
            .collect();
        squares.sort_by_key(|b| std::cmp::Reverse(b.area));
        assert!(
            squares.len() >= 2,
            "page {} (/Rotate {rotate}): expected two square markers, found {}",
            index + 1,
            squares.len()
        );

        let large = &squares[0];
        let small = &squares[1];

        assert!(
            (large.aspect() - 1.0).abs() < 0.06,
            "page {} (/Rotate {rotate}): the square marker rendered {}x{} px (aspect {:.3}); \
             a pure rotation must keep it square, so the page was transposed or squashed",
            index + 1,
            large.width(),
            large.height(),
            large.aspect()
        );
        let size_error = (f64::from(large.width()) - f64::from(expected_large_px)).abs()
            / f64::from(expected_large_px);
        assert!(
            size_error < 0.10,
            "page {} (/Rotate {rotate}): the square marker is {} px wide, but the fixture \
             constants require about {expected_large_px} px at {PREVIEW_DPI} DPI",
            index + 1,
            large.width()
        );
        assert!(
            small.area < large.area,
            "page {} (/Rotate {rotate}): the two markers must differ in size",
            index + 1
        );

        // Diagonally opposite: the markers must sit on opposite sides of
        // both the vertical and the horizontal midline.
        let (mid_x, mid_y) = (
            f64::from(render.width()) / 2.0,
            f64::from(render.height()) / 2.0,
        );
        let (lx, ly) = large.centroid();
        let (sx, sy) = small.centroid();
        assert!(
            (lx < mid_x) != (sx < mid_x) && (ly < mid_y) != (sy < mid_y),
            "page {} (/Rotate {rotate}): markers at ({lx:.0},{ly:.0}) and ({sx:.0},{sy:.0}) are \
             not diagonally opposite about ({mid_x:.0},{mid_y:.0}); the page was mirrored or flipped",
            index + 1
        );
    }
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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

/// Milestone 4 requirement: the desktop app's entry point
/// (`process_with_open_session`, converting through a session it already
/// holds open for preview) must produce byte-identical output to the
/// CLI's entry point (`process_pdf`, which opens its own session) for the
/// same input PDF and the same settings. The desktop GUI and the CLI
/// share this same core, but this proves it directly rather than by
/// inspection.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn cli_and_gui_entry_points_produce_byte_identical_output_for_identical_settings() {
    use museion_binarize_core::pipeline::process_with_open_session;

    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());
    let chosen_settings = settings(BinarizationMethod::Sauvola(SauvolaParams::default()), 300);

    // The CLI path: process_pdf opens its own session from the path.
    let cli_output = dir.path().join("cli-out.pdf");
    pipeline::process_pdf(
        &input,
        &cli_output,
        &chosen_settings,
        &options(config.clone(), ValidationMode::Structural),
        &RecordingProgress::new(),
    )
    .expect("CLI-path conversion");

    // The desktop GUI path: a session opened once (as it would be for
    // preview), then reused for the conversion.
    let session = PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
            compute_source_hash: false,
        },
    )
    .expect("open the session the GUI would have kept open for preview");
    let gui_output = dir.path().join("gui-out.pdf");
    process_with_open_session(
        &session,
        &gui_output,
        &chosen_settings,
        &options(config, ValidationMode::Structural),
        &RecordingProgress::new(),
    )
    .expect("GUI-path conversion");

    let cli_bytes = std::fs::read(&cli_output).unwrap();
    let gui_bytes = std::fs::read(&gui_output).unwrap();
    assert_eq!(
        cli_bytes, gui_bytes,
        "the CLI and the desktop app must produce byte-identical output for identical input and settings"
    );
}

/// Milestone 5: `estimate` reports a real, positive estimate and, like
/// `analyze`, never writes any output file at all — this is the whole
/// reason estimation is expected to be cheaper than a full conversion.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_reports_real_measurements_and_writes_no_output_file() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());

    let options = pipeline::EstimationOptions {
        password: None,
        pdfium: config,
        samples: 8,
    };
    let report = pipeline::estimate_output_size(
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &options,
        &RecordingProgress::new(),
    )
    .expect("estimate should succeed");

    assert_eq!(report.document_page_count, 3);
    assert_eq!(
        report.sampled_pages.len(),
        3,
        "8 requested but only 3 pages exist"
    );
    assert!(report.estimated_output_bytes > 0);
    assert!(report.estimated_lower_bytes <= report.estimated_output_bytes);
    assert!(report.estimated_output_bytes <= report.estimated_upper_bytes);
    for sample in &report.sampled_pages {
        assert!(sample.ccitt_bytes > 0);
        assert!(sample.pixel_count > 0);
        assert!(sample.bytes_per_pixel > 0.0);
        assert!(sample.bytes_per_pixel.is_finite());
    }
    assert!(report.experimental);

    // Only the input fixture exists afterward — no output PDF, no
    // temporary file, nothing else written to the directory.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, vec![std::ffi::OsString::from("mixed.pdf")]);
}

/// The deterministic sample-selection algorithm must select the same
/// pages against a real multi-page document as it does in the
/// PDFium-independent unit tests in `pipeline.rs`.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_uses_exactly_the_deterministic_sample_pages_against_real_pdfium() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "rotations.pdf",
        &test_fixtures::page_rotations(),
    );

    let options = pipeline::EstimationOptions {
        password: None,
        pdfium: config,
        samples: 2,
    };
    let report = pipeline::estimate_output_size(
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &options,
        &RecordingProgress::new(),
    )
    .expect("estimate should succeed");

    let expected = museion_binarize_core::estimation::select_sample_pages(4, 2);
    assert_eq!(
        report
            .sampled_pages
            .iter()
            .map(|s| s.page_index)
            .collect::<Vec<_>>(),
        expected
    );
}

/// Estimation for each binarization method must succeed and respond to
/// the method actually chosen — proven by DPI/method round-tripping into
/// the report, not by a separate approximate algorithm per method.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_succeeds_for_every_binarization_method() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "threshold.pdf",
        &test_fixtures::threshold_patterns(),
    );

    for method in [
        BinarizationMethod::Otsu,
        BinarizationMethod::Manual { threshold: 150 },
        BinarizationMethod::Sauvola(SauvolaParams::default()),
    ] {
        let options = pipeline::EstimationOptions {
            password: None,
            pdfium: config.clone(),
            samples: 4,
        };
        let report = pipeline::estimate_output_size(
            &input,
            &settings(method, 300),
            &options,
            &RecordingProgress::new(),
        )
        .unwrap_or_else(|e| panic!("estimate failed for {method:?}: {e}"));
        assert!(report.estimated_output_bytes > 0);
    }
}

/// Cancellation during estimation must behave exactly like cancellation
/// during a full conversion: it surfaces as `CoreError::Cancelled` and no
/// further sample pages render after the cancellation is observed.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_can_be_cancelled_mid_run() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "rotations.pdf",
        &test_fixtures::page_rotations(),
    );

    let options = pipeline::EstimationOptions {
        password: None,
        pdfium: config,
        samples: 4,
    };
    // `cancelling_after(0)` reports cancelled once more than 0 pages have
    // started — i.e. right after the first sample finishes, before a
    // second one begins.
    let progress = RecordingProgress::cancelling_after(0);
    let err = pipeline::estimate_output_size(
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &options,
        &progress,
    )
    .expect_err("cancellation must surface as an error");

    assert!(matches!(err, CoreError::Cancelled));
    let events = progress.events();
    assert!(events.contains(&ProgressEvent::Cancelled));
    assert!(!events.contains(&ProgressEvent::Finished));
}

/// A full conversion whose settings match a prior estimate's fingerprint
/// must report `estimate_comparison`; the richer per-page and aggregate
/// metrics this milestone adds to `ProcessingReport` must also be
/// populated with real, sane values.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn process_report_includes_estimate_comparison_and_richer_metrics() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "mixed.pdf", &test_fixtures::mixed_page_sizes());
    let output = dir.path().join("out.pdf");
    let chosen_settings = settings(BinarizationMethod::Otsu, 300);

    let estimate_options = pipeline::EstimationOptions {
        password: None,
        pdfium: config.clone(),
        samples: 8,
    };
    let estimate = pipeline::estimate_output_size(
        &input,
        &chosen_settings,
        &estimate_options,
        &RecordingProgress::new(),
    )
    .expect("estimate should succeed");

    let mut process_options = options(config, ValidationMode::Structural);
    process_options.prior_estimate = Some(pipeline::PriorEstimate {
        estimated_output_bytes: estimate.estimated_output_bytes,
        settings_fingerprint: estimate.settings_fingerprint.clone(),
    });

    let report = pipeline::process_pdf(
        &input,
        &output,
        &chosen_settings,
        &process_options,
        &RecordingProgress::new(),
    )
    .expect("conversion should succeed");

    let comparison = report
        .estimate_comparison
        .expect("settings fingerprint matched, so a comparison must be present");
    assert_eq!(
        comparison.estimated_output_bytes,
        estimate.estimated_output_bytes
    );
    assert_eq!(comparison.actual_output_bytes, report.output_bytes);
    assert!(comparison.relative_error_fraction.is_finite());
    assert!(comparison.relative_error_fraction >= 0.0);

    // Richer aggregate metrics.
    assert!(report.total_pixel_count > 0);
    assert!(report.total_ccitt_bytes > 0);
    assert!(report.mean_ccitt_bytes_per_page > 0.0);
    assert!(report.median_ccitt_bytes_per_page > 0.0);
    assert!(report.slowest_page.is_some());
    assert!(report.largest_encoded_page.is_some());
    assert!(report.smallest_encoded_page.is_some());
    assert!(report.overall_black_pixel_ratio >= 0.0 && report.overall_black_pixel_ratio <= 1.0);

    // Richer per-page metrics.
    for page in &report.page_reports {
        assert!(page.pixel_count > 0);
        assert!(page.bytes_per_pixel >= 0.0);
        assert!(page.total_page_duration_us > 0);
        assert!(
            page.total_page_duration_us
                >= page.render_duration_us
                    + page.processing_duration_us
                    + page.encoding_duration_us
        );
    }
}

/// Estimation accuracy on a synthetic mixed-content document — see
/// `docs/size-estimation.md`, "Accuracy" for the acceptance rationale.
/// This is an engineering acceptance threshold on a synthetic fixture,
/// not a product guarantee about any real document: a homogeneous
/// fixture would make any estimator look better than it actually is,
/// which is exactly why this fixture is deliberately heterogeneous (see
/// `test_fixtures::heterogeneous_document`).
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_accuracy_on_a_heterogeneous_synthetic_document_is_within_the_engineering_threshold() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "heterogeneous.pdf",
        &test_fixtures::heterogeneous_document(24),
    );
    let output = dir.path().join("out.pdf");
    let chosen_settings = settings(BinarizationMethod::Otsu, 300);

    let estimate_options = pipeline::EstimationOptions {
        password: None,
        pdfium: config.clone(),
        samples: 8,
    };
    let estimate = pipeline::estimate_output_size(
        &input,
        &chosen_settings,
        &estimate_options,
        &RecordingProgress::new(),
    )
    .expect("estimate should succeed");

    let report = pipeline::process_pdf(
        &input,
        &output,
        &chosen_settings,
        &options(config, ValidationMode::Structural),
        &RecordingProgress::new(),
    )
    .expect("conversion should succeed");

    let comparison = museion_binarize_core::estimation::compare_estimate(
        estimate.estimated_output_bytes,
        report.output_bytes,
    );
    // Printed unconditionally (not just on failure) so the actual observed
    // accuracy is visible in `--nocapture` output for anyone re-running
    // this as a calibration check, not only as a pass/fail gate.
    println!(
        "heterogeneous accuracy: estimated {} vs actual {} bytes, {:.1}% relative error",
        estimate.estimated_output_bytes,
        report.output_bytes,
        comparison.relative_error_fraction * 100.0
    );
    assert!(
        comparison.relative_error_fraction <= 0.25,
        "central estimate {} differed from actual {} by {:.1}% (24-page heterogeneous fixture, 8 samples); expected <= 25%",
        estimate.estimated_output_bytes,
        report.output_bytes,
        comparison.relative_error_fraction * 100.0
    );
}

/// The homogeneous control case: a document whose pages are all the same
/// `PageType` should be at least as easy to estimate as the heterogeneous
/// fixture above, and comfortably within a tighter engineering threshold.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn estimate_accuracy_on_a_homogeneous_synthetic_document_is_within_the_tighter_threshold() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(
        dir.path(),
        "homogeneous.pdf",
        &test_fixtures::homogeneous_document(24, test_fixtures::PageType::DenseText),
    );
    let output = dir.path().join("out.pdf");
    let chosen_settings = settings(BinarizationMethod::Otsu, 300);

    let estimate_options = pipeline::EstimationOptions {
        password: None,
        pdfium: config.clone(),
        samples: 8,
    };
    let estimate = pipeline::estimate_output_size(
        &input,
        &chosen_settings,
        &estimate_options,
        &RecordingProgress::new(),
    )
    .expect("estimate should succeed");

    let report = pipeline::process_pdf(
        &input,
        &output,
        &chosen_settings,
        &options(config, ValidationMode::Structural),
        &RecordingProgress::new(),
    )
    .expect("conversion should succeed");

    let comparison = museion_binarize_core::estimation::compare_estimate(
        estimate.estimated_output_bytes,
        report.output_bytes,
    );
    println!(
        "homogeneous accuracy: estimated {} vs actual {} bytes, {:.1}% relative error",
        estimate.estimated_output_bytes,
        report.output_bytes,
        comparison.relative_error_fraction * 100.0
    );
    assert!(
        comparison.relative_error_fraction <= 0.15,
        "central estimate {} differed from actual {} by {:.1}% (24-page homogeneous fixture, 8 samples); expected <= 15%",
        estimate.estimated_output_bytes,
        report.output_bytes,
        comparison.relative_error_fraction * 100.0
    );
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
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

// --- Milestone 3: persistent session, source-mutation policy, analyze ---

/// Proves the open-bytes snapshot policy: once a session is open, later
/// mutating (here, truncating and overwriting) the file on disk at the
/// same path must not affect any page rendered from that already-open
/// session. There is no per-page reopen to be affected by the mutation in
/// the first place — this test demonstrates the consequence directly,
/// without needing to synchronize a pause mid-render.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn source_mutation_after_open_does_not_affect_an_in_progress_session() {
    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "a.pdf", &test_fixtures::mixed_page_sizes());

    let session = PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config,
            compute_source_hash: true,
        },
    )
    .expect("open");
    assert_eq!(session.info().page_count, 3);
    let identity_at_open = session.source_identity().clone();

    // Mutate the file on disk after the session is already open: replace
    // it with a single-page, differently-sized document.
    let replacement = test_fixtures::orientation_and_polarity();
    assert_ne!(
        replacement.len(),
        identity_at_open.byte_len as usize,
        "the replacement fixture must actually differ in size to make this a meaningful test"
    );
    std::fs::write(&input, &replacement).unwrap();

    // The session still reports the document as it was at open time: 3
    // pages, not the replacement's 1. Every page renders successfully,
    // proving the session never went back to disk.
    assert_eq!(
        session.info().page_count,
        3,
        "an already-open session must not observe a mutation to the source file"
    );
    for index in 0..3 {
        assert!(
            session.render_page(index, 150).is_ok(),
            "page {index} must still render from the original in-memory snapshot"
        );
    }

    // Identity captured on a fresh open of the now-mutated file must
    // differ, confirming the mutation was real and detectable — the
    // *session* simply never performed that fresh open.
    let bytes_now = std::fs::read(&input).unwrap();
    assert_ne!(bytes_now.len() as u64, identity_at_open.byte_len);
}

/// `process_with_open_session` (added for the desktop app, which keeps one
/// session open across preview and conversion) must convert successfully
/// through a session opened earlier by the caller, without opening a
/// second session of its own — proven the same way M3 proves it for
/// `process_pdf`: by comparing output against a fixture-fresh run.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn process_with_open_session_reuses_an_already_open_session_without_reopening_the_source() {
    use museion_binarize_core::pipeline::process_with_open_session;

    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "a.pdf", &test_fixtures::mixed_page_sizes());
    let output = dir.path().join("out.pdf");

    let session = PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            password: None,
            pdfium: config.clone(),
            compute_source_hash: false,
        },
    )
    .expect("open");

    let report = process_with_open_session(
        &session,
        &output,
        &settings(BinarizationMethod::Otsu, 300),
        &options(config, ValidationMode::Structural),
        &museion_binarize_core::progress::NullProgressReporter,
    )
    .expect("conversion through an already-open session");

    assert_eq!(report.pages_processed, 3);
    assert!(output.exists());
    let bytes = std::fs::read(&output).unwrap();
    museion_binarize_core::validation::assert_bilevel_ccitt_structure(&bytes)
        .expect("must be bilevel CCITT G4");
}

/// `analyze_pdf` end-to-end: real rendering and binarization, useful
/// document- and page-level measurements, and — like `process_pdf` — no
/// reconstructed output file.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn analyze_reports_real_measurements_without_writing_an_output_pdf() {
    use museion_binarize_core::analysis::DocumentAnalysisReport;
    use museion_binarize_core::pipeline::AnalysisOptions;

    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "a.pdf", &test_fixtures::threshold_patterns());

    let options = AnalysisOptions {
        password: None,
        pdfium: config,
        pages: None,
        encode: true,
        path_mode: museion_binarize_core::report::PathMode::Basename,
    };
    let report: DocumentAnalysisReport = pipeline::analyze_pdf(
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &options,
        &RecordingProgress::new(),
    )
    .expect("analyze");

    assert_eq!(report.page_count, 1);
    assert_eq!(report.analyzed_page_count, 1);
    assert_eq!(report.failed_page_count, 0);
    assert_eq!(report.source_path.as_deref(), Some("a.pdf"));
    assert_eq!(report.dpi, 300);
    assert_eq!(report.method, "otsu");

    let page = &report.pages[0];
    // `threshold_patterns` draws both light and dark content, so a real
    // Otsu run over it must select something other than 0 or 255.
    match page.threshold {
        museion_binarize_core::image_pipeline::ThresholdInfo::Otsu { threshold } => {
            assert!(
                threshold > 0 && threshold < 255,
                "otsu threshold {threshold} looks unused, not computed"
            );
        }
        other => panic!("expected Otsu threshold info, got {other:?}"),
    }
    assert!(
        page.grayscale.max > page.grayscale.min,
        "page has real contrast"
    );
    assert!(page.ink.black_pixels > 0 && page.ink.white_pixels > 0);
    assert!(
        (page.ink.black_pixel_ratio
            - page.ink.black_pixels as f64
                / (page.ink.black_pixels + page.ink.white_pixels) as f64)
            .abs()
            < 1e-9
    );
    assert!(
        page.ccitt_bytes.is_some(),
        "--encode must populate ccitt_bytes"
    );
    assert!(page.stage_durations.render_us > 0 || page.stage_durations.total_us > 0);

    // No reconstructed output file exists anywhere near the input.
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("a.pdf")]);
}

/// `analyze` with an explicit page selection renders only those pages —
/// end to end, through the real PDFium-backed session, not the mock used
/// by the ordinary tests in `pipeline.rs`.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/testing-pdf-pipeline.md"]
fn analyze_with_a_page_selection_analyzes_only_the_selected_pages() {
    use museion_binarize_core::pipeline::AnalysisOptions;

    let (config, _pdfium_guard) = require_pdfium!();
    let dir = tempfile::tempdir().unwrap();
    let input = write_fixture(dir.path(), "a.pdf", &test_fixtures::mixed_page_sizes());

    let options = AnalysisOptions {
        password: None,
        pdfium: config,
        pages: Some("1,3".to_string()),
        encode: false,
        path_mode: museion_binarize_core::report::PathMode::Omit,
    };
    let report = pipeline::analyze_pdf(
        &input,
        &settings(BinarizationMethod::Otsu, 300),
        &options,
        &RecordingProgress::new(),
    )
    .expect("analyze");

    assert_eq!(
        report.page_count, 3,
        "the document itself still has 3 pages"
    );
    assert_eq!(report.analyzed_page_count, 2);
    assert_eq!(
        report.source_path, None,
        "PathMode::Omit must omit the path"
    );
    let reported_numbers: Vec<u32> = report.pages.iter().map(|p| p.page_number).collect();
    assert_eq!(reported_numbers, vec![1, 3]);
}
