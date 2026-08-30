//! The provider-neutral OCR and text-enhancement contracts.
//!
//! # The one rule this module exists to enforce
//!
//! Every complete OCR provider must independently hand back the *same*
//! canonical structure: [`crate::ocr::OcrPage`], a block/line/word tree with
//! measured boxes in the page's own pixel coordinate system. A provider is not
//! complete OCR merely because local code can align its plain text onto boxes
//! produced by another engine.
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
//! Text enhancement is a second, narrower contract. It consumes an existing
//! coordinate-bearing page and may return text patches bound to stable word
//! paths and source digests. It cannot return a replacement page or geometry.
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
pub mod text_enhancer;

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use image::DynamicImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::jobs::ExecutionLocation;
use crate::ocr::{OcrError, OcrPage};

/// Version of the provider contract itself. Bound into the checkpoint
/// fingerprint of any cloud run, so changing the contract cannot silently
/// reuse evidence produced under the previous one.
pub const PROVIDER_SCHEMA: &str = "mpdf-ocr-provider";
pub const PROVIDER_SCHEMA_VERSION: &str = "0.2";
pub const PROVIDER_CONTRACT_VERSION: &str = "mpdf-ocr-provider/2";

/// Stable product-level refusal for the retained legacy BYOK surface.
///
/// Both front ends use this exact text and reject before consulting a
/// credential store or constructing a transport.
pub const BYOK_DISABLED_MESSAGE: &str = "gemini-byok is disabled in this version; BYOK will be reconsidered only for an API that independently returns complete coordinate OCR";

/// Which recognized current or historical execution mode produced a page.
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
    /// All values accepted by historical evidence and request deserializers.
    /// `GeminiByok` remains here solely for compatibility.
    pub const ALL: [OcrProviderMode; 3] = [Self::Local, Self::GeminiByok, Self::MpdfCredits];
    /// Modes a current product surface may offer for a new run.
    pub const PRODUCT_MODES: [OcrProviderMode; 2] = [Self::Local, Self::MpdfCredits];

    pub fn id(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::GeminiByok => "gemini-byok",
            Self::MpdfCredits => "mpdf-credits",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Local => "Local OCR plugin",
            Self::GeminiByok => "Gemini BYOK (legacy, disabled)",
            Self::MpdfCredits => "M PDF Cloud OCR",
        }
    }

    pub fn availability(self) -> &'static str {
        match self {
            Self::Local => "stable",
            Self::GeminiByok => "disabled",
            Self::MpdfCredits => "unavailable",
        }
    }

    pub fn release_blockers(self) -> Vec<&'static str> {
        match self {
            Self::Local => Vec::new(),
            Self::GeminiByok => vec![BYOK_DISABLED_MESSAGE],
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
        self.execution_location_kind().as_str()
    }

    /// Typed durable provenance for OCR page records and job checkpoints.
    /// The historical BYOK variant is intentionally only a value mapping;
    /// it does not change that mode's disabled product availability.
    pub const fn execution_location_kind(self) -> ExecutionLocation {
        match self {
            Self::Local => ExecutionLocation::Local,
            Self::GeminiByok => ExecutionLocation::LegacyUserKey,
            Self::MpdfCredits => ExecutionLocation::BrokeredCloud,
        }
    }
}

/// The role a component plays. Only `CompleteOcr` may originate OCR evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRole {
    CompleteOcr,
    TextEnhancer,
    /// Plain text plus geometry supplied by a separate local OCR engine.
    /// Retained for reproducibility; never a complete-OCR claim.
    #[default]
    ExperimentalComposite,
}

/// Finest coordinate unit independently returned by the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateGranularity {
    #[default]
    None,
    Line,
    Word,
}

/// Whether reading order is part of the provider's machine contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingOrderCapability {
    #[default]
    None,
    BestEffort,
    Stable,
}

/// How returned text is bound to returned geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextGeometryMapping {
    #[default]
    None,
    /// Coordinates came from another recognizer and text was aligned locally.
    LocalAlignment,
    /// Every returned text unit directly identifies its returned box.
    Direct,
}

/// Finest level at which optional metadata is available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSupport {
    #[default]
    Unsupported,
    Page,
    Line,
    Word,
}

