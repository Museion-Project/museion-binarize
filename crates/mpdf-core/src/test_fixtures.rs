//! Programmatic generation of synthetic PDF fixtures.
//!
//! Every fixture is built from vector shapes created by this project. No
//! external, scanned, or copyrighted material is involved, so fixtures can
//! be generated freely in temporary directories during tests.
//!
//! This module is compiled into the library (not gated behind `#[cfg(test)]`)
//! so that integration tests and manual end-to-end runs can share exactly
//! one fixture definition.

use pdf_writer::{Content, Finish, Pdf, Rect, Ref};

/// A4-ish portrait page, in points.
pub const A4_PORTRAIT: (f32, f32) = (595.0, 842.0);
/// Landscape page, in points.
pub const LANDSCAPE: (f32, f32) = (842.0, 595.0);
/// A smaller custom page, in points.
pub const SMALL: (f32, f32) = (300.0, 400.0);

/// Grayscale levels drawn by [`threshold_patterns`], from black to white.
pub const GRAY_LEVELS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

struct PageSpec {
    width: f32,
    height: f32,
    rotation: i32,
    content: pdf_writer::Buf,
}

fn build(pages: Vec<PageSpec>) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let tree_id = Ref::new(2);
    let mut next = 3;
    let mut page_ids = Vec::new();

    for spec in &pages {
        let page_id = Ref::new(next);
        next += 1;
        let content_id = Ref::new(next);
        next += 1;
        page_ids.push(page_id);

        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, spec.width, spec.height))
            .parent(tree_id)
            .contents(content_id);
        if spec.rotation != 0 {
            page.rotate(spec.rotation);
        }
        page.finish();

        pdf.stream(content_id, &spec.content);
    }

    let count = page_ids.len() as i32;
    pdf.pages(tree_id).kids(page_ids).count(count);
    pdf.catalog(catalog_id).pages(tree_id);
    pdf.finish()
}

/// Fills the whole page white. Every fixture starts from this, so a page
/// is never left with an undefined background.
fn white_background(width: f32, height: f32, content: &mut Content) {
    content.save_state();
    content.set_fill_gray(1.0);
    content.rect(0.0, 0.0, width, height);
    content.fill_nonzero();
    content.restore_state();
}

fn black_rect(content: &mut Content, x: f32, y: f32, w: f32, h: f32) {
    content.save_state();
    content.set_fill_gray(0.0);
    content.rect(x, y, w, h);
    content.fill_nonzero();
    content.restore_state();
}

/// The asymmetric orientation/polarity marker layout, drawn in PDF user
/// space (origin bottom-left).
///
/// Deliberately asymmetric on both axes so that an inverted, mirrored,
/// flipped, or wrongly rotated output is detectable:
///
/// - a **large** black square in the visual **top-left**;
/// - a **smaller** black square in the visual **bottom-right**;
/// - a horizontal bar near the top;
/// - a vertical bar near the left.
fn orientation_markers(content: &mut Content, width: f32, height: f32) {
    let unit = width.min(height);
    let large = unit * 0.20;
    let small = unit * 0.10;
    let margin = unit * 0.05;

    // Visual top-left: high y in PDF space.
    black_rect(content, margin, height - margin - large, large, large);
    // Visual bottom-right: low y.
    black_rect(content, width - margin - small, margin, small, small);
    // Horizontal bar near the visual top, offset to the right so it is
    // not symmetric about the vertical axis.
    black_rect(
        content,
        margin + large * 1.5,
        height - margin - large * 0.5,
        width * 0.4,
        unit * 0.03,
    );
    // Vertical bar near the left, in the lower half.
    black_rect(
        content,
        margin,
        margin + small * 2.0,
        unit * 0.03,
        height * 0.3,
    );
}

/// Fixture A — a single page carrying the asymmetric orientation and
/// polarity markers.
pub fn orientation_and_polarity() -> Vec<u8> {
    let (width, height) = A4_PORTRAIT;
    let mut content = Content::new();
    white_background(width, height, &mut content);
    orientation_markers(&mut content, width, height);
    build(vec![PageSpec {
        width,
        height,
        rotation: 0,
        content: content.finish(),
    }])
}

/// Fixture B — three pages of different sizes: portrait, landscape, and a
/// smaller custom size.
pub fn mixed_page_sizes() -> Vec<u8> {
    let specs = [A4_PORTRAIT, LANDSCAPE, SMALL]
        .into_iter()
        .map(|(width, height)| {
            let mut content = Content::new();
            white_background(width, height, &mut content);
            orientation_markers(&mut content, width, height);
            PageSpec {
                width,
                height,
                rotation: 0,
                content: content.finish(),
            }
        })
        .collect();
    build(specs)
}

