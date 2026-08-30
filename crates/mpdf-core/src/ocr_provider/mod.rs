//! The provider-neutral OCR contract.
//!
//! # The one rule this module exists to enforce
//!
//! Every OCR provider — local Tesseract, a cloud model driven with the user's
//! own key, or the same model executed on M PDF's behalf — must hand back the
//! *same* canonical structure: [`crate::ocr::OcrPage`], a block/line/word tree
//! with measured boxes in the page's own pixel coordinate system.
//!
//! That is not a stylistic preference. Everything downstream — logical line
//! assembly, the derived bundle, printed-page mapping, body-heading
//! verification, bookmark scoring, and the invisible text layer written into
//! the final PDF — consumes `OcrPage` and nothing else. A cloud provider that
//! returned "the text of the page" as a string would be an attachment, not
//! OCR: it could not be aligned to a contents row, it could not carry a page
//! number back to a target page, and it could not be drawn at the right place
//! in the searchable layer. So a cloud provider does not get to be a special
//! case; it gets to be an implementation of this trait.
//!
//! # What is deliberately *not* here
//!
//! No HTTP, no credential storage, no key material. `mpdf-core` stays usable
//! with the network stack removed: the cloud providers in this module are
//! written against small transport traits, and the real HTTPS implementations
//! live in `mpdf-api-client`. That also makes the interesting behaviour —
//! prompts, alignment, geometry gates, fallback, provenance — testable with a
//! deterministic fake and no API calls.

pub mod alignment;
pub mod credits;
pub mod gemini;
pub mod redaction;
pub mod runner;
pub mod structured_bbox;

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use image::DynamicImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ocr::{OcrError, OcrPage};

/// Version of the provider contract itself. Bound into the checkpoint
/// fingerprint of any cloud run, so changing the contract cannot silently
/// reuse evidence produced under the previous one.
pub const PROVIDER_CONTRACT_VERSION: &str = "mpdf-ocr-provider/1";

/// Which of the three supported execution modes produced a page.
///
/// `Local` is the default and the only mode that never touches the network.
/// Nothing in this crate may select a non-local mode implicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OcrProviderMode {
    #[default]
    Local,
    GeminiByok,
    MpdfCredits,
}

impl OcrProviderMode {
    pub const ALL: [OcrProviderMode; 3] = [Self::Local, Self::GeminiByok, Self::MpdfCredits];

    pub fn id(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::GeminiByok => "gemini-byok",
            Self::MpdfCredits => "mpdf-credits",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Local => "Local OCR",
            Self::GeminiByok => "Gemini API — Use My Key",
            Self::MpdfCredits => "M PDF Cloud OCR",
        }
    }

    pub fn availability(self) -> &'static str {
        match self {
            Self::Local => "stable",
            Self::GeminiByok => "beta",
            Self::MpdfCredits => "unavailable",
        }
    }

    pub fn release_blockers(self) -> Vec<&'static str> {
        match self {
            Self::Local => Vec::new(),
            Self::GeminiByok => vec![
                "live Gemini success, rejection, rate-limit, timeout and cancellation validation is pending",
                "provider terms, privacy disclosure and retention review are pending",
                "cross-platform credential-store validation is pending",
            ],
            Self::MpdfCredits => credits::release_blockers(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == value)
    }

    /// Whether selecting this mode causes document page images to leave the
    /// machine. Used by the consent copy in both front ends, so the answer
    /// lives here rather than being retyped in a label.
    pub fn uses_network(self) -> bool {
        !matches!(self, Self::Local)
    }

    pub fn is_default(self) -> bool {
        matches!(self, Self::Local)
    }

    /// Where the model actually ran. `gemini-byok` and `mpdf-credits` can run
    /// the *same* model, so the provenance has to distinguish who executed
    /// it, not just what executed.
    pub fn execution_location(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::GeminiByok => "remote:user-key",
            Self::MpdfCredits => "remote:mpdf-brokered",
        }
    }
}

impl fmt::Display for OcrProviderMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// How the coordinates on a page were obtained.
///
/// This is the field a reviewer reads to decide whether to trust a box. A
/// language model's self-reported rectangle and a rectangle measured by the
/// local detector are not the same kind of claim, so they are not the same
/// value here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometrySource {
    /// Boxes came from PDFium's extracted text runs (no OCR was needed).
    NativeText,
    /// Boxes were measured by the local detector on the original raster.
    LocalLayout,
    /// Boxes were measured locally; the *text* came from a cloud
    /// transcription that aligned to those lines.
    LocalLayoutAlignedText,
    /// Boxes came from a structured provider response and passed every
    /// cross-validation gate. Experimental; never a default.
    ProviderStructuredValidated,
}

