//! Three provider modes, one canonical block tree, one bookmark pipeline.
//!
//! These tests exist to hold the central architectural claim of the cloud OCR
//! work: a cloud provider is not an attachment bolted onto the side of the
//! pipeline, it is another producer of `OcrPage` records, and everything
//! downstream — logical lines, the derived bundle, printed-page mapping,
//! body-heading verification, bookmark scoring — consumes them without
//! knowing or caring which mode produced them.
//!
//! Nothing here touches the network. The cloud path is driven through
//! `ScriptedTransport`, which answers from a fixed table, so a test run can
//! never call a real API or spend anything.

use std::collections::BTreeMap;

use image::{DynamicImage, RgbImage};
use mpdf_core::bookmark_fixtures::{self as fixtures, FixtureLine, FixturePage};
use mpdf_core::bookmarks::{
    self, AutoBookmarkConfig, AutoBookmarkInput, BookmarkStatus, GenerationMode,
};
use mpdf_core::derived::DerivedDocument;
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::ocr::{
    OcrError, OcrPage, OcrProviderProvenance, OcrRun, PageOcrProvider, OCR_PROTOCOL,
    OCR_PROTOCOL_VERSION,
};
use mpdf_core::ocr_provider::gemini::{
    CloudOcrConfig, CloudOcrProvider, ScriptedTransport, TranscriptionResponse, TransportError,
};
use mpdf_core::ocr_provider::runner::ProviderPageAdapter;
use mpdf_core::ocr_provider::{
    CloudFallback, GeometrySource, OcrProvider, OcrProviderMode, OutputContract, PageOcrRequest,
};

/// The canary. If this string appears in any artifact a test writes, reads or
/// serializes, the run has leaked a credential.
const CANARY: &str = "AIzaSyCANARY-do-not-leak-0123456789ab";

/// Simulates local OCR that read the page correctly enough to measure it and
/// badly enough to need help: every `e` came back as `c`, and every `l` as
/// `1`. Geometry is untouched, which is the whole point.
fn degrade(page: &FixturePage) -> FixturePage {
    let mut degraded = page.clone();
    degraded.lines = page
        .lines
        .iter()
        .map(|line| {
            let mut copy = FixtureLine::new(
                &line.text.replace('e', "c").replace('l', "1"),
                line.x,
                line.y,
            );
            copy.width = line.width;
            copy.height = line.height;
            copy.confidence = line.confidence;
            copy
        })
        .collect();
    degraded
}

/// Returns the fixture page exactly as the local detector would have: correct
/// rectangles, degraded characters.
struct FixtureLocalProvider {
    pages: Vec<FixturePage>,
}

impl PageOcrProvider for FixtureLocalProvider {
    fn recognize(
        &mut self,
        page_index: u32,
        _image: &DynamicImage,
        digest: &str,
    ) -> Result<OcrPage, OcrError> {
        let page = self
            .pages
            .get(page_index as usize)
            .cloned()
            .unwrap_or_default();
        let run = fixtures::ocr_run(
            &[page],
            Some(OcrProviderProvenance {
                engine: "tesseract".into(),
                model: "tessdata_best".into(),
                version: "5.3".into(),
                parameters: BTreeMap::new(),
                input_asset_sha256: digest.to_owned(),
                execution_location: "local".into(),
                language_profile: Some("auto".into()),
                model_digest: None,
                model_license: None,
                model_set: Some("tessdata_best 4.1.0".into()),
            }),
        );
        let mut page = run.pages.into_iter().next().expect("one page");
        page.page_index = page_index;
        Ok(page)
    }
}