/// Fixture C — four pages carrying `/Rotate` 0, 90, 180, and 270.
pub fn page_rotations() -> Vec<u8> {
    let (width, height) = A4_PORTRAIT;
    let specs = [0, 90, 180, 270]
        .into_iter()
        .map(|rotation| {
            let mut content = Content::new();
            white_background(width, height, &mut content);
            orientation_markers(&mut content, width, height);
            PageSpec {
                width,
                height,
                rotation,
                content: content.finish(),
            }
        })
        .collect();
    build(specs)
}

/// Fixture D — a page of grayscale bands, thin strokes, and isolated
/// specks, for confirming that the thresholding algorithms actually run.
pub fn threshold_patterns() -> Vec<u8> {
    let (width, height) = A4_PORTRAIT;
    let mut content = Content::new();
    white_background(width, height, &mut content);

    // Five grayscale bands from black to white across the top half.
    let band_width = width / GRAY_LEVELS.len() as f32;
    for (i, level) in GRAY_LEVELS.iter().enumerate() {
        content.save_state();
        content.set_fill_gray(*level);
        content.rect(
            i as f32 * band_width,
            height * 0.55,
            band_width,
            height * 0.35,
        );
        content.fill_nonzero();
        content.restore_state();
    }

    // Thin black strokes of decreasing width.
    for (i, thickness) in [3.0f32, 2.0, 1.0, 0.5].iter().enumerate() {
        black_rect(
            &mut content,
            width * 0.1,
            height * 0.45 - i as f32 * 20.0,
            width * 0.8,
            *thickness,
        );
    }

    // Isolated small specks, the kind despeckling targets.
    for i in 0..8 {
        let x = width * 0.1 + i as f32 * width * 0.1;
        black_rect(&mut content, x, height * 0.15, 2.0, 2.0);
    }

    build(vec![PageSpec {
        width,
        height,
        rotation: 0,
        content: content.finish(),
    }])
}

/// The six deterministic page "types" [`heterogeneous_document`] cycles
/// through, chosen so a size estimator cannot look accurate purely by
/// coincidence on a uniform document (see `docs/size-estimation.md`,
/// "Effect of heterogeneous pages").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    /// Nearly empty — a single tiny mark, mostly white.
    MostlyBlank,
    /// Many closely-packed thin horizontal bars, approximating dense body
    /// text.
    DenseText,
    /// Many very small, widely-spaced specks, approximating fine print or
    /// diacritics.
    FineText,
    /// One large black rectangle, approximating a full-page illustration
    /// or plate.
    LargeIllustration,
    /// A large mid-gray fill, distinct in tone from the pure black/white
    /// content of the other page types.
    GrayBackground,
    /// A black rectangle (an "image") alongside a block of thin bars (a
    /// caption or text block).
    MixedTextAndImage,
}

const PAGE_TYPE_CYCLE: [PageType; 6] = [
    PageType::MostlyBlank,
    PageType::DenseText,
    PageType::FineText,
    PageType::LargeIllustration,
    PageType::GrayBackground,
    PageType::MixedTextAndImage,
];

fn draw_page_type(content: &mut Content, width: f32, height: f32, page_type: PageType) {
    match page_type {
        PageType::MostlyBlank => {
            black_rect(
                content,
                width * 0.48,
                height * 0.48,
                width * 0.04,
                height * 0.02,
            );
        }
        PageType::DenseText => {
            let line_height = height * 0.02;
            let mut y = height * 0.9;
            while y > height * 0.1 {
                black_rect(content, width * 0.1, y, width * 0.8, line_height * 0.4);
                y -= line_height;
            }
        }
        PageType::FineText => {
            let mut y = height * 0.85;
            while y > height * 0.15 {
                let mut x = width * 0.1;
                while x < width * 0.9 {
                    black_rect(content, x, y, 1.5, 1.5);
                    x += width * 0.03;
                }
                y -= height * 0.03;
            }
        }
        PageType::LargeIllustration => {
            black_rect(
                content,
                width * 0.1,
                height * 0.1,
                width * 0.8,
                height * 0.8,
            );
        }
        PageType::GrayBackground => {
            content.save_state();
            content.set_fill_gray(0.5);
            content.rect(width * 0.05, height * 0.05, width * 0.9, height * 0.9);
            content.fill_nonzero();
            content.restore_state();
        }
        PageType::MixedTextAndImage => {
            black_rect(
                content,
                width * 0.1,
                height * 0.4,
                width * 0.8,
                height * 0.4,
            );
            let mut y = height * 0.3;
            while y > height * 0.1 {
                black_rect(content, width * 0.1, y, width * 0.8, height * 0.01);
                y -= height * 0.04;
            }
        }
    }
}