/// How use of a provider is paid for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingModel {
    #[default]
    Free,
    UserManaged,
    BrokeredCredits,
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
#[serde(deny_unknown_fields)]
pub struct OcrProviderCapabilities {
    pub mode: OcrProviderMode,
    pub provider_name: String,
    pub model: String,
    pub model_version: String,
    /// A machine-readable classification. A text-only model aligned onto
    /// another engine's boxes must declare `experimental_composite`.
    #[serde(default)]
    pub role: ProviderRole,
    /// At least `line` is required for complete OCR.
    #[serde(default)]
    pub coordinate_granularity: CoordinateGranularity,
    /// Complete OCR requires stable, provider-owned reading order.
    #[serde(default)]
    pub reading_order: ReadingOrderCapability,
    /// Complete OCR requires direct text-to-box identity suitable for the
    /// hidden PDF text layer. Local post-hoc alignment does not qualify.
    #[serde(default)]
    pub text_geometry_mapping: TextGeometryMapping,
    #[serde(default)]
    pub confidence_metadata: MetadataSupport,
    #[serde(default)]
    pub language_metadata: MetadataSupport,
    #[serde(default)]
    pub billing: BillingModel,
    #[serde(default)]
    pub requires_explicit_consent: bool,
    #[serde(default)]
    pub requires_cost_limit: bool,
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

impl OcrProviderCapabilities {
    /// Proves that this is an independently complete OCR capability.
    ///
    /// Production readiness is intentionally checked separately: an
    /// unavailable future backend may describe a valid complete-OCR contract,
    /// while an available plain-text model still cannot pass this gate.
    pub fn validate_complete_ocr(&self) -> Result<(), ProviderContractError> {
        if self.role != ProviderRole::CompleteOcr {
            return Err(ProviderContractError::NotCompleteOcrRole(self.role));
        }
        if self.coordinate_granularity == CoordinateGranularity::None
            || !self.supports_structured_bbox
        {
            return Err(ProviderContractError::CoordinatesRequired);
        }
        if self.reading_order != ReadingOrderCapability::Stable {
            return Err(ProviderContractError::StableReadingOrderRequired);
        }
        if self.text_geometry_mapping != TextGeometryMapping::Direct {
            return Err(ProviderContractError::DirectTextGeometryMappingRequired);
        }
        if self.mode == OcrProviderMode::GeminiByok {
            return Err(ProviderContractError::ByokDisabled);
        }
        match self.mode {
            OcrProviderMode::GeminiByok => unreachable!("rejected above"),
            OcrProviderMode::Local => {
                if self.billing != BillingModel::Free
                    || self.uses_network
                    || self.requires_credential
                    || self.requires_explicit_consent
                    || self.requires_cost_limit
                {
                    return Err(ProviderContractError::ModePolicyMismatch);
                }
            }
            OcrProviderMode::MpdfCredits => {
                if self.billing != BillingModel::BrokeredCredits
                    || !self.uses_network
                    || self.requires_credential
                    || !self.requires_explicit_consent
                    || !self.requires_cost_limit
                {
                    return Err(ProviderContractError::BrokeredCloudControlsRequired);
                }
            }
        }
        if self.provider_name.trim().is_empty()
            || self.provider_name.chars().count() > 256
            || self.model.trim().is_empty()
            || self.model.chars().count() > 256
            || self.model_version.trim().is_empty()
            || self.model_version.chars().count() > 128
            || self.model_version.eq_ignore_ascii_case("latest")
        {
            return Err(ProviderContractError::PinnedIdentityRequired);
        }
        Ok(())
    }
}

/// A serializable 0.2 provider contract manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrProviderContractV2 {
    pub schema: String,
    pub schema_version: String,
    pub capabilities: OcrProviderCapabilities,
}

impl OcrProviderContractV2 {
    pub fn new(capabilities: OcrProviderCapabilities) -> Self {
        Self {
            schema: PROVIDER_SCHEMA.to_owned(),
            schema_version: PROVIDER_SCHEMA_VERSION.to_owned(),
            capabilities,
        }
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema != PROVIDER_SCHEMA || self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(ProviderContractError::UnsupportedSchema {
                schema: self.schema.clone(),
                version: self.schema_version.clone(),
            });
        }
        self.capabilities.validate_complete_ocr()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderContractError {
    #[error("provider role {0:?} is not complete OCR")]
    NotCompleteOcrRole(ProviderRole),
    #[error("complete OCR must independently return line or word coordinates")]
    CoordinatesRequired,
    #[error("complete OCR must return stable reading order")]
    StableReadingOrderRequired,
    #[error("complete OCR must directly map returned text to returned geometry")]
    DirectTextGeometryMappingRequired,
    #[error("gemini-byok is disabled in this version; BYOK will be reconsidered only for an API that independently returns complete coordinate OCR")]
    ByokDisabled,
    #[error("brokered cloud OCR requires explicit consent and a hard cost limit")]
    BrokeredCloudControlsRequired,
    #[error(
        "provider mode, network, credential, consent, cost, and billing declarations conflict"
    )]
    ModePolicyMismatch,
    #[error("provider, model, and a pinned model version are required")]
    PinnedIdentityRequired,
    #[error("unsupported provider contract {schema}/{version}")]
    UnsupportedSchema { schema: String, version: String },
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
    pub execution_location: ExecutionLocation,
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

