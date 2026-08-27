use std::collections::BTreeMap;
use std::fs;

use mpdf_core::bookmarks::{self, BookmarkReviews, ReviewAction};
use mpdf_core::derived::DerivedDocument;
use mpdf_core::document_package::{
    DocumentPackage, ExistingOutlineEvidence, PrintedLabelSource, PrintedPageLabel, Rect,
    RegionEvidence, TypographyEvidence,
};
use mpdf_core::document_session::{DocumentSession, PdfDocumentSession, PdfOpenOptions};
use mpdf_core::ocr::{
    OcrBlock, OcrBox, OcrLine, OcrPage, OcrProviderProvenance, OcrRoute, OcrRouteReason, OcrRun,
    OcrWord, OCR_PROTOCOL, OCR_PROTOCOL_VERSION,
};
use mpdf_core::searchable_pdf;

fn ocr_fixture(package: &DocumentPackage) -> OcrRun {
    let pages = package
        .pages
        .iter()
        .map(|page| {
            let (text, confidence) = match page.physical_index {
                0 => (Some("1. Ἀρχὴ"), 0.98),
                1 => (Some("1.1 Πολιτείας"), 0.60),
                2 => (Some("2. Appendix iv"), 0.95),
                _ => (None, 0.98),
            };
            let blocks = text
                .map(|text| {
                    let bbox = OcrBox {
                        x: 50.0,
                        y: 80.0,
                        width: 600.0,
                        height: 90.0,
                    };
                    vec![OcrBlock {
                        bbox: bbox.clone(),
                        confidence,
                        reading_order: 0,
                        lines: vec![OcrLine {
                            bbox: bbox.clone(),
                            confidence,
                            reading_order: 0,
                            words: vec![OcrWord {
                                text: text.into(),
                                normalized_text: text.into(),
                                bbox,
                                confidence,
                                reading_order: 0,
                            }],
                        }],
                    }]
                })
                .unwrap_or_default();
            OcrPage {
                page_index: page.physical_index,
                route: OcrRoute::Ocr {
                    reason: OcrRouteReason::MissingText,
                },
                width: page.master_space.width.round() as u32,
                height: page.master_space.height.round() as u32,
                blocks,
                revisions: vec![],
                provider_provenance: Some(OcrProviderProvenance {
                    engine: "reference".into(),
                    model: "m5-integration".into(),
                    version: "0.1".into(),
                    parameters: BTreeMap::new(),
                    input_asset_sha256: "a".repeat(64),
                    execution_location: "local".into(),
                }),
                provider_raw_artifact: Some("m5-integration".into()),
            }
        })
        .collect();
    OcrRun {
        protocol: OCR_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        pages,
        errors: vec![],
    }
}