fn transcription_of(page: &FixturePage) -> String {
    page.lines
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs every page of a fixture book through a cloud provider and returns the
/// resulting canonical run, plus the pages that fell back.
fn cloud_run(
    truth: &[FixturePage],
    config: CloudOcrConfig,
    failures: &[u32],
) -> (OcrRun, Vec<u32>) {
    let mut transport = ScriptedTransport {
        credential: true,
        ..Default::default()
    };
    for (index, page) in truth.iter().enumerate() {
        let index = index as u32;
        let answer = if failures.contains(&index) {
            Err(TransportError::failed(format!(
                "provider rejected the request; key={CANARY} was echoed in the body"
            )))
        } else {
            Ok(TranscriptionResponse {
                body: transcription_of(page),
                model_version: "2026-08".into(),
                input_tokens: 1_500,
                output_tokens: 400,
            })
        };
        transport.answers.insert(index, answer);
    }
    let degraded: Vec<FixturePage> = truth.iter().map(degrade).collect();
    let mut provider = CloudOcrProvider::new(
        config,
        Box::new(transport),
        Box::new(FixtureLocalProvider { pages: degraded }),
        "tesseract-5/tessdata_best-4.1.0",
    );

    let image = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
    let mut pages = Vec::new();
    let mut fell_back = Vec::new();
    for index in 0..truth.len() as u32 {
        let outcome = provider
            .recognize_page(&PageOcrRequest {
                document_sha256: &"d".repeat(64),
                page_index: index,
                page_image_sha256: &"ab".repeat(32),
                page_image: &image,
                language_profile: "auto",
                dpi: 300,
                local_layout_version: "tesseract-5",
                requested_output_contract: OutputContract::TranscriptionOnly,
                deadline: std::time::Duration::from_secs(30),
                idempotency_key: &format!("page-{index}"),
            })
            .expect("a cloud page never fails the run under the default fallback policy");
        if outcome.fallback_reason.is_some() {
            fell_back.push(index);
        }
        pages.push(outcome.page);
    }
    (
        OcrRun {
            protocol: OCR_PROTOCOL.into(),
            protocol_version: OCR_PROTOCOL_VERSION.into(),
            pages,
            errors: Vec::new(),
        },
        fell_back,
    )
}

fn bookmarks_of(package: &DocumentPackage, run: &OcrRun) -> bookmarks::AutoBookmarkResult {
    let derived = DerivedDocument::from_package(package, Some(run)).expect("derived document");
    bookmarks::generate_auto(
        &AutoBookmarkInput {
            package,
            ocr: Some(run),
            derived: Some(&derived),
        },
        &AutoBookmarkConfig::default(),
    )
    .expect("automatic generation")
}

fn confirmed_titles(result: &bookmarks::AutoBookmarkResult) -> Vec<String> {
    result
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.status == BookmarkStatus::AutoConfirmed)
        .map(|candidate| candidate.effective_title.clone())
        .collect()
}

#[test]
fn cloud_material_reaches_the_same_bookmark_pipeline_and_recovers_the_titles() {
    let (package, truth) = fixtures::aligned_book();

    // The local-only baseline: the degraded characters are what the bookmark
    // engine has to work with.
    let degraded: Vec<FixturePage> = truth.iter().map(degrade).collect();
    let local_only = fixtures::ocr_run(&degraded, None);
    let local_titles = confirmed_titles(&bookmarks_of(&package, &local_only));
    assert!(
        !local_titles.contains(&"Introduction".to_owned()),
        "the degraded fixture must not already read correctly, or this test proves nothing: \
         {local_titles:?}"
    );

    // The same pages, same rectangles, with the transcription aligned onto
    // them. Nothing about the bookmark engine changed.
    let (run, fell_back) = cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[]);
    assert!(fell_back.is_empty());
    let result = bookmarks_of(&package, &run);
    assert_eq!(result.report.mode, GenerationMode::TocAligned);
    assert_eq!(
        confirmed_titles(&result),
        vec!["Introduction", "The Wine Dark Sea", "Homeric Formulae"]
    );
}

#[test]
fn printed_page_numbers_survive_the_cloud_path_and_map_to_the_right_targets() {
    // The contents rows carry "title ......... 12". Losing the number, or
    // attaching it to the wrong row, is the failure that made every entry
    // unmappable in the earlier region-based adapter — so the mapping itself
    // is asserted, not just the titles.
    let (package, truth) = fixtures::aligned_book();
    let (run, _) = cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[]);
    let result = bookmarks_of(&package, &run);
    let targets: Vec<(String, u32)> = result
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.status == BookmarkStatus::AutoConfirmed)
        .map(|candidate| {
            (
                candidate.effective_title.clone(),
                candidate.physical_page_index,
            )
        })
        .collect();
    // The fixture prints page 1 on physical index 4, so the mapping is a
    // constant +3 offset that the engine has to discover rather than assume.
    assert_eq!(
        targets,
        vec![
            ("Introduction".to_owned(), 4),
            ("The Wine Dark Sea".to_owned(), 15),
            ("Homeric Formulae".to_owned(), 26),
        ]
    );
}

#[test]
fn a_page_the_provider_failed_keeps_its_local_text_and_the_run_still_produces_bookmarks() {
    let (package, truth) = fixtures::aligned_book();
    // Page 1 is the contents page: the worst one to lose, and the one whose
    // silent loss would produce an outline-free document with no explanation.
    let (run, fell_back) = cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[2, 5]);
    assert_eq!(fell_back, vec![2, 5]);
    // Every page still carries words: a provider failure never yields a page
    // with an empty text layer.
    for page in &run.pages {
        let words: usize = page
            .blocks
            .iter()
            .flat_map(|block| block.lines.iter())
            .map(|line| line.words.len())
            .sum();
        if !truth[page.page_index as usize].lines.is_empty() {
            assert!(words > 0, "page {} lost its text layer", page.page_index);
        }
    }
    let result = bookmarks_of(&package, &run);
    assert_eq!(result.report.mode, GenerationMode::TocAligned);
    assert!(!confirmed_titles(&result).is_empty());

    // The evidence says, per page, which provider actually produced it.
    let fallback_pages: Vec<u32> = run
        .pages
        .iter()
        .filter(|page| {
            page.provider_provenance
                .as_ref()
                .and_then(|provenance| provenance.parameters.get("geometry_source"))
                .is_some_and(|source| source == GeometrySource::LocalLayout.as_str())
        })
        .map(|page| page.page_index)
        .collect();
    assert_eq!(fallback_pages, vec![2, 5]);
}

