//! Synthetic MDP + OCR evidence builders for bookmark tests and examples.
//!
//! These construct exactly the typed records a real run produces — the same
//! `DocumentPackage` shape as `create_from_session` and the same `OcrRun`
//! shape as the M3 local pipeline or an M6 remote install — without needing
//! PDFium, a provider, or any copyrighted source material. Compiled into the
//! library so integration tests and the CLI's own tests share one definition.

use crate::document_package::{
    document_id_for_sha256, sha256_digest, source_id_for_sha256, AffineTransform, CoordinateSpace,
    CoordinateUnit, DocumentPackage, ExecutionKind, Manifest, Origin, Page, ProvenanceStep, Source,
    SourceKind, ToolInfo, MDP_SCHEMA, MDP_SCHEMA_VERSION,
};
use crate::ocr::{
    OcrBlock, OcrBox, OcrLine, OcrPage, OcrProviderProvenance, OcrRoute, OcrRouteReason, OcrRun,
    OcrWord, OCR_PROTOCOL, OCR_PROTOCOL_VERSION,
};

/// Master-space page size used by every synthetic fixture, in pixels at the
/// canonical 300 DPI master resolution. The PDF source space is the same
/// page in points.
pub const FIXTURE_PAGE_WIDTH: f64 = 600.0;
pub const FIXTURE_PAGE_HEIGHT: f64 = 800.0;
const POINTS_PER_PIXEL: f64 = 72.0 / 300.0;
pub const FIXTURE_SOURCE_WIDTH: f64 = FIXTURE_PAGE_WIDTH * POINTS_PER_PIXEL;
pub const FIXTURE_SOURCE_HEIGHT: f64 = FIXTURE_PAGE_HEIGHT * POINTS_PER_PIXEL;

/// One synthetic text line: text, top-left master position, and per-word
/// confidence.
#[derive(Debug, Clone)]
pub struct FixtureLine {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub confidence: f32,
}

impl FixtureLine {
    pub fn new(text: &str, x: f64, y: f64) -> Self {
        Self {
            width: (text.chars().count() as f64 * 6.0).min(FIXTURE_PAGE_WIDTH - x),
            text: text.to_owned(),
            x,
            y,
            height: 12.0,
            confidence: 0.97,
        }
    }
    pub fn with_width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }
    pub fn with_height(mut self, height: f64) -> Self {
        self.height = height;
        self
    }
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence;
        self
    }
}

/// One synthetic page of lines.
#[derive(Debug, Clone, Default)]
pub struct FixturePage {
    pub lines: Vec<FixtureLine>,
    pub native_text: bool,
}

impl FixturePage {
    pub fn new(lines: Vec<FixtureLine>) -> Self {
        Self {
            lines,
            native_text: false,
        }
    }
    /// Marks the page as taking the native-text route, whose line and word
    /// boxes are approximate geometry.
    pub fn native(mut self) -> Self {
        self.native_text = true;
        self
    }
}

/// A minimal, self-generated source PDF whose pages are exactly the
/// fixtures' source space. No external or copyrighted material is involved.
pub fn source_pdf(page_count: u32) -> Vec<u8> {
    use pdf_writer::{Content, Finish, Pdf, Rect, Ref};
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let mut next = 3i32;
    let mut page_ids = Vec::new();
    for index in 0..page_count {
        let page_id = Ref::new(next);
        next += 1;
        let content_id = Ref::new(next);
        next += 1;
        page_ids.push(page_id);
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(
            0.0,
            0.0,
            FIXTURE_SOURCE_WIDTH as f32,
            FIXTURE_SOURCE_HEIGHT as f32,
        ))
        .parent(tree)
        .contents(content_id);
        page.resources();
        page.finish();
        let mut content = Content::new();
        content.save_state();
        content.set_fill_gray(0.0);
        content.rect(10.0, 10.0 + (index % 3) as f32 * 5.0, 20.0, 20.0);
        content.fill_nonzero();
        content.restore_state();
        pdf.stream(content_id, &content.finish());
    }
    let count = page_ids.len() as i32;
    pdf.pages(tree).kids(page_ids).count(count);
    pdf.catalog(catalog).pages(tree);
    pdf.finish()
}

