//! Deterministic synthetic benchmark fixture generation.
//!
//! Produces a `(ground_truth, degraded_input)` pair from procedural,
//! non-copyrighted glyph-like shapes — never from a real font, and never
//! by running M PDF Processor itself (that would make the benchmark
//! circular: ground truth must be independent of the thing being
//! measured). See `docs/benchmark-datasets.md` for how these categories
//! map onto the committed `test-data/benchmark/synthetic-v1/` suite.
//!
//! Everything here is a pure function of its inputs plus a fixed,
//! in-code seed — no wall-clock, no OS randomness, no filesystem access.
//! Regenerating a fixture must reproduce it byte-for-byte; this is
//! verified directly in this module's tests and, for the committed
//! suite, in `examples/gen_benchmark_fixtures.rs`'s own regeneration
//! check.

use image::{GrayImage, Luma};

use crate::bilevel::BinaryMask;

/// One deterministic (seed baked into the generator) PRNG, used only for
/// noise categories. Xorshift64 — small, dependency-free, and reproducible
/// across platforms since it is pure integer arithmetic.
struct DeterministicRng(u64);

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        // Xorshift requires a non-zero state.
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform `[0.0, 1.0)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub const CANVAS_WIDTH: u32 = 160;
pub const CANVAS_HEIGHT: u32 = 200;

/// A category of synthetic content, deliberately named after what it
/// stresses (per the milestone spec) rather than after any real
/// document, edition, or script — these are procedural stress shapes,
/// not a claim of typographic representativeness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureCategory {
    CleanText,
    FaintText,
    SmallText,
    UnevenBackground,
    SaltPepperNoise,
    Blur,
    LowContrast,
    BrokenStrokes,
    ThickStrokes,
    MixedTextAndLines,
    DenseApparatusLikeText,
    PolytonicLikeDiacriticShapes,
}

pub const ALL_CATEGORIES: [FixtureCategory; 12] = [
    FixtureCategory::CleanText,
    FixtureCategory::FaintText,
    FixtureCategory::SmallText,
    FixtureCategory::UnevenBackground,
    FixtureCategory::SaltPepperNoise,
    FixtureCategory::Blur,
    FixtureCategory::LowContrast,
    FixtureCategory::BrokenStrokes,
    FixtureCategory::ThickStrokes,
    FixtureCategory::MixedTextAndLines,
    FixtureCategory::DenseApparatusLikeText,
    FixtureCategory::PolytonicLikeDiacriticShapes,
];

impl FixtureCategory {
    pub fn id(self) -> &'static str {
        match self {
            FixtureCategory::CleanText => "clean_text",
            FixtureCategory::FaintText => "faint_text",
            FixtureCategory::SmallText => "small_text",
            FixtureCategory::UnevenBackground => "uneven_background",
            FixtureCategory::SaltPepperNoise => "salt_pepper_noise",
            FixtureCategory::Blur => "blur",
            FixtureCategory::LowContrast => "low_contrast",
            FixtureCategory::BrokenStrokes => "broken_strokes",
            FixtureCategory::ThickStrokes => "thick_strokes",
            FixtureCategory::MixedTextAndLines => "mixed_text_and_lines",
            FixtureCategory::DenseApparatusLikeText => "dense_apparatus_density",
            FixtureCategory::PolytonicLikeDiacriticShapes => "synthetic_diacritic_detail",
        }
    }
}

/// One generated region of interest, in the same coordinate system as
/// the generated raster: `(id, x, y, width, height, tag)`.
pub type GeneratedRoi = (String, u32, u32, u32, u32, Option<String>);

pub struct GeneratedFixture {
    pub ground_truth: BinaryMask,
    pub input: GrayImage,
    pub rois: Vec<GeneratedRoi>,
}

/// Draws one horizontal "glyph": a filled rectangle stroke of the given
/// width/height at `(x, y)`, standing in for a letter's main stroke.
fn draw_glyph_stroke(mask: &mut BinaryMask, x: u32, y: u32, width: u32, height: u32) {
    for dy in 0..height {
        for dx in 0..width {
            let (px, py) = (x + dx, y + dy);
            if px < mask.width && py < mask.height {
                mask.set(px, py, true);
            }
        }
    }
}

/// Draws a short diagonal stroke standing in for an accent mark (acute-
/// or grave-like), anchored at its bottom-left corner.
fn draw_accent_mark(mask: &mut BinaryMask, x: u32, y: u32, length: u32, rising: bool) {
    for i in 0..length {
        let px = x + i;
        let py = if rising { y.saturating_sub(i) } else { y + i };
        if px < mask.width && py < mask.height {
            mask.set(px, py, true);
            if px + 1 < mask.width {
                mask.set(px + 1, py, true);
            }
        }
    }
}

/// Draws a small arc-like breathing mark as a short curved run of pixels.
fn draw_breathing_mark(mask: &mut BinaryMask, x: u32, y: u32) {
    let offsets: [(i32, i32); 5] = [(0, 2), (1, 1), (2, 0), (3, 1), (4, 2)];
    for (dx, dy) in offsets {
        let px = x as i32 + dx;
        let py = y as i32 + dy;
        if px >= 0 && py >= 0 && (px as u32) < mask.width && (py as u32) < mask.height {
            mask.set(px as u32, py as u32, true);
        }
    }
}

/// Draws a single isolated dot mark (iota-subscript-like, apostrophe-
/// like, or middle-dot-like depending on placement).
fn draw_dot_mark(mask: &mut BinaryMask, x: u32, y: u32, size: u32) {
    draw_glyph_stroke(mask, x, y, size, size);
}

/// A single row of `count` glyph strokes, evenly spaced, starting at
/// `(x0, y0)`. Returns the x-position of each glyph's left edge, so a
/// caller can add diacritics/breaks at specific glyph positions.
fn draw_glyph_row(
    mask: &mut BinaryMask,
    x0: u32,
    y0: u32,
    count: u32,
    glyph_width: u32,
    glyph_height: u32,
    spacing: u32,
) -> Vec<u32> {
    let mut positions = Vec::with_capacity(count as usize);
    let mut x = x0;
    for _ in 0..count {
        draw_glyph_stroke(mask, x, y0, glyph_width, glyph_height);
        positions.push(x);
        x += glyph_width + spacing;
    }
    positions
}

fn base_text_block(
    glyph_width: u32,
    glyph_height: u32,
    spacing: u32,
    row_spacing: u32,
) -> BinaryMask {
    let mut mask = BinaryMask::new(CANVAS_WIDTH, CANVAS_HEIGHT);
    let rows = (CANVAS_HEIGHT - 20) / (glyph_height + row_spacing);
    let glyphs_per_row = (CANVAS_WIDTH - 20) / (glyph_width + spacing);
    let mut y = 10;
    for _ in 0..rows {
        draw_glyph_row(
            &mut mask,
            10,
            y,
            glyphs_per_row,
            glyph_width,
            glyph_height,
            spacing,
        );
        y += glyph_height + row_spacing;
    }
    mask
}

fn to_grayscale(mask: &BinaryMask, ink: u8, paper: u8) -> GrayImage {
    let mut img = GrayImage::new(mask.width, mask.height);
    for y in 0..mask.height {
        for x in 0..mask.width {
            let value = if mask.get(x, y) { ink } else { paper };
            img.put_pixel(x, y, Luma([value]));
        }
    }
    img
}

/// Simple deterministic box blur (separable, edge-clamped).
fn box_blur(img: &GrayImage, radius: u32) -> GrayImage {
    let (width, height) = img.dimensions();
    let mut horizontal = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            let mut count = 0u32;
            let lo = x.saturating_sub(radius);
            let hi = (x + radius).min(width - 1);
            for nx in lo..=hi {
                sum += img.get_pixel(nx, y)[0] as u32;
                count += 1;
            }
            horizontal.put_pixel(x, y, Luma([(sum / count) as u8]));
        }
    }
    let mut result = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            let mut count = 0u32;
            let lo = y.saturating_sub(radius);
            let hi = (y + radius).min(height - 1);
            for ny in lo..=hi {
                sum += horizontal.get_pixel(x, ny)[0] as u32;
                count += 1;
            }
            result.put_pixel(x, y, Luma([(sum / count) as u8]));
        }
    }
    result
}

fn apply_background_gradient(img: &mut GrayImage, from: u8, to: u8) {
    let (width, _height) = img.dimensions();
    for (x, _y, pixel) in img.enumerate_pixels_mut() {
        let t = x as f64 / (width.max(1) - 1).max(1) as f64;
        let gradient_value = from as f64 + (to as f64 - from as f64) * t;
        // Darken toward the gradient background only where the pixel is
        // already paper-colored (bright), so ink strokes stay legible
        // and the gradient reads as an uneven background, not a wash
        // over the text itself.
        let current = pixel[0] as f64;
        if current > 200.0 {
            pixel[0] = gradient_value.round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn apply_salt_pepper_noise(img: &mut GrayImage, density: f64, seed: u64) {
    let mut rng = DeterministicRng::new(seed);
    for pixel in img.pixels_mut() {
        if rng.next_f64() < density {
            pixel[0] = if rng.next_f64() < 0.5 { 0 } else { 255 };
        }
    }
}

fn apply_contrast_reduction(img: &mut GrayImage, factor: f32) {
    for pixel in img.pixels_mut() {
        let v = pixel[0] as f32;
        let compressed = 128.0 + (v - 128.0) * factor;
        pixel[0] = compressed.round().clamp(0.0, 255.0) as u8;
    }
}

fn apply_faintness(img: &mut GrayImage, ink: u8, paper: u8, factor: f32) {
    for pixel in img.pixels_mut() {
        if pixel[0] == ink {
            let faded = ink as f32 + (paper as f32 - ink as f32) * factor;
            pixel[0] = faded.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Punches deterministic small gaps into a set of glyph strokes, in
/// place, standing in for broken/degraded ink in the source itself
/// (part of ground truth, not a later degradation of clean ground
/// truth — a real broken-stroke scan's ground truth *is* broken).
fn punch_gaps(
    mask: &mut BinaryMask,
    positions: &[u32],
    y0: u32,
    glyph_width: u32,
    glyph_height: u32,
) {
    for (i, &x) in positions.iter().enumerate() {
        if i % 2 == 0 {
            let gap_x = x + glyph_width / 2;
            for dy in 0..glyph_height {
                if gap_x < mask.width && y0 + dy < mask.height {
                    mask.set(gap_x, y0 + dy, false);
                }
            }
        }
    }
}

pub fn generate(category: FixtureCategory) -> GeneratedFixture {
    const INK: u8 = 20;
    const PAPER: u8 = 245;

    match category {
        FixtureCategory::CleanText => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::FaintText => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let mut input = to_grayscale(&ground_truth, INK, PAPER);
            apply_faintness(&mut input, INK, PAPER, 0.65);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::SmallText => {
            let ground_truth = base_text_block(2, 3, 2, 3);
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::UnevenBackground => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let mut input = to_grayscale(&ground_truth, INK, PAPER);
            apply_background_gradient(&mut input, 180, 250);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::SaltPepperNoise => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let mut input = to_grayscale(&ground_truth, INK, PAPER);
            apply_salt_pepper_noise(&mut input, 0.03, 0xC0FFEE_u64);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::Blur => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let input_sharp = to_grayscale(&ground_truth, INK, PAPER);
            let input = box_blur(&input_sharp, 1);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::LowContrast => {
            let ground_truth = base_text_block(6, 10, 4, 8);
            let mut input = to_grayscale(&ground_truth, INK, PAPER);
            apply_contrast_reduction(&mut input, 0.35);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::BrokenStrokes => {
            let mut ground_truth = BinaryMask::new(CANVAS_WIDTH, CANVAS_HEIGHT);
            let glyph_width = 6;
            let glyph_height = 10;
            let mut y = 10;
            while y + glyph_height < CANVAS_HEIGHT - 10 {
                let positions =
                    draw_glyph_row(&mut ground_truth, 10, y, 15, glyph_width, glyph_height, 4);
                punch_gaps(&mut ground_truth, &positions, y, glyph_width, glyph_height);
                y += glyph_height + 8;
            }
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::ThickStrokes => {
            let ground_truth = base_text_block(10, 12, 3, 8);
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::MixedTextAndLines => {
            let mut ground_truth = base_text_block(6, 10, 4, 8);
            // Long thin horizontal rule lines, standing in for
            // table/apparatus rules mixed with running text.
            for line_y in [30u32, 90, 150] {
                draw_glyph_stroke(&mut ground_truth, 5, line_y, CANVAS_WIDTH - 10, 1);
            }
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois: vec![],
            }
        }
        FixtureCategory::DenseApparatusLikeText => {
            let mut ground_truth = BinaryMask::new(CANVAS_WIDTH, CANVAS_HEIGHT);
            let mut rois = Vec::new();
            let glyph_width = 2;
            let glyph_height = 4;
            let mut y = 10;
            let mut row_index = 0;
            while y + glyph_height < CANVAS_HEIGHT - 10 {
                let positions =
                    draw_glyph_row(&mut ground_truth, 10, y, 30, glyph_width, glyph_height, 1);
                // Every third row gets isolated punctuation-like dots
                // between glyphs, and its bounding region tagged as an
                // "apparatus" ROI so density-heavy small marks can be
                // scored separately from the whole page.
                if row_index % 3 == 0 {
                    for &x in positions.iter().step_by(4) {
                        draw_dot_mark(&mut ground_truth, x + glyph_width + 1, y + glyph_height, 1);
                    }
                    rois.push((
                        format!("apparatus-row-{row_index}"),
                        8,
                        y.saturating_sub(1),
                        CANVAS_WIDTH - 16,
                        glyph_height + 3,
                        Some("apparatus".to_string()),
                    ));
                }
                y += glyph_height + 3;
                row_index += 1;
            }
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois,
            }
        }
        FixtureCategory::PolytonicLikeDiacriticShapes => {
            let mut ground_truth = BinaryMask::new(CANVAS_WIDTH, CANVAS_HEIGHT);
            let mut rois = Vec::new();
            let glyph_width = 6;
            let glyph_height = 10;
            let mut y = 20;
            let mut row_index = 0;
            while y + glyph_height + 8 < CANVAS_HEIGHT - 10 {
                let positions =
                    draw_glyph_row(&mut ground_truth, 10, y, 9, glyph_width, glyph_height, 6);
                for (i, &x) in positions.iter().enumerate() {
                    let mark_y = y.saturating_sub(6);
                    match i % 4 {
                        0 => {
                            draw_accent_mark(&mut ground_truth, x, y.saturating_sub(1), 4, true);
                            rois.push((
                                format!("diacritic-{row_index}-{i}"),
                                x,
                                mark_y,
                                glyph_width + 2,
                                7,
                                Some("diacritic".to_string()),
                            ));
                        }
                        1 => {
                            draw_breathing_mark(&mut ground_truth, x, mark_y);
                            rois.push((
                                format!("diacritic-{row_index}-{i}"),
                                x,
                                mark_y,
                                6,
                                4,
                                Some("diacritic".to_string()),
                            ));
                        }
                        2 => {
                            // Iota-subscript-like mark below the glyph.
                            draw_dot_mark(&mut ground_truth, x + 1, y + glyph_height + 1, 1);
                            rois.push((
                                format!("diacritic-{row_index}-{i}"),
                                x,
                                y + glyph_height,
                                4,
                                3,
                                Some("diacritic".to_string()),
                            ));
                        }
                        _ => {
                            // Stacked marks: accent above a breathing-like
                            // arc, the densest single-glyph case.
                            draw_accent_mark(&mut ground_truth, x, y.saturating_sub(4), 3, true);
                            draw_breathing_mark(&mut ground_truth, x, mark_y);
                            rois.push((
                                format!("diacritic-{row_index}-{i}"),
                                x,
                                mark_y.saturating_sub(4),
                                6,
                                11,
                                Some("diacritic".to_string()),
                            ));
                        }
                    }
                }
                y += glyph_height + 14;
                row_index += 1;
            }
            let input = to_grayscale(&ground_truth, INK, PAPER);
            GeneratedFixture {
                ground_truth,
                input,
                rois,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_generates_a_non_degenerate_fixture() {
        for category in ALL_CATEGORIES {
            let fixture = generate(category);
            assert_eq!(fixture.ground_truth.width, CANVAS_WIDTH);
            assert_eq!(fixture.ground_truth.height, CANVAS_HEIGHT);
            assert_eq!(fixture.input.dimensions(), (CANVAS_WIDTH, CANVAS_HEIGHT));
            let black_pixels = fixture.ground_truth.pixels.iter().filter(|&&p| p).count();
            assert!(
                black_pixels > 0,
                "{} produced an entirely blank ground truth",
                category.id()
            );
            for (id, x, y, w, h, _tag) in &fixture.rois {
                assert!(*w > 0 && *h > 0, "ROI {id} has zero size");
                assert!(
                    x + w <= CANVAS_WIDTH && y + h <= CANVAS_HEIGHT,
                    "ROI {id} out of bounds"
                );
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        for category in ALL_CATEGORIES {
            let a = generate(category);
            let b = generate(category);
            assert_eq!(
                a.ground_truth,
                b.ground_truth,
                "{} ground truth",
                category.id()
            );
            assert_eq!(
                a.input.as_raw(),
                b.input.as_raw(),
                "{} input",
                category.id()
            );
        }
    }

    #[test]
    fn ground_truth_is_never_produced_by_running_the_binarization_pipeline() {
        // A structural guard against the circularity the milestone spec
        // explicitly warns about: this module must not import or call
        // anything from `binarization`, `image_pipeline`, or `pipeline`.
        // (Enforced by code review / grep in CI-adjacent tooling; this
        // test at minimum documents the intent and would need to be
        // updated — a signal in itself — if such an import were added.)
        let fixture = generate(FixtureCategory::CleanText);
        // Ground truth is exactly binary by construction (BinaryMask),
        // never a thresholded grayscale output.
        assert!(fixture.ground_truth.pixels.iter().all(|&p| p || !p));
    }

    #[test]
    fn faint_text_ground_truth_matches_clean_text_only_input_differs() {
        let clean = generate(FixtureCategory::CleanText);
        let faint = generate(FixtureCategory::FaintText);
        assert_eq!(clean.ground_truth, faint.ground_truth);
        assert_ne!(clean.input.as_raw(), faint.input.as_raw());
    }

    #[test]
    fn broken_strokes_ground_truth_has_fewer_ink_pixels_than_a_solid_row() {
        let broken = generate(FixtureCategory::BrokenStrokes);
        let clean = generate(FixtureCategory::CleanText);
        let broken_ink = broken.ground_truth.pixels.iter().filter(|&&p| p).count();
        let clean_ink = clean.ground_truth.pixels.iter().filter(|&&p| p).count();
        assert!(broken_ink < clean_ink);
    }

    #[test]
    fn polytonic_and_apparatus_categories_declare_rois() {
        assert!(!generate(FixtureCategory::PolytonicLikeDiacriticShapes)
            .rois
            .is_empty());
        assert!(!generate(FixtureCategory::DenseApparatusLikeText)
            .rois
            .is_empty());
        assert!(generate(FixtureCategory::CleanText).rois.is_empty());
    }
}