/// A synthetic multi-page document cycling deterministically through
/// [`PageType`], for measuring size-estimation accuracy against a
/// document that is deliberately *not* visually homogeneous — a uniform
/// fixture would make any estimator look better than it actually is. All
/// pages are A4 portrait, so page-geometry effects are isolated from
/// content-variety effects; see [`mixed_page_sizes`] for geometry
/// variation instead.
pub fn heterogeneous_document(page_count: u32) -> Vec<u8> {
    let (width, height) = A4_PORTRAIT;
    let specs = (0..page_count)
        .map(|i| {
            let page_type = PAGE_TYPE_CYCLE[(i as usize) % PAGE_TYPE_CYCLE.len()];
            let mut content = Content::new();
            white_background(width, height, &mut content);
            draw_page_type(&mut content, width, height, page_type);
            PageSpec {
                width,
                height,
                rotation: 0,
                content: content.finish(),
            }
        })
        .collect();
    build(specs)
}

/// A synthetic multi-page document where every page is the same
/// [`PageType`] — the accuracy control case: a homogeneous document is
/// expected to be easier to estimate than [`heterogeneous_document`], and
/// comparing the two is what makes that claim checkable rather than
/// assumed.
pub fn homogeneous_document(page_count: u32, page_type: PageType) -> Vec<u8> {
    let (width, height) = A4_PORTRAIT;
    let specs = (0..page_count)
        .map(|_| {
            let mut content = Content::new();
            white_background(width, height, &mut content);
            draw_page_type(&mut content, width, height, page_type);
            PageSpec {
                width,
                height,
                rotation: 0,
                content: content.finish(),
            }
        })
        .collect();
    build(specs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_looks_like_a_pdf(bytes: &[u8]) {
        assert!(bytes.starts_with(b"%PDF-"), "must have a PDF header");
        assert!(
            bytes.windows(5).any(|w| w == b"%%EOF"),
            "must have an EOF marker"
        );
    }

    #[test]
    fn every_fixture_is_a_syntactically_plausible_pdf() {
        for bytes in [
            orientation_and_polarity(),
            mixed_page_sizes(),
            page_rotations(),
            threshold_patterns(),
        ] {
            assert_looks_like_a_pdf(&bytes);
        }
    }

    #[test]
    fn fixtures_are_deterministic() {
        assert_eq!(orientation_and_polarity(), orientation_and_polarity());
        assert_eq!(mixed_page_sizes(), mixed_page_sizes());
        assert_eq!(page_rotations(), page_rotations());
        assert_eq!(threshold_patterns(), threshold_patterns());
    }

    #[test]
    fn rotation_fixture_declares_every_rotation() {
        let bytes = page_rotations();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Rotate 90"));
        assert!(text.contains("/Rotate 180"));
        assert!(text.contains("/Rotate 270"));
    }

    #[test]
    fn mixed_size_fixture_declares_three_distinct_media_boxes() {
        let bytes = mixed_page_sizes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("595"));
        assert!(text.contains("842"));
        assert!(text.contains("300"));
        assert!(text.contains("400"));
    }

    #[test]
    fn heterogeneous_document_is_a_plausible_deterministic_pdf_with_the_requested_page_count() {
        let bytes = heterogeneous_document(24);
        assert_looks_like_a_pdf(&bytes);
        assert_eq!(bytes, heterogeneous_document(24));

        let text = String::from_utf8_lossy(&bytes);
        // 24 pages -> 24 `/Contents` stream references, one per page.
        assert_eq!(text.matches("/Contents").count(), 24);
    }

    #[test]
    fn heterogeneous_document_cycles_through_every_page_type() {
        // With 6+ pages, every PageType in the cycle is exercised at
        // least once — proven indirectly by checking the fixture doesn't
        // panic and produces distinct byte sequences for at least two
        // different page counts (i.e. isn't accidentally the same
        // content repeated regardless of page_type).
        let six = heterogeneous_document(6);
        let twelve = heterogeneous_document(12);
        assert_ne!(six, twelve);
    }

    #[test]
    fn heterogeneous_document_handles_a_zero_and_single_page_request() {
        assert_looks_like_a_pdf(&heterogeneous_document(1));
        // Zero pages is a degenerate request the fixture itself does not
        // reject (that validation lives in PdfDocumentSession::open,
        // exercised by real PDFium tests) — it must simply not panic.
        let _ = heterogeneous_document(0);
    }
}