/// Builds a valid MDP package bound to real source bytes, so the searchable
/// writer's digest check can be exercised without PDFium.
pub fn package_for_source(bytes: &[u8], page_count: u32) -> DocumentPackage {
    package_with_digest(sha256_digest(bytes), bytes.len() as u64, page_count)
}

/// Builds a valid MDP package for `page_count` identical synthetic pages.
/// `seed` makes the source digest (and therefore every stable ID) distinct
/// between fixtures without touching any real file.
pub fn package(seed: &str, page_count: u32) -> DocumentPackage {
    package_with_digest(
        sha256_digest(seed.as_bytes()),
        seed.len() as u64,
        page_count,
    )
}

fn package_with_digest(digest: String, byte_len: u64, page_count: u32) -> DocumentPackage {
    let mut pages = Vec::with_capacity(page_count as usize);
    for index in 0..page_count {
        let master = CoordinateSpace {
            id: format!("page-{}-master", index + 1),
            unit: CoordinateUnit::Pixels,
            width: FIXTURE_PAGE_WIDTH,
            height: FIXTURE_PAGE_HEIGHT,
            origin: Origin::TopLeft,
            pixels_per_inch: Some(crate::document_package::CANONICAL_MASTER_DPI),
        };
        let source = CoordinateSpace {
            id: format!("page-{}-pdf", index + 1),
            unit: CoordinateUnit::PdfPoints,
            width: FIXTURE_SOURCE_WIDTH,
            height: FIXTURE_SOURCE_HEIGHT,
            origin: Origin::BottomLeft,
            pixels_per_inch: None,
        };
        pages.push(Page {
            page_id: crate::document_package::page_id_for_sha256(&digest, index),
            physical_index: index,
            order: index,
            rotation_degrees: 0,
            transforms: vec![AffineTransform {
                from_space: source.id.clone(),
                to_space: master.id.clone(),
                a: FIXTURE_PAGE_WIDTH / FIXTURE_SOURCE_WIDTH,
                b: 0.0,
                c: 0.0,
                d: -FIXTURE_PAGE_HEIGHT / FIXTURE_SOURCE_HEIGHT,
                e: 0.0,
                f: FIXTURE_PAGE_HEIGHT,
            }],
            master_space: master,
            source_space: source,
            printed_page_label: None,
            existing_outline_evidence: Vec::new(),
            typography_evidence: Vec::new(),
            region_evidence: Vec::new(),
            asset_ids: Vec::new(),
        });
    }
    let source = Source {
        source_id: source_id_for_sha256(&digest),
        kind: SourceKind::Pdf,
        content_sha256: digest.clone(),
        byte_len,
        page_count,
        external_reference: Some("fixture.pdf".to_owned()),
        packaged_path: None,
    };
    let mut package = DocumentPackage {
        manifest: Manifest {
            schema: MDP_SCHEMA.to_owned(),
            schema_version: MDP_SCHEMA_VERSION.to_owned(),
            document_id: document_id_for_sha256(&digest),
            source_id: source.source_id.clone(),
            page_count,
            asset_count: 0,
            tool: ToolInfo {
                name: "mpdf".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        provenance: vec![ProvenanceStep {
            step_id: "step-source-inspect".to_owned(),
            operation: "source_inspect".to_owned(),
            inputs: vec![source.source_id.clone()],
            outputs: pages.iter().map(|page| page.page_id.clone()).collect(),
            parameters: std::collections::BTreeMap::new(),
            software: "mpdf".to_owned(),
            software_version: env!("CARGO_PKG_VERSION").to_owned(),
            execution: ExecutionKind::Local,
        }],
        source,
        pages,
        assets: Vec::new(),
        validation: crate::document_package::empty_validation_summary(),
    };
    package.validate().expect("fixture package is valid");
    package.validation = package
        .validation_report()
        .expect("fixture validation report");
    package
}

/// Builds a typed OCR run over synthetic pages, exactly as either the local
/// route or an installed remote result would leave it in `ocr/`.
pub fn ocr_run(pages: &[FixturePage], provenance: Option<OcrProviderProvenance>) -> OcrRun {
    OcrRun {
        protocol: OCR_PROTOCOL.to_owned(),
        protocol_version: OCR_PROTOCOL_VERSION.to_owned(),
        pages: pages
            .iter()
            .enumerate()
            .map(|(index, page)| ocr_page(index as u32, page, provenance.clone()))
            .collect(),
        errors: Vec::new(),
    }
}

fn ocr_page(index: u32, page: &FixturePage, provenance: Option<OcrProviderProvenance>) -> OcrPage {
    let mut blocks = Vec::new();
    for (ordinal, line) in page.lines.iter().enumerate() {
        let words = split_words(line, ordinal as u32);
        let bbox = OcrBox {
            x: line.x as f32,
            y: line.y as f32,
            width: line.width as f32,
            height: line.height as f32,
        };
        blocks.push(OcrBlock {
            bbox: bbox.clone(),
            confidence: line.confidence,
            reading_order: ordinal as u32,
            lines: vec![OcrLine {
                bbox,
                confidence: line.confidence,
                reading_order: ordinal as u32,
                words,
            }],
        });
    }
    OcrPage {
        page_index: index,
        route: if page.native_text {
            OcrRoute::NativeText
        } else {
            OcrRoute::Ocr {
                reason: OcrRouteReason::MissingText,
            }
        },
        width: FIXTURE_PAGE_WIDTH as u32,
        height: FIXTURE_PAGE_HEIGHT as u32,
        blocks,
        revisions: Vec::new(),
        provider_provenance: provenance,
        provider_raw_artifact: None,
    }
}

fn split_words(line: &FixtureLine, reading_order: u32) -> Vec<OcrWord> {
    let tokens: Vec<&str> = line.text.split_whitespace().collect();
    let total: usize = tokens
        .iter()
        .map(|token| token.chars().count())
        .sum::<usize>()
        + tokens.len().saturating_sub(1);
    let mut words = Vec::with_capacity(tokens.len());
    let mut cursor = line.x;
    for (ordinal, token) in tokens.iter().enumerate() {
        let share = if total == 0 {
            0.0
        } else {
            line.width * token.chars().count() as f64 / total as f64
        };
        words.push(OcrWord {
            text: (*token).to_owned(),
            normalized_text: normalize(token),
            bbox: OcrBox {
                x: cursor as f32,
                y: line.y as f32,
                width: share as f32,
                height: line.height as f32,
            },
            confidence: line.confidence,
            reading_order: reading_order * 1_000 + ordinal as u32,
        });
        cursor += share
            + if total == 0 {
                0.0
            } else {
                line.width / total as f64
            };
    }
    words
}

fn normalize(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfc().collect()
}

/// A small, fully aligned book: a contents page plus body chapters whose
/// printed labels are offset from the physical pages by a constant.
pub fn aligned_book() -> (DocumentPackage, Vec<FixturePage>) {
    let titles = [
        ("Introduction", 1u32),
        ("The Wine Dark Sea", 12),
        ("Homeric Formulae", 23),
    ];
    let offset = 3u32; // printed page 1 is physical page 4 (index 3)
    let mut pages = vec![FixturePage::default(), FixturePage::default()];
    pages[0].lines = vec![FixtureLine::new("A Study in Fixtures", 200.0, 100.0)];
    pages[1].lines = {
        let mut lines = vec![FixtureLine::new("Contents", 60.0, 60.0)];
        for (ordinal, (title, printed)) in titles.iter().enumerate() {
            lines.push(
                FixtureLine::new(
                    &format!("{title} ......... {printed}"),
                    60.0,
                    120.0 + ordinal as f64 * 30.0,
                )
                .with_width(420.0),
            );
        }
        lines
    };
    let last_printed = titles
        .iter()
        .map(|(_, printed)| *printed)
        .max()
        .unwrap_or(1);
    for physical in 2..(offset + last_printed + 1) {
        let printed = physical as i64 - offset as i64;
        let mut lines = Vec::new();
        if let Some((title, _)) = titles.iter().find(|(_, page)| i64::from(*page) == printed) {
            lines.push(
                FixtureLine::new(title, 60.0, 80.0)
                    .with_height(20.0)
                    .with_width(300.0),
            );
        }
        lines.push(FixtureLine::new(
            &format!("body text of page {printed}"),
            60.0,
            300.0,
        ));
        if printed > 0 {
            lines.push(FixtureLine::new(&format!("{printed}"), 300.0, 760.0));
        }
        pages.push(FixturePage::new(lines));
    }
    let package = package("aligned-book", pages.len() as u32);
    (package, pages)
}
