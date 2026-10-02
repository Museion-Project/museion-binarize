//! Bridging the provider contract onto the existing durable OCR loop.
//!
//! [`crate::ocr::run_session_durable`] already does the hard part: claim a
//! page, run a provider, validate the typed record, checkpoint it, and never
//! re-run a page whose committed file still matches its digest. That loop is
//! provider-shaped, not local-shaped, so the three modes plug into it rather
//! than getting their own copy of it. A second orchestration loop is how
//! "resume reuses a page recognized under a different model" gets introduced.
//!
//! The adapter is also where the per-page ledger is kept: which mode actually
//! produced each page, which pages fell back, and what they cost. The
//! orchestrator reads that back for the run report, so a user can always see
//! that pages 3, 17 and 41 of a cloud run are local text.

use std::time::Duration;

use image::DynamicImage;

use crate::jobs::ExecutionLocation;
use crate::ocr::{OcrError, OcrPage, PageOcrProvider};

use super::credits::page_idempotency_key;
use super::{
    GeometrySource, OcrProvider, OcrProviderMode, OutputContract, PageOcrRequest, ProviderUsage,
};

/// What one page cost and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLedgerEntry {
    pub page_index: u32,
    pub mode: OcrProviderMode,
    pub geometry_source: GeometrySource,
    pub fallback_code: Option<&'static str>,
    pub fallback_detail: Option<String>,
    pub usage: ProviderUsage,
}

/// Adapts an [`OcrProvider`] to the page-at-a-time interface the durable loop
/// drives, and records what happened on the way through.
pub struct ProviderPageAdapter<'a> {
    provider: &'a mut dyn OcrProvider,
    document_sha256: String,
    language_profile: String,
    dpi: u16,
    local_layout_version: String,
    provider_config_digest: String,
    output_contract: OutputContract,
    execution_location: ExecutionLocation,
    deadline: Duration,
    ledger: Vec<PageLedgerEntry>,
    usage: ProviderUsage,
}

impl<'a> ProviderPageAdapter<'a> {
    pub fn new(
        provider: &'a mut dyn OcrProvider,
        document_sha256: impl Into<String>,
        language_profile: impl Into<String>,
        dpi: u16,
        local_layout_version: impl Into<String>,
        provider_config_digest: impl Into<String>,
    ) -> Self {
        let execution_location = provider.capabilities().mode.execution_location_kind();
        Self {
            provider,
            document_sha256: document_sha256.into(),
            language_profile: language_profile.into(),
            dpi,
            local_layout_version: local_layout_version.into(),
            provider_config_digest: provider_config_digest.into(),
            output_contract: OutputContract::TranscriptionOnly,
            execution_location,
            deadline: Duration::from_secs(180),
            ledger: Vec::new(),
            usage: ProviderUsage::default(),
        }
    }

    pub fn with_output_contract(mut self, contract: OutputContract) -> Self {
        self.output_contract = contract;
        self
    }

    pub fn ledger(&self) -> &[PageLedgerEntry] {
        &self.ledger
    }

    pub fn usage(&self) -> ProviderUsage {
        self.usage
    }

    /// Pages that did not use the provider they were run under.
    pub fn fallback_pages(&self) -> Vec<u32> {
        self.ledger
            .iter()
            .filter(|entry| entry.fallback_code.is_some())
            .map(|entry| entry.page_index)
            .collect()
    }
}