#[test]
fn every_mode_produces_a_run_the_evidence_validator_accepts() {
    let (_, truth) = fixtures::aligned_book();
    for config in [
        CloudOcrConfig::gemini_byok("slot"),
        CloudOcrConfig::mpdf_credits(2),
    ] {
        let mode = config.mode;
        let (run, _) = cloud_run(&truth, config, &[]);
        run.validate()
            .unwrap_or_else(|error| panic!("{} produced invalid evidence: {error}", mode.id()));
        for page in &run.pages {
            let provenance = page
                .provider_provenance
                .as_ref()
                .expect("every cloud page records its provenance");
            assert_eq!(provenance.parameters["provider_mode"], mode.id());
            assert_eq!(provenance.execution_location, mode.execution_location());
        }
    }
    // The local route's own evidence is unchanged and still valid.
    fixtures::ocr_run(&truth, None)
        .validate()
        .expect("local evidence stays valid");
}

#[test]
fn byok_and_brokered_pages_are_distinguishable_in_the_evidence() {
    let (_, truth) = fixtures::aligned_book();
    let (byok, _) = cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[]);
    let (credits, _) = cloud_run(&truth, CloudOcrConfig::mpdf_credits(2), &[]);
    let byok_location = &byok.pages[0]
        .provider_provenance
        .as_ref()
        .unwrap()
        .execution_location;
    let credits_location = &credits.pages[0]
        .provider_provenance
        .as_ref()
        .unwrap()
        .execution_location;
    assert_ne!(byok_location, credits_location);
}

#[test]
fn changing_the_provider_or_the_alignment_invalidates_the_bookmark_evidence() {
    // The rule: bookmark candidates are bound to the OCR evidence they were
    // compiled from. If the same page is later recognized by a different
    // provider, under a different prompt, or aligned by different rules, the
    // old candidates describe evidence that no longer exists — so the digest
    // the snapshot carries has to move.
    let (package, truth) = fixtures::aligned_book();
    let byok = bookmarks_of(
        &package,
        &cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[]).0,
    );
    let credits = bookmarks_of(
        &package,
        &cloud_run(&truth, CloudOcrConfig::mpdf_credits(2), &[]).0,
    );
    let local = bookmarks_of(&package, &fixtures::ocr_run(&truth, None));

    let digests = [
        byok.report.ocr_digest.clone(),
        credits.report.ocr_digest.clone(),
        local.report.ocr_digest.clone(),
    ];
    let unique: std::collections::BTreeSet<_> = digests.iter().collect();
    assert_eq!(unique.len(), 3, "{digests:?}");

    // A different prompt is a different transcription contract, so it is also
    // different evidence even under the same mode and model.
    let mut other_prompt = CloudOcrConfig::gemini_byok("slot");
    other_prompt
        .prompt
        .instruction
        .push_str(" Also transcribe marginalia.");
    let reprompted = bookmarks_of(&package, &cloud_run(&truth, other_prompt, &[]).0);
    assert_ne!(reprompted.report.ocr_digest, byok.report.ocr_digest);

    // And a page that fell back is not the same evidence as one that did not.
    let with_fallback = bookmarks_of(
        &package,
        &cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[4]).0,
    );
    assert_ne!(with_fallback.report.ocr_digest, byok.report.ocr_digest);
}

