//! Opt-in live validation against the real Gemini API.
//!
//! Every test here is `#[ignore]`d and additionally refuses to run unless
//! `MPDF_GEMINI_LIVE=1` is set. Two independent gates, because a live run
//! **costs the operator money** and an accidental `--include-ignored` in CI
//! would bill them for it.
//!
//! ```text
//! MPDF_GEMINI_LIVE=1 MPDF_GEMINI_LIVE_SLOT=default \
//!   cargo test -p mpdf-api-client --test gemini_live -- --ignored --nocapture
//! ```
//!
//! The key is read from the OS credential store by slot name. It is never an
//! argument, never an environment variable, and nothing in this file formats,
//! prints, asserts on, or returns it. The only credential fact these tests can
//! observe is "a key is present in that slot".
//!
//! What this suite is *not*: it is not the accuracy evaluation. Character and
//! word error rates against the 63-page CGPG Greek holdout and the 8-page
//! mixed Greek/German/Latin holdout are measured by `scripts/ocr/gold`, whose
//! corpora are provisioned out of band and never committed. This suite answers
//! a narrower question — does the transport work, end to end, against the real
//! service, without leaking anything.

use std::sync::Arc;
use std::time::Duration;

use mpdf_api_client::cloud_ocr::{
    CloudTransportPolicy, GeminiByokFactory, ModelProviderSecretStore, GEMINI_ENDPOINT,
};
use mpdf_core::ocr_provider::gemini::{
    CloudOcrConfig, PageTranscriptionTransport, PromptConfig, TranscriptionRequest,
};
use mpdf_core::ocr_provider::OutputContract;

fn live_enabled() -> Option<String> {
    if std::env::var("MPDF_GEMINI_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("MPDF_GEMINI_LIVE_SLOT").unwrap_or_else(|_| "default".to_owned()))
}

/// A page image the repository owns: rendered here, from vendored OFL fonts,
/// so no third-party or copyrighted material is ever uploaded by a test.
fn fixture_page_png() -> Vec<u8> {
    use image::{ImageFormat, RgbImage};
    // Deliberately a blank page. The point of the live suite is the
    // transport, not the recognizer; sending real text would make the
    // assertions depend on model behaviour that changes without notice.
    let image = RgbImage::from_pixel(600, 800, image::Rgb([255, 255, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut out, ImageFormat::Png)
        .expect("encode fixture page");
    out.into_inner()
}

#[test]
#[ignore = "calls the real Gemini API and costs money; set MPDF_GEMINI_LIVE=1"]
fn a_metadata_probe_reaches_the_model_without_transcribing_anything() {
    let Some(slot) = live_enabled() else {
        eprintln!("skipped: MPDF_GEMINI_LIVE is not 1");
        return;
    };
    let factory = GeminiByokFactory::new(
        CloudOcrConfig::gemini_byok(&slot),
        GEMINI_ENDPOINT,
        CloudTransportPolicy::default(),
        "live-validation",
    );
    let result = factory.connection_test();
    // Reported without the key, and with a diagnostic the transport already
    // redacted and bounded.
    eprintln!(
        "provider={} model={} available={} slot={} present={} diagnostic={}",
        result.provider_name,
        result.model,
        result.model_available,
        result.credential.slot,
        result.credential.present,
        result.diagnostic
    );
    assert!(
        result.credential.present,
        "store a key in slot {slot} first"
    );
    assert_eq!(result.credential.masked, "****");
    assert!(result.model_available, "{}", result.diagnostic);
}

#[test]
#[ignore = "calls the real Gemini API and costs money; set MPDF_GEMINI_LIVE=1"]
fn one_page_round_trips_and_reports_usage_without_leaking_the_key() {
    let Some(slot) = live_enabled() else {
        eprintln!("skipped: MPDF_GEMINI_LIVE is not 1");
        return;
    };
    let secrets = Arc::new(ModelProviderSecretStore);
    assert!(
        secrets.describe(&slot).present,
        "store a key in slot {slot} first"
    );
    let mut transport = mpdf_api_client::cloud_ocr::GeminiTransport::new(
        GEMINI_ENDPOINT,
        &slot,
        secrets,
        CloudTransportPolicy::default(),
    )
    .expect("transport");

    let png = fixture_page_png();
    let prompt = PromptConfig::default();
    let response = transport
        .transcribe(&TranscriptionRequest {
            model: "gemini-3.7-flash",
            model_version: "2026-08",
            prompt: &prompt,
            output_contract: OutputContract::TranscriptionOnly,
            page_png: &png,
            page_width: 600,
            page_height: 800,
            page_image_sha256: &"0".repeat(64),
            page_index: 0,
            language_profile: "auto",
            deadline: Duration::from_secs(120),
            idempotency_key: "live-validation-page-0",
        })
        .expect("the live call succeeds");
    eprintln!(
        "model_version={} input_tokens={} output_tokens={} body_bytes={}",
        response.model_version,
        response.input_tokens,
        response.output_tokens,
        response.body.len()
    );
    assert!(response.input_tokens > 0, "usage must be reported");
    // The transcription of a blank page is not asserted on — that is the
    // model's business. What is asserted is that nothing credential-shaped
    // came back into our hands.
    assert!(!response.body.contains("AIza"));
}