impl GeometrySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeText => "native_text",
            Self::LocalLayout => "local_layout",
            Self::LocalLayoutAlignedText => "local_layout_aligned_text",
            Self::ProviderStructuredValidated => "provider_structured_validated",
        }
    }

    /// Whether a coordinate from this source may be written into the final
    /// PDF's text layer. Every source here may, which is the point: an
    /// unvalidated model coordinate never becomes a `GeometrySource` at all,
    /// it becomes a [`FallbackReason`].
    pub fn is_writable(self) -> bool {
        true
    }
}

/// Why a page (or a line) did not use the provider result it asked for.
///
/// Fallback is a first-class, per-page, reportable outcome. A cloud run that
/// silently degraded to local text on 40% of its pages, and reported success,
/// would be the worst possible failure mode: the user paid, the evidence is
/// local, and nothing says so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum FallbackReason {
    /// No credential was configured for the selected mode.
    CredentialMissing,
    /// The provider refused the credential.
    CredentialRejected,
    /// Not enough credits were reserved for this page.
    InsufficientCredits,
    /// The transport failed (timeout, connection, rate limit exhaustion).
    TransportFailed(String),
    /// The connection ended after the request may have reached the provider;
    /// upstream usage and billing cannot be proven from the client side.
    TransportOutcomeUnknown(String),
    /// The response could not be parsed under the strict schema.
    ResponseInvalid(String),
    /// The transcription aligned to too few of the locally detected lines.
    AlignmentCoverageBelowGate { coverage: f32, required: f32 },
    /// The structured boxes failed a geometry gate.
    GeometryGateFailed(String),
    /// The provider produced nothing for this page.
    EmptyTranscription,
    /// The run was cancelled while this page was in flight.
    Cancelled,
}

impl FallbackReason {
    /// A short, stable code for reports, evidence parameters and UI. The free
    /// text detail is separate and already redacted.
    pub fn code(&self) -> &'static str {
        match self {
            Self::CredentialMissing => "credential_missing",
            Self::CredentialRejected => "credential_rejected",
            Self::InsufficientCredits => "insufficient_credits",
            Self::TransportFailed(_) => "transport_failed",
            Self::TransportOutcomeUnknown(_) => "transport_outcome_unknown",
            Self::ResponseInvalid(_) => "response_invalid",
            Self::AlignmentCoverageBelowGate { .. } => "alignment_coverage_below_gate",
            Self::GeometryGateFailed(_) => "geometry_gate_failed",
            Self::EmptyTranscription => "empty_transcription",
            Self::Cancelled => "cancelled",
        }
    }

    /// A one-line, already-redacted description safe for logs and UI.
    pub fn describe(&self) -> String {
        match self {
            Self::TransportFailed(detail)
            | Self::TransportOutcomeUnknown(detail)
            | Self::ResponseInvalid(detail)
            | Self::GeometryGateFailed(detail) => {
                format!("{}: {}", self.code(), redaction::redact(detail))
            }
            Self::AlignmentCoverageBelowGate { coverage, required } => format!(
                "{}: {:.3} of local lines aligned, {:.3} required",
                self.code(),
                coverage,
                required
            ),
            other => other.code().to_owned(),
        }
    }
}

/// What a provider can do, asked before anything is sent anywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrProviderCapabilities {
    pub mode: OcrProviderMode,
    pub provider_name: String,
    pub model: String,
    pub model_version: String,
    pub uses_network: bool,
    pub requires_credential: bool,
    /// Whether this provider *can* return line rectangles at all.
    pub supports_structured_bbox: bool,
    /// Whether those rectangles are trusted enough to be the default
    /// coordinate source. This is `false` for every provider in this build;
    /// see [`structured_bbox`] for the gates that would have to pass first.
    pub structured_bbox_default_enabled: bool,
    /// Whether this provider is cleared to be used for real work, as opposed
    /// to being wired up but not backed by a production service.
    pub production_ready: bool,
    /// Why not, when `production_ready` is false. Shown verbatim by the CLI
    /// and the desktop app so neither can claim more than is true.
    pub not_production_ready_reason: Option<String>,
}

/// Everything a provider is told about one page.
///
/// There is no credential field, by construction. A provider that needs one
/// resolved it from the OS credential store when it was built, holds it in a
/// non-serializable wrapper, and never receives it through a request that
/// could be logged, checkpointed, or sent over IPC.
pub struct PageOcrRequest<'a> {
    pub document_sha256: &'a str,
    pub page_index: u32,
    pub page_image_sha256: &'a str,
    pub page_image: &'a DynamicImage,
    pub language_profile: &'a str,
    pub dpi: u16,
    /// Identity of the local detector whose geometry a cloud transcription
    /// will be aligned to. Part of the checkpoint fingerprint.
    pub local_layout_version: &'a str,
    pub requested_output_contract: OutputContract,
    pub deadline: Duration,
    /// Stable across retries of the same page in the same job, so a retry
    /// never bills twice. Derived, never random.
    pub idempotency_key: &'a str,
}