/// Shared lifecycle for provider implementations.
///
/// Implementations must be safe to call for the same `page_index` twice with
/// the same `idempotency_key` without double-charging. Implementing this trait
/// is not itself a complete-OCR claim; callers must validate
/// [`OcrProviderCapabilities::validate_complete_ocr`]. This distinction keeps
/// the retained plain-text-plus-local-alignment experiment from masquerading
/// as independent OCR.
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

/// Marker implemented only by providers intended to originate complete OCR.
///
/// The runtime capability gate remains mandatory because a marker alone cannot
/// prove what a remote response contains.
pub trait CompleteOcrProvider: OcrProvider {}

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

    fn complete_capabilities() -> OcrProviderCapabilities {
        OcrProviderCapabilities {
            mode: OcrProviderMode::Local,
            provider_name: "fixture-coordinate-ocr".into(),
            model: "fixture".into(),
            model_version: "1.0.0".into(),
            role: ProviderRole::CompleteOcr,
            coordinate_granularity: CoordinateGranularity::Word,
            reading_order: ReadingOrderCapability::Stable,
            text_geometry_mapping: TextGeometryMapping::Direct,
            confidence_metadata: MetadataSupport::Word,
            language_metadata: MetadataSupport::Line,
            billing: BillingModel::Free,
            requires_explicit_consent: false,
            requires_cost_limit: false,
            uses_network: false,
            requires_credential: false,
            supports_structured_bbox: true,
            structured_bbox_default_enabled: true,
            production_ready: true,
            not_production_ready_reason: None,
        }
    }

    struct FixtureCompleteProvider;

    impl OcrProvider for FixtureCompleteProvider {
        fn capabilities(&self) -> OcrProviderCapabilities {
            complete_capabilities()
        }

        fn validate_configuration(&self) -> Result<(), ProviderConfigError> {
            Ok(())
        }

        fn prepare_job(&mut self, job: &JobPreparation) -> Result<JobTicket, OcrError> {
            Ok(JobTicket {
                job_id: job.job_id.clone(),
                reservation_id: None,
                reserved_credits: 0,
                estimated_credits: 0,
            })
        }

        fn recognize_page(
            &mut self,
            _request: &PageOcrRequest<'_>,
        ) -> Result<PageOcrOutcome, OcrError> {
            Err(OcrError::ProviderUnavailable(
                "fixture has no page script".into(),
            ))
        }

        fn cancel_job(&mut self, _ticket: &JobTicket) -> Result<(), OcrError> {
            Ok(())
        }

        fn finalize_job(&mut self, _ticket: &JobTicket) -> Result<JobSettlement, OcrError> {
            Ok(JobSettlement::default())
        }

        fn fingerprint_contribution(&self) -> String {
            "fixture-coordinate-ocr/1".into()
        }
    }

    impl CompleteOcrProvider for FixtureCompleteProvider {}

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
    fn current_product_modes_exclude_legacy_byok_without_breaking_its_parse() {
        assert_eq!(
            OcrProviderMode::PRODUCT_MODES,
            [OcrProviderMode::Local, OcrProviderMode::MpdfCredits]
        );
        assert_eq!(
            OcrProviderMode::parse("gemini-byok"),
            Some(OcrProviderMode::GeminiByok)
        );
        assert_eq!(OcrProviderMode::GeminiByok.availability(), "disabled");
    }

    #[test]
    fn zero_one_capabilities_still_deserialize_but_never_gain_complete_ocr_status() {
        let legacy = serde_json::json!({
            "mode": "gemini-byok",
            "provider_name": "legacy-gemini",
            "model": "gemini-2.5-flash",
            "model_version": "2025-06",
            "uses_network": true,
            "requires_credential": true,
            "supports_structured_bbox": false,
            "structured_bbox_default_enabled": false,
            "production_ready": false,
            "not_production_ready_reason": "historical fixture"
        });
        let capabilities: OcrProviderCapabilities = serde_json::from_value(legacy).unwrap();
        assert_eq!(capabilities.mode, OcrProviderMode::GeminiByok);
        assert_eq!(capabilities.role, ProviderRole::ExperimentalComposite);
        assert_eq!(
            capabilities.validate_complete_ocr(),
            Err(ProviderContractError::NotCompleteOcrRole(
                ProviderRole::ExperimentalComposite
            ))
        );
    }

    #[test]
    fn incomplete_capabilities_are_rejected_but_coordinate_ocr_passes() {
        let provider: &dyn CompleteOcrProvider = &FixtureCompleteProvider;
        provider.capabilities().validate_complete_ocr().unwrap();
        let mut incomplete = complete_capabilities();
        incomplete.coordinate_granularity = CoordinateGranularity::None;
        assert_eq!(
            incomplete.validate_complete_ocr(),
            Err(ProviderContractError::CoordinatesRequired)
        );
        let mut contradictory = complete_capabilities();
        contradictory.supports_structured_bbox = false;
        assert_eq!(
            contradictory.validate_complete_ocr(),
            Err(ProviderContractError::CoordinatesRequired)
        );
        let mut incomplete = complete_capabilities();
        incomplete.reading_order = ReadingOrderCapability::BestEffort;
        assert_eq!(
            incomplete.validate_complete_ocr(),
            Err(ProviderContractError::StableReadingOrderRequired)
        );
        let mut incomplete = complete_capabilities();
        incomplete.text_geometry_mapping = TextGeometryMapping::LocalAlignment;
        assert_eq!(
            incomplete.validate_complete_ocr(),
            Err(ProviderContractError::DirectTextGeometryMappingRequired)
        );
        let mut contradictory = complete_capabilities();
        contradictory.uses_network = true;
        assert_eq!(
            contradictory.validate_complete_ocr(),
            Err(ProviderContractError::ModePolicyMismatch)
        );
    }

    #[test]
    fn a_plain_text_composite_cannot_claim_complete_ocr() {
        let mut capabilities = complete_capabilities();
        capabilities.role = ProviderRole::ExperimentalComposite;
        capabilities.coordinate_granularity = CoordinateGranularity::None;
        capabilities.text_geometry_mapping = TextGeometryMapping::LocalAlignment;
        assert_eq!(
            capabilities.validate_complete_ocr(),
            Err(ProviderContractError::NotCompleteOcrRole(
                ProviderRole::ExperimentalComposite
            ))
        );
    }

    #[test]
    fn a_complete_credits_provider_must_be_paid_brokered_and_budgeted() {
        let mut capabilities = complete_capabilities();
        capabilities.mode = OcrProviderMode::MpdfCredits;
        capabilities.billing = BillingModel::BrokeredCredits;
        capabilities.uses_network = true;
        capabilities.requires_explicit_consent = true;
        capabilities.requires_cost_limit = true;
        capabilities.validate_complete_ocr().unwrap();

        capabilities.requires_cost_limit = false;
        assert_eq!(
            capabilities.validate_complete_ocr(),
            Err(ProviderContractError::BrokeredCloudControlsRequired)
        );
    }

    #[test]
    fn provider_0_2_rust_shape_matches_the_strict_schema() {
        let contract = OcrProviderContractV2::new(complete_capabilities());
        contract.validate().unwrap();
        let actual = serde_json::to_value(&contract).unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../schemas/mpdf-ocr-provider-0.2.schema.json"
        ))
        .unwrap();
        assert_eq!(actual["schema"], schema["properties"]["schema"]["const"]);
        assert_eq!(
            actual["schema_version"],
            schema["properties"]["schema_version"]["const"]
        );
        fn keys(value: &serde_json::Value) -> std::collections::BTreeSet<String> {
            value.as_object().unwrap().keys().cloned().collect()
        }
        let expected_top: std::collections::BTreeSet<String> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys(&actual), expected_top);
        let expected_capabilities: std::collections::BTreeSet<String> = schema["$defs"]
            ["capabilities"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys(&actual["capabilities"]), expected_capabilities);
        assert_eq!(
            schema["$defs"]["capabilities"]["additionalProperties"],
            false
        );
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn byok_and_brokered_execution_are_distinguishable_even_on_one_model() {
        // The same Gemini model can run either way. Provenance has to say
        // *who executed it*, because the privacy and billing stories differ.
        assert_eq!(
            OcrProviderMode::GeminiByok.execution_location_kind(),
            ExecutionLocation::LegacyUserKey
        );
        assert_eq!(
            OcrProviderMode::MpdfCredits.execution_location_kind(),
            ExecutionLocation::BrokeredCloud
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