impl PageOcrProvider for ProviderPageAdapter<'_> {
    fn requires_raster_recognition(&self) -> bool {
        self.provider.requires_raster_recognition()
    }

    fn execution_location(&self) -> ExecutionLocation {
        self.execution_location
    }

    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> Result<OcrPage, OcrError> {
        // Derived from the work, never from the attempt. The broker uses this
        // as its billing idempotency key; BYOK keeps it only as local audit
        // identity because the upstream API offers no exactly-once contract.
        let idempotency_key = page_idempotency_key(
            &self.document_sha256,
            page_index,
            input_asset_sha256,
            &self.provider_config_digest,
        );
        let outcome = self.provider.recognize_page(&PageOcrRequest {
            document_sha256: &self.document_sha256,
            page_index,
            page_image_sha256: input_asset_sha256,
            page_image: image,
            language_profile: &self.language_profile,
            dpi: self.dpi,
            local_layout_version: &self.local_layout_version,
            requested_output_contract: self.output_contract,
            deadline: self.deadline,
            idempotency_key: &idempotency_key,
        })?;
        self.usage.add(outcome.usage);
        self.ledger.push(PageLedgerEntry {
            page_index,
            mode: outcome.provider_mode,
            geometry_source: outcome.geometry_source,
            fallback_code: outcome.fallback_reason.as_ref().map(|reason| reason.code()),
            fallback_detail: outcome
                .fallback_reason
                .as_ref()
                .map(|reason| reason.describe()),
            usage: outcome.usage,
        });
        Ok(outcome.page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrBlock, OcrBox, OcrLine, OcrRoute, OcrRouteReason, OcrWord};
    use crate::ocr_provider::gemini::{
        CloudOcrConfig, CloudOcrProvider, ScriptedTransport, TransportError,
    };
    use image::RgbImage;

    struct FixtureLocal;

    impl PageOcrProvider for FixtureLocal {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::Local
        }

        fn recognize(
            &mut self,
            page_index: u32,
            _image: &DynamicImage,
            _digest: &str,
        ) -> Result<OcrPage, OcrError> {
            let bbox = OcrBox {
                x: 5.0,
                y: 5.0,
                width: 100.0,
                height: 20.0,
            };
            Ok(OcrPage {
                page_index,
                route: OcrRoute::Ocr {
                    reason: OcrRouteReason::MissingText,
                },
                width: 200,
                height: 200,
                blocks: vec![OcrBlock {
                    bbox: bbox.clone(),
                    confidence: 0.8,
                    reading_order: 0,
                    lines: vec![OcrLine {
                        bbox: bbox.clone(),
                        confidence: 0.8,
                        reading_order: 0,
                        words: vec![OcrWord {
                            text: "Einleitng".into(),
                            normalized_text: "Einleitng".into(),
                            bbox,
                            confidence: 0.8,
                            reading_order: 0,
                        }],
                    }],
                }],
                revisions: Vec::new(),
                provider_provenance: None,
                provider_raw_artifact: None,
            })
        }
    }

    #[test]
    fn the_ledger_names_every_page_that_fell_back_and_why() {
        let transport = ScriptedTransport::with_text("Einleitung\n")
            .answer(1, Err(TransportError::failed("504 gateway timeout")));
        let mut cloud = CloudOcrProvider::new(
            CloudOcrConfig::gemini_byok("slot"),
            Box::new(transport),
            Box::new(FixtureLocal),
            "tesseract-5",
        );
        let mut adapter = ProviderPageAdapter::new(
            &mut cloud,
            "d".repeat(64),
            "auto",
            300,
            "tesseract-5",
            "cfg",
        );
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 200));
        for page in 0..3 {
            adapter.recognize(page, &image, &"i".repeat(64)).unwrap();
        }
        assert_eq!(adapter.fallback_pages(), vec![1]);
        assert_eq!(
            adapter.ledger()[0].geometry_source,
            GeometrySource::LocalLayoutAlignedText
        );
        assert_eq!(
            adapter.ledger()[1].geometry_source,
            GeometrySource::LocalLayout
        );
        assert_eq!(adapter.ledger()[1].fallback_code, Some("transport_failed"));
        assert_eq!(adapter.usage().requests, 2);
    }

    #[test]
    fn the_same_page_always_gets_the_same_idempotency_key() {
        // Two adapters over the same work must derive identical keys, or a
        // resumed run would look like new work to the billing side.
        let first = page_idempotency_key(&"d".repeat(64), 7, &"i".repeat(64), "cfg");
        let second = page_idempotency_key(&"d".repeat(64), 7, &"i".repeat(64), "cfg");
        assert_eq!(first, second);
    }
}