/// What the caller wants back. A provider may return *less* structure than
/// asked (and say so through [`GeometrySource`]), never more trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputContract {
    /// Full-page transcription only; geometry comes from the local detector.
    #[default]
    TranscriptionOnly,
    /// Structured line rectangles, still cross-validated against the local
    /// detector before any of them is used.
    StructuredLines,
}

impl OutputContract {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TranscriptionOnly => "transcription_only",
            Self::StructuredLines => "structured_lines",
        }
    }
}

/// Consumption for one page, as reported by the provider.
///
/// `credits_charged` is populated only by brokered execution; a BYOK run
/// costs the user money at their own provider and this project never invents
/// a number for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub credits_charged: u64,
    pub requests: u32,
}

impl ProviderUsage {
    pub fn add(&mut self, other: ProviderUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.credits_charged = self.credits_charged.saturating_add(other.credits_charged);
        self.requests = self.requests.saturating_add(other.requests);
    }
}

/// One page's result, plus every field a reviewer needs to decide how much to
/// trust it.
#[derive(Debug, Clone)]
pub struct PageOcrOutcome {
    pub page: OcrPage,
    pub provider_mode: OcrProviderMode,
    pub provider_name: String,
    pub model: String,
    pub model_version: String,
    /// Digest of the exact raw transcription. The transcription itself is not
    /// retained by default: the canonical block tree is the evidence, and a
    /// second free-text copy of the page is an extra place for a secret or a
    /// privacy problem to hide.
    pub raw_transcription_digest: Option<String>,
    pub geometry_source: GeometrySource,
    pub alignment_version: &'static str,
    pub prompt_config_digest: Option<String>,
    pub usage: ProviderUsage,
    pub warnings: Vec<String>,
    pub fallback_reason: Option<FallbackReason>,
    pub execution_location: String,
    /// Per-line reason codes: what was replaced, kept, or dropped and why.
    pub decisions: alignment::DecisionSummary,
}

impl PageOcrOutcome {
    /// Flattens the provenance into the `parameters` map that
    /// [`crate::ocr::OcrProviderProvenance`] already persists per page.
    ///
    /// Doing it this way is deliberate: the OCR evidence schema does not
    /// change, every existing reader keeps working, and provider provenance
    /// still lands in the durable record that the checkpoint digests.
    pub fn provenance_parameters(&self) -> BTreeMap<String, String> {
        let mut parameters = BTreeMap::new();
        parameters.insert("provider_mode".into(), self.provider_mode.id().into());
        parameters.insert("provider_name".into(), self.provider_name.clone());
        parameters.insert("model_version".into(), self.model_version.clone());
        parameters.insert(
            "geometry_source".into(),
            self.geometry_source.as_str().into(),
        );
        parameters.insert("alignment_version".into(), self.alignment_version.into());
        parameters.insert(
            "provider_contract".into(),
            PROVIDER_CONTRACT_VERSION.to_owned(),
        );
        if let Some(digest) = &self.prompt_config_digest {
            parameters.insert("prompt_config_digest".into(), digest.clone());
        }
        if let Some(digest) = &self.raw_transcription_digest {
            parameters.insert("raw_transcription_sha256".into(), digest.clone());
        }
        if let Some(reason) = &self.fallback_reason {
            parameters.insert("fallback_reason".into(), reason.code().to_owned());
            parameters.insert("fallback_detail".into(), reason.describe());
        }
        parameters.insert("lines_replaced".into(), self.decisions.replaced.to_string());
        parameters.insert("lines_kept_local".into(), self.decisions.kept.to_string());
        parameters.insert(
            "lines_dropped_provider".into(),
            self.decisions.dropped.to_string(),
        );
        parameters.insert(
            "alignment_coverage".into(),
            format!("{:.4}", self.decisions.coverage),
        );
        if self.usage != ProviderUsage::default() {
            parameters.insert(
                "usage_input_tokens".into(),
                self.usage.input_tokens.to_string(),
            );
            parameters.insert(
                "usage_output_tokens".into(),
                self.usage.output_tokens.to_string(),
            );
            parameters.insert(
                "usage_credits_charged".into(),
                self.usage.credits_charged.to_string(),
            );
            parameters.insert("usage_requests".into(), self.usage.requests.to_string());
        }
        parameters
    }
}

/// What a provider needs to know before the first page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPreparation {
    pub job_id: String,
    pub document_sha256: String,
    pub page_indices: Vec<u32>,
    pub language_profile: String,
    pub dpi: u16,
    /// Hard ceiling the caller authorized. A provider must refuse to start
    /// rather than exceed it.
    pub max_credits: u64,
}