#[test]
fn m5_pdfium_source_preserving_reopen_and_rotation_fixture(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(lib) = std::env::var_os("MPDF_PDFIUM_LIBRARY") else {
        return Ok(());
    };
    let config = mpdf_core::pdfium_backend::PdfiumConfig {
        library_path: Some(lib.into()),
        allow_system_library: false,
    };
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("rotations.pdf");
    fs::write(&source, mpdf_core::test_fixtures::page_rotations())?;
    let session = PdfDocumentSession::open(
        &source,
        &PdfOpenOptions {
            compute_source_hash: true,
            pdfium: config.clone(),
            password: None,
        },
    )?;
    let before_renders = (0..session.info().page_count)
        .map(|index| session.render_page(index, 72).map(|image| image.to_rgba8()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut package = DocumentPackage::create_from_session(&session, Some("rotations.pdf".into()))?;
    let mut outline_package = package.clone();
    let outline_target = outline_package.pages[1].page_id.clone();
    outline_package.pages[0]
        .existing_outline_evidence
        .push(ExistingOutlineEvidence {
            title: "Πρόλογος".into(),
            level: 0,
            target_page_id: Some(outline_target),
            source: "source-pdf".into(),
        });
    let imported = bookmarks::generate(&outline_package, None)?;
    assert_eq!(imported.candidates.len(), 1);
    assert_eq!(imported.candidates[0].source_title, "Πρόλογος");
    assert_eq!(imported.candidates[0].source_level, 0);
    assert_eq!(imported.candidates[0].physical_page_index, 1);
    assert_eq!(
        imported.candidates[0]
            .outline_evidence
            .as_ref()
            .unwrap()
            .source,
        "source-pdf"
    );
    for page in package.pages.iter_mut().take(2) {
        page.typography_evidence.push(TypographyEvidence {
            role: "heading".into(),
            bounds: Rect {
                space_id: page.master_space.id.clone(),
                x: 50.0,
                y: 80.0,
                width: 600.0,
                height: 90.0,
            },
            font_size_points: Some(if page.physical_index == 0 { 20.0 } else { 16.0 }),
        });
    }
    let toc_space = package.pages[2].master_space.id.clone();
    package.pages[2].region_evidence.push(RegionEvidence {
        kind: "table_of_contents".into(),
        bounds: Rect {
            space_id: toc_space,
            x: 50.0,
            y: 80.0,
            width: 600.0,
            height: 90.0,
        },
    });
    package.pages[3].printed_page_label = Some(PrintedPageLabel {
        label: "iv".into(),
        source: PrintedLabelSource::Observed,
    });
    package.validate()?;
    let mut repeated_package = package.clone();
    for page in &mut repeated_package.pages {
        page.region_evidence.clear();
        page.printed_page_label = None;
        page.typography_evidence = vec![TypographyEvidence {
            role: "heading".into(),
            bounds: Rect {
                space_id: page.master_space.id.clone(),
                x: 50.0,
                y: 80.0,
                width: 600.0,
                height: 90.0,
            },
            font_size_points: Some(18.0),
        }];
    }
    let mut repeated_ocr = ocr_fixture(&repeated_package);
    let repeated_block = repeated_ocr.pages[0].blocks[0].clone();
    for page in &mut repeated_ocr.pages {
        page.blocks = vec![repeated_block.clone()];
        let word = &mut page.blocks[0].lines[0].words[0];
        word.text = "M PDF Header".into();
        word.normalized_text = "M PDF Header".into();
        word.confidence = 0.99;
    }
    let repeated_derived = DerivedDocument::from_package(&repeated_package, Some(&repeated_ocr))?;
    let repeated_snapshot = bookmarks::generate(&repeated_package, Some(&repeated_derived))?;
    assert_eq!(repeated_snapshot.candidates.len(), 4);
    assert!(repeated_snapshot.candidates.iter().all(|candidate| {
        candidate.status == bookmarks::BookmarkStatus::NeedsReview
            && candidate
                .reason_codes
                .iter()
                .any(|reason| reason == "repeated_header_footer_suppressed")
    }));
    let ocr = ocr_fixture(&package);
    ocr.validate()?;
    let derived = DerivedDocument::from_package(&package, Some(&ocr))?;
    let snapshot = bookmarks::generate(&package, Some(&derived))?;
    assert_eq!(snapshot, bookmarks::generate(&package, Some(&derived))?);
    assert_eq!(snapshot.candidates.len(), 3);
    assert_eq!(snapshot.candidates[0].source_level, 1);
    assert_eq!(snapshot.candidates[1].source_level, 2);
    assert_eq!(
        snapshot.candidates[1].status,
        bookmarks::BookmarkStatus::NeedsReview
    );
    assert_eq!(
        snapshot.candidates[1].source_parent_id.as_deref(),
        Some(snapshot.candidates[0].candidate_id.as_str())
    );
    assert_eq!(snapshot.candidates[2].source_title, "2. Appendix");
    assert_eq!(snapshot.candidates[2].physical_page_index, 3);
    assert!(snapshot.candidates[2]
        .reason_codes
        .iter()
        .any(|reason| reason == "toc_exact_page_label"));
    let mut reviews = BookmarkReviews::empty(snapshot.generation_digest.clone());
    for candidate in &snapshot.candidates {
        bookmarks::append(
            &snapshot,
            &mut reviews,
            candidate.candidate_id.clone(),
            ReviewAction::Confirm,
        )?;
    }
    let effective = bookmarks::effective(&snapshot, &reviews)?;
    let built = searchable_pdf::build(&fs::read(&source)?, &package, &effective, Some(&derived))?;
    let output = dir.path().join("searchable.pdf");
    fs::write(&output, built)?;
    let reopened = PdfDocumentSession::open(
        &output,
        &PdfOpenOptions {
            pdfium: config,
            ..Default::default()
        },
    )?;
    assert_eq!(reopened.info().page_count, session.info().page_count);
    for (index, (before, after)) in session
        .info()
        .pages
        .iter()
        .zip(reopened.info().pages.iter())
        .enumerate()
    {
        assert_eq!(before.geometry.width_points, after.geometry.width_points);
        assert_eq!(before.geometry.height_points, after.geometry.height_points);
        assert_eq!(before.source_rotation, after.source_rotation);
        assert_eq!(
            before_renders[index],
            reopened.render_page(index as u32, 72)?.to_rgba8(),
            "invisible text must not change visible pixels or image polarity"
        );
    }
    assert!(reopened.native_text(0)?.text.contains("Ἀρχὴ"));
    assert!(reopened.native_text(1)?.text.contains("Πολιτείας"));
    let outline = reopened.native_outline()?;
    assert_eq!(outline.len(), 3);
    assert_eq!(outline[0].title, "1. Ἀρχὴ");
    assert_eq!(outline[0].level, 0);
    assert_eq!(outline[0].page_index, 0);
    assert!(outline[0].x.is_some() && outline[0].y.is_some());
    assert_eq!(outline[1].title, "1.1 Πολιτείας");
    assert_eq!(outline[1].level, 1);
    assert_eq!(outline[1].page_index, 1);
    assert_eq!(outline[2].title, "2. Appendix");
    assert_eq!(outline[2].page_index, 3);

    let source_pdf = lopdf::Document::load(&source)?;
    let output_pdf = lopdf::Document::load(&output)?;
    for ((_, source_id), (_, output_id)) in source_pdf
        .get_pages()
        .iter()
        .zip(output_pdf.get_pages().iter())
    {
        let source_page = source_pdf.get_dictionary(*source_id)?;
        let output_page = output_pdf.get_dictionary(*output_id)?;
        assert_eq!(source_page.get(b"MediaBox")?, output_page.get(b"MediaBox")?);
        assert_eq!(
            source_page.get(b"Rotate").ok(),
            output_page.get(b"Rotate").ok()
        );
    }
    Ok(())
}