#[test]
fn a_credential_canary_never_reaches_evidence_reports_or_the_workspace() {
    // Every failing page's transport error body contains the canary, exactly
    // as a real provider's 403 body sometimes echoes the key back. It must
    // survive nowhere.
    let (package, truth) = fixtures::aligned_book();
    let (run, fell_back) = cloud_run(&truth, CloudOcrConfig::gemini_byok("slot"), &[0, 1, 4]);
    assert_eq!(fell_back, vec![0, 1, 4]);

    let workspace = tempfile::tempdir().expect("workspace");
    mpdf_core::ocr::write_ocr_records(workspace.path(), &run).expect("write evidence");

    let derived = DerivedDocument::from_package(&package, Some(&run)).expect("derived");
    let result = bookmarks_of(&package, &run);

    let mut haystacks: Vec<(String, String)> = vec![
        (
            "ocr run json".into(),
            serde_json::to_string(&run).expect("run json"),
        ),
        (
            "derived json".into(),
            serde_json::to_string(&derived).expect("derived json"),
        ),
        (
            "bookmark report json".into(),
            serde_json::to_string(&result.report).expect("report json"),
        ),
        (
            "bookmark snapshot json".into(),
            serde_json::to_string(&result.snapshot).expect("snapshot json"),
        ),
        ("run debug".into(), format!("{run:?}")),
    ];
    // Everything the run wrote to disk, including the per-page records and
    // the summary a resumed run reads back.
    for entry in walk(workspace.path()) {
        let bytes = std::fs::read(&entry).expect("read artifact");
        haystacks.push((
            entry.display().to_string(),
            String::from_utf8_lossy(&bytes).into_owned(),
        ));
    }
    for (what, haystack) in haystacks {
        assert!(
            !haystack.contains(CANARY),
            "the credential canary leaked into {what}"
        );
        assert!(
            !haystack.contains("AIzaSy"),
            "a Google-shaped key leaked into {what}"
        );
    }

    // And the fallback is still *reported* — redaction must not silence the
    // fact that three pages fell back.
    let reasons: Vec<&String> = run
        .pages
        .iter()
        .filter_map(|page| page.provider_provenance.as_ref())
        .filter_map(|provenance| provenance.parameters.get("fallback_reason"))
        .collect();
    assert_eq!(reasons.len(), 3);
    assert!(reasons.iter().all(|reason| *reason == "transport_failed"));
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn the_local_mode_needs_no_credential_and_declares_no_network_use() {
    assert!(!OcrProviderMode::Local.uses_network());
    assert!(OcrProviderMode::Local.is_default());
    // A local fixture run produces evidence with no provider parameters at
    // all — no mode, no prompt digest, nothing that implies a cloud call.
    let (_, truth) = fixtures::aligned_book();
    let run = fixtures::ocr_run(&truth, None);
    for page in &run.pages {
        assert!(page.provider_provenance.is_none());
    }
    run.validate().expect("local evidence is valid");
}

#[test]
fn the_fail_fallback_policy_stops_rather_than_shipping_mixed_provenance() {
    let (_, truth) = fixtures::aligned_book();
    let mut transport = ScriptedTransport {
        credential: true,
        ..Default::default()
    };
    transport
        .answers
        .insert(0, Err(TransportError::failed("503 upstream")));
    let mut provider = CloudOcrProvider::new(
        CloudOcrConfig {
            fallback: CloudFallback::Fail,
            ..CloudOcrConfig::gemini_byok("slot")
        },
        Box::new(transport),
        Box::new(FixtureLocalProvider {
            pages: truth.iter().map(degrade).collect(),
        }),
        "tesseract-5",
    );
    let image = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
    let error = provider
        .recognize_page(&PageOcrRequest {
            document_sha256: &"d".repeat(64),
            page_index: 0,
            page_image_sha256: &"ab".repeat(32),
            page_image: &image,
            language_profile: "auto",
            dpi: 300,
            local_layout_version: "tesseract-5",
            requested_output_contract: OutputContract::TranscriptionOnly,
            deadline: std::time::Duration::from_secs(5),
            idempotency_key: "page-0",
        })
        .expect_err("fail policy must stop the page");
    assert!(error.to_string().contains("cloud-fallback"));
}

#[test]
fn the_adapter_ledger_matches_the_evidence_it_produced() {
    let (_, truth) = fixtures::aligned_book();
    let mut transport = ScriptedTransport {
        credential: true,
        ..Default::default()
    };
    for (index, page) in truth.iter().enumerate() {
        transport.answers.insert(
            index as u32,
            Ok(TranscriptionResponse {
                body: transcription_of(page),
                model_version: "2026-08".into(),
                input_tokens: 100,
                output_tokens: 20,
            }),
        );
    }
    transport
        .answers
        .insert(3, Err(TransportError::CredentialRejected));
    let mut provider = CloudOcrProvider::new(
        CloudOcrConfig::gemini_byok("slot"),
        Box::new(transport),
        Box::new(FixtureLocalProvider {
            pages: truth.iter().map(degrade).collect(),
        }),
        "tesseract-5",
    );
    let mut adapter = ProviderPageAdapter::new(
        &mut provider,
        "d".repeat(64),
        "auto",
        300,
        "tesseract-5",
        "cfg",
    );
    let image = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
    for index in 0..truth.len() as u32 {
        adapter
            .recognize(index, &image, &"ab".repeat(32))
            .expect("page");
    }
    assert_eq!(adapter.fallback_pages(), vec![3]);
    assert_eq!(
        adapter.usage().requests as usize,
        truth.len() - 1,
        "a failed page must not be counted as a billed request"
    );
}