/// The provider's acknowledgement that a job may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTicket {
    pub job_id: String,
    /// Present only for brokered execution.
    pub reservation_id: Option<String>,
    pub reserved_credits: u64,
    pub estimated_credits: u64,
}

/// What the provider says happened once every page is done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobSettlement {
    pub charged_credits: u64,
    pub refunded_credits: u64,
    pub released_credits: u64,
    pub settled: bool,
}

/// A configuration problem, reported before anything runs and before any
/// output file is touched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderConfigError {
    #[error("no credential is stored for slot {slot}")]
    CredentialMissing { slot: String },
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Invalid(String),
    #[error("not cleared for production use: {0}")]
    NotProductionReady(String),
}

/// The provider-neutral OCR contract.
///
/// Implementations must be safe to call for the same `page_index` twice with
/// the same `idempotency_key` without double-charging.
pub trait OcrProvider {
    fn capabilities(&self) -> OcrProviderCapabilities;

    /// Everything that can be checked without spending anything: credentials
    /// present, model directory populated, endpoint well-formed, mode cleared
    /// for use. Must not perform a billable call.
    fn validate_configuration(&self) -> Result<(), ProviderConfigError>;

    fn prepare_job(&mut self, job: &JobPreparation) -> Result<JobTicket, OcrError>;

    fn recognize_page(&mut self, request: &PageOcrRequest<'_>) -> Result<PageOcrOutcome, OcrError>;

    fn cancel_job(&mut self, ticket: &JobTicket) -> Result<(), OcrError>;

    fn finalize_job(&mut self, ticket: &JobTicket) -> Result<JobSettlement, OcrError>;

    /// The provider's contribution to the durable job fingerprint.
    ///
    /// Must contain the provider mode, implementation version, model and
    /// model-version policy, prompt/config digest, alignment version and
    /// fallback policy — and must never contain key material, an
    /// `Authorization` header, or any reversible function of a secret.
    fn fingerprint_contribution(&self) -> String;
}

/// What to do when a cloud provider cannot produce a page.
///
/// The default is [`CloudFallback::Local`]: a partly-local searchable PDF is
/// strictly better than no PDF, and the per-page evidence records exactly
/// which pages fell back. `Fail` exists for callers who would rather stop
/// than ship mixed provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudFallback {
    #[default]
    Local,
    Fail,
}

impl CloudFallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Fail => "fail",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }
}

/// The result of a non-billable connection test.
///
/// Deliberately minimal: provider, whether the model answered, and a masked
/// credential reference. No account identifier, no headers, no raw provider
/// error body, no quota figures that could identify a billing account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionTest {
    pub mode: OcrProviderMode,
    pub provider_name: String,
    pub model: String,
    pub model_available: bool,
    pub credential: redaction::MaskedCredential,
    /// Already redacted and bounded.
    pub diagnostic: String,
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn digest_str(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_the_only_default_and_the_only_offline_mode() {
        let defaults: Vec<_> = OcrProviderMode::ALL
            .into_iter()
            .filter(|mode| mode.is_default())
            .collect();
        assert_eq!(defaults, vec![OcrProviderMode::Local]);
        assert!(!OcrProviderMode::Local.uses_network());
        assert!(OcrProviderMode::GeminiByok.uses_network());
        assert!(OcrProviderMode::MpdfCredits.uses_network());
        assert_eq!(OcrProviderMode::default(), OcrProviderMode::Local);
    }

    #[test]
    fn every_mode_round_trips_through_its_id() {
        for mode in OcrProviderMode::ALL {
            assert_eq!(OcrProviderMode::parse(mode.id()), Some(mode));
        }
        assert_eq!(OcrProviderMode::parse("gemini"), None);
    }

    #[test]
    fn byok_and_brokered_execution_are_distinguishable_even_on_one_model() {
        // The same Gemini model can run either way. Provenance has to say
        // *who executed it*, because the privacy and billing stories differ.
        assert_ne!(
            OcrProviderMode::GeminiByok.execution_location(),
            OcrProviderMode::MpdfCredits.execution_location()
        );
    }

    #[test]
    fn cloud_fallback_defaults_to_keeping_a_usable_local_layer() {
        assert_eq!(CloudFallback::default(), CloudFallback::Local);
        assert_eq!(CloudFallback::parse("fail"), Some(CloudFallback::Fail));
        assert_eq!(CloudFallback::parse("silent"), None);
    }

    #[test]
    fn a_fallback_reason_never_leaks_the_provider_body_it_came_from() {
        let reason = FallbackReason::TransportFailed(
            "403 from provider: key=AIzaSyD-canary-0000000000000000000000".into(),
        );
        let described = reason.describe();
        assert!(!described.contains("AIzaSyD-canary"), "{described}");
        assert_eq!(reason.code(), "transport_failed");
    }
}
