//! The cloud OCR provider: a full-page transcription joined to local geometry.
//!
//! One type serves both cloud modes. `gemini-byok` and `mpdf-credits` run the
//! same model with the same prompt and the same gates; what differs is *who
//! holds the credential* and *who is billed*, and those differences are
//! expressed as a transport and an optional credits backend rather than as a
//! second copy of the recognition logic. Two implementations would drift, and
//! the one that drifted would be the one nobody was watching.
//!
//! # Order of operations, and why it is not negotiable
//!
//! ```text
//!   original page raster
//!     -> LOCAL detector           : blocks, lines, words, rectangles
//!     -> cloud transcription      : characters for the whole page
//!     -> deterministic line split
//!     -> monotone alignment       : which characters belong to which rectangle
//!     -> gates                    : coverage, order, geometry
//!     -> canonical OcrPage        : local rectangles, cloud text
//! ```
//!
//! The local detector runs *first and always*, including on pages the cloud
//! call will later fail. That is what makes fallback free: the page that
//! falls back is already recognized, and the run produces a searchable PDF
//! rather than a page-shaped hole.

use std::io::Cursor;
use std::time::Duration;

use image::ImageFormat;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ocr::{OcrError, OcrPage, OcrProviderProvenance, PageOcrProvider};

use super::alignment::{self, AlignmentConfig, DecisionSummary, ALIGNMENT_VERSION};
use super::credits::{
    self, CreditsBackend, CreditsError, PageCharge, ReleaseReason, ReservationRequest,
};
use super::redaction;
use super::structured_bbox::{self, GeometryGates, StructuredBboxPolicy};
use super::{
    digest_str, CloudFallback, FallbackReason, GeometrySource, JobPreparation, JobSettlement,
    JobTicket, OcrProvider, OcrProviderCapabilities, OcrProviderMode, OutputContract,
    PageOcrOutcome, PageOcrRequest, ProviderConfigError, ProviderUsage,
};

/// Version of this provider implementation. Part of every cloud fingerprint,
/// so a change to the recognition logic invalidates resumable jobs.
pub const CLOUD_PROVIDER_VERSION: &str = "mpdf-cloud-ocr/2";

/// The model this build is calibrated against.
///
/// The measured baseline: 63 full pages of polytonic Greek at CER 0.0318 /
/// WER 0.0885 with no region hints, and an 8-page Greek/German/Latin mixed
/// holdout at CER 0.0597 against 0.1398 for the combined local pass. Changing
/// the model invalidates that calibration, which is why the name and the
/// pinned version are both in the fingerprint.
pub const DEFAULT_MODEL: &str = "gemini-3.7-flash";

/// Prompt identity. Editing [`PromptConfig::default`] without bumping this is
/// a bug: the digest would change, which is correct, but the human-readable
/// version is what a report shows.
pub const PROMPT_VERSION: &str = "full-page-transcription/2";

/// How the page is described to the model.
///
/// Every clause here is load-bearing and was chosen against the failure modes
/// the holdout showed: summarizing instead of transcribing, "fixing" the
/// orthography of polytonic Greek, transliterating Greek into Latin, dropping
/// the printed page number on a contents page, and inventing a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptConfig {
    pub version: &'static str,
    pub instruction: String,
    /// Milli-units so the prompt identity stays exactly comparable across
    /// runs; a float in a fingerprint is a formatting accident waiting to
    /// happen.
    pub temperature_milli: u16,
    pub max_output_tokens: u32,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            version: PROMPT_VERSION,
            instruction: concat!(
                "Transcribe every visible character on this page image, exactly as printed.\n",
                "Rules:\n",
                "1. Output plain text only. One output line per printed line, in reading order. ",
                "No JSON, no markdown, no commentary, no page summary.\n",
                "2. Do not translate, modernize, expand abbreviations, or correct spelling, ",
                "punctuation or orthography.\n",
                "3. Preserve the original script. Ancient Greek stays in Greek characters with ",
                "all polytonic diacritics: breathings, acute, grave, circumflex, iota subscript ",
                "and diaeresis. Never transliterate Greek into Latin letters.\n",
                "4. Preserve German umlauts and sharp s exactly (ä ö ü Ä Ö Ü ß).\n",
                "5. Transcribe running heads, folios, printed page numbers, footnote markers and ",
                "footnote text. They are part of the page.\n",
                "6. Keep the printed line breaks. Do not join lines, do not re-wrap, do not ",
                "reflow columns; read the left column fully, then the right column.\n",
                "7. If a character is illegible, output U+FFFD once in its place rather than ",
                "guessing a word.\n",
                "8. Output nothing except the transcription."
            )
            .to_owned(),
            // Transcription is not a creative task and a resumable checkpoint
            // needs the same input to give the same output as often as the
            // provider allows.
            temperature_milli: 0,
            // The measured full-page runs used this ceiling. Dense critical
            // apparatus pages can exceed 8k tokens; truncation is a failed
            // transcription, not a shorter successful page.
            max_output_tokens: 32_768,
        }
    }
}

impl PromptConfig {
    /// Stable identity of the prompt *and* its decoding parameters. Bound
    /// into the checkpoint fingerprint: a page transcribed under a different
    /// prompt is different evidence.
    pub fn digest(&self) -> String {
        digest_str(&format!(
            "{}|temp_milli={}|max_tokens={}|{}",
            self.version, self.temperature_milli, self.max_output_tokens, self.instruction
        ))
    }

    /// The exact instruction sent for one output contract.
    ///
    /// Structured evaluation cannot reuse the plain-text instruction: that
    /// instruction explicitly forbids JSON. Keeping the variant here makes
    /// the request, fingerprint and evidence describe the same prompt.
    pub fn instruction_for(
        &self,
        contract: OutputContract,
        page_width: u32,
        page_height: u32,
    ) -> String {
        match contract {
            OutputContract::TranscriptionOnly => self.instruction.clone(),
            OutputContract::StructuredLines => format!(
                "Transcribe every visible character on this page image exactly as printed.\n\
                 Return exactly one JSON object and no markdown or commentary. It must have only \
                 these fields: {{\"schema_version\":\"{}\",\"page_width\":{},\
                 \"page_height\":{},\"lines\":[{{\"text\":\"...\",\"bbox\":[x,y,width,height],\
                 \"reading_order\":0}}]}}. page_width and page_height must equal the supplied \
                 image dimensions shown above. Coordinates are pixels from the top-left. Emit one \
                 entry per printed line in strict reading order, left column fully before right \
                 column. Preserve every script, line, folio, footnote, punctuation mark, abbreviation, \
                 German umlaut, sharp s, Greek breathing, accent, diaeresis and iota subscript. Do not \
                 translate, modernize, correct, expand, summarize, reflow, normalize orthography or \
                 invent illegible text. Use U+FFFD once for an illegible character.",
                structured_bbox::STRUCTURED_BBOX_SCHEMA_VERSION,
                page_width,
                page_height,
            ),
        }
    }

    pub fn digest_for(&self, contract: OutputContract) -> String {
        digest_str(&format!(
            "{}|contract={}|structured_schema={}",
            self.digest(),
            contract.as_str(),
            structured_bbox::STRUCTURED_BBOX_SCHEMA_VERSION
        ))
    }
}

/// Everything about a cloud run that is not a secret.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudOcrConfig {
    pub mode: OcrProviderMode,
    pub provider_name: String,
    pub model: String,
    /// Pinned, not "latest". A floating alias would silently re-run a resumed
    /// job against different weights.
    pub model_version: String,
    pub prompt: PromptConfig,
    pub output_contract: OutputContract,
    pub structured_bbox: StructuredBboxPolicy,
    pub gates: GeometryGates,
    pub alignment: AlignmentConfig,
    pub fallback: CloudFallback,
    pub deadline: Duration,
    /// Credits reserved per page, used to size a reservation before anything
    /// is uploaded. Zero for BYOK, where this project never invents a price.
    pub credits_per_page: u64,
    /// Credential slot name. A *reference*, never key material: this value is
    /// safe to serialize, log, and put in a report.
    pub credential_slot: String,
}

impl CloudOcrConfig {
    pub fn gemini_byok(credential_slot: impl Into<String>) -> Self {
        Self {
            mode: OcrProviderMode::GeminiByok,
            provider_name: "google-gemini".into(),
            model: DEFAULT_MODEL.into(),
            model_version: "2026-08".into(),
            prompt: PromptConfig::default(),
            output_contract: OutputContract::TranscriptionOnly,
            structured_bbox: StructuredBboxPolicy::default(),
            gates: GeometryGates::default(),
            alignment: AlignmentConfig::default(),
            fallback: CloudFallback::default(),
            deadline: Duration::from_secs(120),
            credits_per_page: 0,
            credential_slot: credential_slot.into(),
        }
    }

    pub fn mpdf_credits(credits_per_page: u64) -> Self {
        Self {
            mode: OcrProviderMode::MpdfCredits,
            credits_per_page,
            credential_slot: "mpdf-credits-session".into(),
            ..Self::gemini_byok("mpdf-credits-session")
        }
    }

    /// The digest that identifies this configuration to a reservation and to
    /// the checkpoint. Deliberately excludes the credential slot's *contents*
    /// — there are none here — and includes everything that changes what the
    /// evidence is.
    pub fn digest(&self) -> String {
        digest_str(&self.fingerprint())
    }

    pub fn fingerprint(&self) -> String {
        format!(
            "{CLOUD_PROVIDER_VERSION}|mode={}|provider={}|model={}|model_version={}|prompt={}|contract={}|{}|alignment={}|min_sim={:.3}|min_cov={:.3}|fallback={}|deadline_ms={}",
            self.mode.id(),
            self.provider_name,
            self.model,
            self.model_version,
            self.prompt.digest_for(self.output_contract),
            self.output_contract.as_str(),
            structured_bbox::fingerprint(self.structured_bbox, &self.gates),
            ALIGNMENT_VERSION,
            self.alignment.min_line_similarity,
            self.alignment.min_coverage,
            self.fallback.as_str(),
            self.deadline.as_millis(),
        )
    }
}

/// What a transport is asked to do for one page.
///
/// No credential field: a transport was constructed with whatever authority
/// it needs and holds it privately. This struct is safe to log.
#[derive(Debug, Clone)]
pub struct TranscriptionRequest<'a> {
    pub model: &'a str,
    pub model_version: &'a str,
    pub prompt: &'a PromptConfig,
    pub output_contract: OutputContract,
    /// PNG bytes of the original page raster.
    pub page_png: &'a [u8],
    pub page_width: u32,
    pub page_height: u32,
    pub page_image_sha256: &'a str,
    pub page_index: u32,
    pub language_profile: &'a str,
    pub deadline: Duration,
    pub idempotency_key: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionResponse {
    /// Plain transcription, or the structured JSON body when the contract
    /// asked for one.
    pub body: String,
    pub model_version: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("credential missing")]
    CredentialMissing,
    #[error("credential rejected")]
    CredentialRejected,
    #[error("transport failed: {0}")]
    Failed(String),
    #[error("transport outcome unknown: {0}")]
    OutcomeUnknown(String),
    #[error("response invalid: {0}")]
    InvalidResponse(String),
    #[error("cancelled")]
    Cancelled,
}

impl TransportError {
    /// Every construction site goes through here, so a provider body can
    /// never reach a caller unredacted.
    pub fn failed(detail: impl AsRef<str>) -> Self {
        Self::Failed(redaction::redact(detail.as_ref()))
    }

    pub fn invalid(detail: impl AsRef<str>) -> Self {
        Self::InvalidResponse(redaction::redact(detail.as_ref()))
    }

    pub fn outcome_unknown(detail: impl AsRef<str>) -> Self {
        Self::OutcomeUnknown(redaction::redact(detail.as_ref()))
    }

    fn to_fallback(&self) -> FallbackReason {
        match self {
            Self::CredentialMissing => FallbackReason::CredentialMissing,
            Self::CredentialRejected => FallbackReason::CredentialRejected,
            Self::Failed(detail) => FallbackReason::TransportFailed(detail.clone()),
            Self::OutcomeUnknown(detail) => FallbackReason::TransportOutcomeUnknown(detail.clone()),
            Self::InvalidResponse(detail) => FallbackReason::ResponseInvalid(detail.clone()),
            Self::Cancelled => FallbackReason::Cancelled,
        }
    }
}

/// The network seam. Implemented over HTTPS in `mpdf-api-client`; implemented
/// deterministically by [`ScriptedTransport`] for tests.
pub trait PageTranscriptionTransport {
    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<TranscriptionResponse, TransportError>;

    /// Non-billable reachability and credential check.
    fn probe(&mut self) -> Result<bool, TransportError>;

    /// Tells the transport which reservation this run is executing under.
    ///
    /// Called once, after the credits hold exists and before the first page.
    /// A direct BYOK transport has no reservation and ignores it; a brokered
    /// one must bind every page to the hold, so that a token issued for a
    /// three-page test cannot be replayed against a nine-hundred-page volume.
    fn bind_job(&mut self, _ticket: &JobTicket) {}

    /// Whether a credential is present, without reading it.
    fn credential_present(&self) -> bool;
}

/// A transport that answers from a fixed script. Used by every test in this
/// crate and by `--ocr-provider` dry runs, so no test can ever call a real
/// API or spend anything.
#[derive(Debug, Default)]
pub struct ScriptedTransport {
    pub answers: std::collections::BTreeMap<u32, Result<TranscriptionResponse, TransportError>>,
    pub default_answer: Option<TranscriptionResponse>,
    pub calls: Vec<u32>,
    pub credential: bool,
}

impl ScriptedTransport {
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            default_answer: Some(TranscriptionResponse {
                body: text.into(),
                model_version: "2026-08".into(),
                input_tokens: 1_200,
                output_tokens: 300,
            }),
            credential: true,
            ..Default::default()
        }
    }

    pub fn answer(
        mut self,
        page: u32,
        answer: Result<TranscriptionResponse, TransportError>,
    ) -> Self {
        self.answers.insert(page, answer);
        self
    }
}

impl PageTranscriptionTransport for ScriptedTransport {
    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<TranscriptionResponse, TransportError> {
        self.calls.push(request.page_index);
        if let Some(answer) = self.answers.get(&request.page_index) {
            return answer.clone();
        }
        self.default_answer
            .clone()
            .ok_or_else(|| TransportError::failed("no scripted answer"))
    }

    fn probe(&mut self) -> Result<bool, TransportError> {
        if self.credential {
            Ok(true)
        } else {
            Err(TransportError::CredentialMissing)
        }
    }

    fn credential_present(&self) -> bool {
        self.credential
    }
}

/// What [`CloudOcrProvider::finish`] was asked to record about one page:
/// where its coordinates came from, whether it fell back, what the alignment
/// decided, and what it cost.
struct PageVerdict {
    geometry_source: GeometrySource,
    fallback_reason: Option<FallbackReason>,
    summary: DecisionSummary,
    raw_digest: Option<String>,
    usage: ProviderUsage,
    model_version: String,
}

struct ResponseAudit {
    raw_digest: String,
    usage: ProviderUsage,
    model_version: String,
}

/// The cloud provider.
///
/// `local` is not optional and is not a fallback bolted on afterwards: it is
/// the geometry source for the normal, successful path.
pub struct CloudOcrProvider {
    config: CloudOcrConfig,
    transport: Box<dyn PageTranscriptionTransport>,
    local: Box<dyn PageOcrProvider>,
    local_layout_version: String,
    credits: Option<Box<dyn CreditsBackend>>,
    reservation_id: Option<String>,
    document_sha256: String,
    usage: ProviderUsage,
    /// Pages that did not use the cloud result, with why. Read by the
    /// orchestrator for the run report.
    fallbacks: Vec<(u32, FallbackReason)>,
}

impl CloudOcrProvider {
    pub fn new(
        config: CloudOcrConfig,
        transport: Box<dyn PageTranscriptionTransport>,
        local: Box<dyn PageOcrProvider>,
        local_layout_version: impl Into<String>,
    ) -> Self {
        Self {
            config,
            transport,
            local,
            local_layout_version: local_layout_version.into(),
            credits: None,
            reservation_id: None,
            document_sha256: String::new(),
            usage: ProviderUsage::default(),
            fallbacks: Vec::new(),
        }
    }

    pub fn with_credits(mut self, backend: Box<dyn CreditsBackend>) -> Self {
        self.credits = Some(backend);
        self
    }

    pub fn fallbacks(&self) -> &[(u32, FallbackReason)] {
        &self.fallbacks
    }

    pub fn usage(&self) -> ProviderUsage {
        self.usage
    }

    pub fn config(&self) -> &CloudOcrConfig {
        &self.config
    }

    /// The provider's whole checkpoint contribution.
    ///
    /// Note what is absent and must stay absent: the credential, any header,
    /// and any hash of a secret. The credential *slot* is a user-chosen label
    /// and is included so that switching accounts is visible, but rotating
    /// the key inside one slot deliberately does not invalidate evidence —
    /// the same model under the same prompt produced it.
    fn fingerprint(&self) -> String {
        format!(
            "{}|layout={}|credential_slot={}|credits_per_page={}",
            self.config.fingerprint(),
            self.local_layout_version,
            self.config.credential_slot,
            self.config.credits_per_page
        )
    }

    fn local_page(&mut self, request: &PageOcrRequest<'_>) -> Result<OcrPage, OcrError> {
        self.local.recognize(
            request.page_index,
            request.page_image,
            request.page_image_sha256,
        )
    }

    fn outcome_from_local(
        &mut self,
        page: OcrPage,
        request: &PageOcrRequest<'_>,
        reason: FallbackReason,
        summary: DecisionSummary,
    ) -> Result<PageOcrOutcome, OcrError> {
        if self.config.fallback == CloudFallback::Fail {
            return Err(OcrError::ProviderFailed {
                page: request.page_index,
                reason: format!(
                    "cloud OCR failed and --cloud-fallback=fail was requested: {}",
                    reason.describe()
                ),
            });
        }
        self.fallbacks.push((request.page_index, reason.clone()));
        // A page that fell back cost nothing at the provider, so its usage is
        // zero — which is also what keeps a fallback page out of the billed
        // total in the run report.
        Ok(self.finish(
            page,
            request,
            PageVerdict {
                geometry_source: GeometrySource::LocalLayout,
                fallback_reason: Some(reason),
                summary,
                raw_digest: None,
                usage: ProviderUsage::default(),
                model_version: self.config.model_version.clone(),
            },
        ))
    }

    fn outcome_from_local_after_response(
        &mut self,
        page: OcrPage,
        request: &PageOcrRequest<'_>,
        reason: FallbackReason,
        summary: DecisionSummary,
        audit: ResponseAudit,
    ) -> Result<PageOcrOutcome, OcrError> {
        if self.config.fallback == CloudFallback::Fail {
            return Err(OcrError::ProviderFailed {
                page: request.page_index,
                reason: format!(
                    "cloud OCR failed and --cloud-fallback=fail was requested: {}",
                    reason.describe()
                ),
            });
        }
        self.fallbacks.push((request.page_index, reason.clone()));
        Ok(self.finish(
            page,
            request,
            PageVerdict {
                geometry_source: GeometrySource::LocalLayout,
                fallback_reason: Some(reason),
                summary,
                raw_digest: Some(audit.raw_digest),
                usage: audit.usage,
                model_version: audit.model_version,
            },
        ))
    }

    fn finish(
        &self,
        mut page: OcrPage,
        request: &PageOcrRequest<'_>,
        verdict: PageVerdict,
    ) -> PageOcrOutcome {
        let PageVerdict {
            geometry_source,
            fallback_reason,
            summary,
            raw_digest,
            usage,
            model_version,
        } = verdict;
        let outcome = PageOcrOutcome {
            page: page.clone(),
            provider_mode: self.config.mode,
            provider_name: self.config.provider_name.clone(),
            model: self.config.model.clone(),
            model_version: model_version.clone(),
            raw_transcription_digest: raw_digest,
            geometry_source,
            alignment_version: ALIGNMENT_VERSION,
            prompt_config_digest: Some(self.config.prompt.digest()),
            usage,
            warnings: Vec::new(),
            fallback_reason,
            execution_location: self.config.mode.execution_location().to_owned(),
            decisions: summary,
        };
        // The provenance the durable evidence keeps is this outcome's own
        // description of itself, so nothing downstream has to reconstruct it.
        page.provider_provenance = Some(OcrProviderProvenance {
            engine: self.config.provider_name.clone(),
            model: self.config.model.clone(),
            version: CLOUD_PROVIDER_VERSION.to_owned(),
            parameters: outcome.provenance_parameters(),
            input_asset_sha256: request.page_image_sha256.to_owned(),
            execution_location: self.config.mode.execution_location().to_owned(),
            language_profile: Some(request.language_profile.to_owned()),
            model_digest: None,
            model_license: None,
            model_set: Some(format!("{} {}", self.config.model, model_version)),
        });
        // The raw transcription itself is deliberately not stored: the block
        // tree is the evidence, and a second free-text copy of every page is
        // an extra place for personal data to live.
        page.provider_raw_artifact = None;
        PageOcrOutcome { page, ..outcome }
    }
}

impl OcrProvider for CloudOcrProvider {
    fn capabilities(&self) -> OcrProviderCapabilities {
        let (production_ready, reason) = match self.config.mode {
            OcrProviderMode::MpdfCredits if credits::PRODUCTION_BACKEND.is_none() => {
                (false, Some(credits::release_blockers().join("; ")))
            }
            _ => (true, None),
        };
        OcrProviderCapabilities {
            mode: self.config.mode,
            provider_name: self.config.provider_name.clone(),
            model: self.config.model.clone(),
            model_version: self.config.model_version.clone(),
            uses_network: true,
            requires_credential: true,
            supports_structured_bbox: true,
            structured_bbox_default_enabled: structured_bbox::GATES_VALIDATED_FOR_DEFAULT,
            production_ready,
            not_production_ready_reason: reason,
        }
    }

    fn validate_configuration(&self) -> Result<(), ProviderConfigError> {
        if !self.transport.credential_present() {
            return Err(ProviderConfigError::CredentialMissing {
                slot: self.config.credential_slot.clone(),
            });
        }
        if self.config.model.is_empty() || self.config.model_version.is_empty() {
            return Err(ProviderConfigError::Invalid(
                "the model and its pinned version must both be set".into(),
            ));
        }
        if self.config.output_contract == OutputContract::StructuredLines
            && self.config.structured_bbox == StructuredBboxPolicy::Disabled
        {
            return Err(ProviderConfigError::Invalid(
                "structured line output was requested while the structured-bbox policy is disabled"
                    .into(),
            ));
        }
        if self.config.mode == OcrProviderMode::MpdfCredits
            && credits::PRODUCTION_BACKEND.is_none()
            && self.credits.is_none()
        {
            return Err(ProviderConfigError::NotProductionReady(
                credits::release_blockers().join("; "),
            ));
        }
        Ok(())
    }

    fn prepare_job(&mut self, job: &JobPreparation) -> Result<JobTicket, OcrError> {
        self.document_sha256 = job.document_sha256.clone();
        let estimated = self
            .config
            .credits_per_page
            .saturating_mul(job.page_indices.len() as u64);
        let Some(backend) = self.credits.as_mut() else {
            let ticket = JobTicket {
                job_id: job.job_id.clone(),
                reservation_id: None,
                reserved_credits: 0,
                estimated_credits: estimated,
            };
            self.transport.bind_job(&ticket);
            return Ok(ticket);
        };
        // The ceiling is checked before the hold, and the hold before any
        // page image is encoded, let alone uploaded.
        if estimated > job.max_credits {
            return Err(OcrError::ProviderUnavailable(format!(
                "this run needs about {estimated} credits but only {} were authorized",
                job.max_credits
            )));
        }
        let request = ReservationRequest::new(
            job.document_sha256.clone(),
            job.page_indices.first().copied().unwrap_or(0),
            job.page_indices.len() as u32,
            self.config.digest(),
            estimated,
            job.max_credits,
        );
        let reservation = backend
            .reserve(&request)
            .map_err(|error| map_credits_error(&error))?;
        self.reservation_id = Some(reservation.reservation_id.clone());
        let ticket = JobTicket {
            job_id: job.job_id.clone(),
            reservation_id: Some(reservation.reservation_id),
            reserved_credits: reservation.held_credits,
            estimated_credits: estimated,
        };
        self.transport.bind_job(&ticket);
        Ok(ticket)
    }

    fn recognize_page(&mut self, request: &PageOcrRequest<'_>) -> Result<PageOcrOutcome, OcrError> {
        // Step one, always: measure the page locally. A cloud failure after
        // this point still leaves usable local evidence. A transport timeout
        // can have unknown upstream usage, so it must not be described as
        // guaranteed zero cost.
        let local = self.local_page(request)?;

        let mut png = Cursor::new(Vec::new());
        request
            .page_image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|error| OcrError::ProviderFailed {
                page: request.page_index,
                reason: error.to_string(),
            })?;

        let transcription = self.transport.transcribe(&TranscriptionRequest {
            model: &self.config.model,
            model_version: &self.config.model_version,
            prompt: &self.config.prompt,
            output_contract: self.config.output_contract,
            page_png: png.get_ref(),
            page_width: request.page_image.width(),
            page_height: request.page_image.height(),
            page_image_sha256: request.page_image_sha256,
            page_index: request.page_index,
            language_profile: request.language_profile,
            deadline: self.config.deadline.min(request.deadline),
            idempotency_key: request.idempotency_key,
        });
        let response = match transcription {
            Ok(response) => response,
            Err(error) => {
                let reason = error.to_fallback();
                return self.outcome_from_local(local, request, reason, DecisionSummary::default());
            }
        };
        let page_usage = ProviderUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            credits_charged: self.config.credits_per_page,
            requests: 1,
        };
        self.usage.add(page_usage);

        // Billing happens on a *received* response, keyed to the work, so a
        // retry of the same page against the same image cannot bill twice.
        if let (Some(backend), Some(reservation)) =
            (self.credits.as_mut(), self.reservation_id.as_ref())
        {
            backend
                .record_page(
                    reservation,
                    &PageCharge {
                        page_index: request.page_index,
                        idempotency_key: request.idempotency_key.to_owned(),
                        credits: self.config.credits_per_page,
                        input_tokens: response.input_tokens,
                        output_tokens: response.output_tokens,
                    },
                )
                .map_err(|error| map_credits_error(&error))?;
        }

        let raw_digest = super::hex(&Sha256::digest(response.body.as_bytes()));
        if response.body.trim().is_empty() {
            return self.outcome_from_local_after_response(
                local,
                request,
                FallbackReason::EmptyTranscription,
                DecisionSummary::default(),
                ResponseAudit {
                    raw_digest,
                    usage: page_usage,
                    model_version: response.model_version,
                },
            );
        }

        // The experimental structured path, when it has been explicitly
        // enabled for evaluation. Any failure falls through to the geometry
        // that was actually measured; it never fails the page.
        if self.config.output_contract == OutputContract::StructuredLines
            && self.config.structured_bbox == StructuredBboxPolicy::EvaluateWithFallback
        {
            match structured_bbox::parse_structured(&response.body, &self.config.gates) {
                Ok(parsed) => {
                    let (report, verdict) =
                        structured_bbox::evaluate_gates(&parsed, &local, &self.config.gates);
                    if verdict.is_ok() {
                        let page =
                            structured_bbox::to_ocr_page(&parsed, &local, &self.config.alignment);
                        let summary = DecisionSummary {
                            local_lines: report.local_lines,
                            provider_lines: report.provider_lines,
                            replaced: report.provider_lines,
                            coverage: report.local_match_coverage,
                            ..Default::default()
                        };
                        return Ok(self.finish(
                            page,
                            request,
                            PageVerdict {
                                geometry_source: GeometrySource::ProviderStructuredValidated,
                                fallback_reason: None,
                                summary,
                                raw_digest: Some(raw_digest),
                                usage: page_usage,
                                model_version: response.model_version,
                            },
                        ));
                    }
                    return self.outcome_from_local_after_response(
                        local,
                        request,
                        FallbackReason::GeometryGateFailed(error_text(&verdict)),
                        DecisionSummary::default(),
                        ResponseAudit {
                            raw_digest,
                            usage: page_usage,
                            model_version: response.model_version,
                        },
                    );
                }
                Err(error) => {
                    // A structured response that will not parse still contains
                    // the text; but guessing which part of a schema violation
                    // was the transcription is exactly the kind of recovery
                    // that produces confident nonsense, so the page falls back.
                    return self.outcome_from_local_after_response(
                        local,
                        request,
                        FallbackReason::ResponseInvalid(redaction::redact(&error.to_string())),
                        DecisionSummary::default(),
                        ResponseAudit {
                            raw_digest,
                            usage: page_usage,
                            model_version: response.model_version,
                        },
                    );
                }
            }
        }

        let aligned =
            alignment::align_transcription(&local, &response.body, &self.config.alignment);
        if !aligned.used_transcription {
            let reason = FallbackReason::AlignmentCoverageBelowGate {
                coverage: aligned.summary.coverage,
                required: self.config.alignment.min_coverage,
            };
            return self.outcome_from_local_after_response(
                local,
                request,
                reason,
                aligned.summary,
                ResponseAudit {
                    raw_digest,
                    usage: page_usage,
                    model_version: response.model_version,
                },
            );
        }
        Ok(self.finish(
            aligned.page,
            request,
            PageVerdict {
                geometry_source: GeometrySource::LocalLayoutAlignedText,
                fallback_reason: None,
                summary: aligned.summary,
                raw_digest: Some(raw_digest),
                usage: page_usage,
                model_version: response.model_version,
            },
        ))
    }

    fn cancel_job(&mut self, _ticket: &JobTicket) -> Result<(), OcrError> {
        if let (Some(backend), Some(reservation)) =
            (self.credits.as_mut(), self.reservation_id.as_ref())
        {
            backend
                .release(reservation, ReleaseReason::Cancelled)
                .map_err(|error| map_credits_error(&error))?;
        }
        Ok(())
    }

    fn finalize_job(&mut self, _ticket: &JobTicket) -> Result<JobSettlement, OcrError> {
        let Some(reservation) = self.reservation_id.clone() else {
            return Ok(JobSettlement::default());
        };
        let Some(backend) = self.credits.as_mut() else {
            return Ok(JobSettlement::default());
        };
        let settlement = backend
            .settle(&reservation)
            .map_err(|error| map_credits_error(&error))?;
        Ok(JobSettlement {
            charged_credits: settlement.charged_credits,
            refunded_credits: settlement.refunded_credits,
            released_credits: settlement.released_credits,
            settled: true,
        })
    }

    fn fingerprint_contribution(&self) -> String {
        self.fingerprint()
    }
}

fn error_text(verdict: &Result<(), structured_bbox::StructuredBboxError>) -> String {
    verdict
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_else(|| "unknown geometry gate failure".to_owned())
}

fn map_credits_error(error: &CreditsError) -> OcrError {
    match error {
        CreditsError::InsufficientCredits {
            available,
            required,
        } => OcrError::ProviderUnavailable(format!(
            "insufficient credits: {available} available, {required} required"
        )),
        other => OcrError::ProviderUnavailable(redaction::redact(&other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrBlock, OcrBox, OcrLine, OcrRoute, OcrRouteReason, OcrWord};
    use image::{DynamicImage, RgbImage};

    /// A stand-in for the local detector: it returns the rectangles a real
    /// detector would have measured for a known fixture page.
    struct FixtureLocal(Vec<String>);

    impl PageOcrProvider for FixtureLocal {
        fn recognize(
            &mut self,
            page_index: u32,
            _image: &DynamicImage,
            input_asset_sha256: &str,
        ) -> Result<OcrPage, OcrError> {
            let blocks = self
                .0
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let words: Vec<OcrWord> = text
                        .split_whitespace()
                        .enumerate()
                        .map(|(word_index, word)| OcrWord {
                            text: word.to_owned(),
                            normalized_text: word.to_owned(),
                            bbox: OcrBox {
                                x: 10.0 + word_index as f32 * 30.0,
                                y: 20.0 + index as f32 * 30.0,
                                width: 28.0,
                                height: 20.0,
                            },
                            confidence: 0.7,
                            reading_order: word_index as u32,
                        })
                        .collect();
                    let bbox = OcrBox {
                        x: 10.0,
                        y: 20.0 + index as f32 * 30.0,
                        width: 200.0,
                        height: 20.0,
                    };
                    OcrBlock {
                        bbox: bbox.clone(),
                        confidence: 0.7,
                        reading_order: index as u32,
                        lines: vec![OcrLine {
                            bbox,
                            confidence: 0.7,
                            reading_order: 0,
                            words,
                        }],
                    }
                })
                .collect();
            Ok(OcrPage {
                page_index,
                route: OcrRoute::Ocr {
                    reason: OcrRouteReason::MissingText,
                },
                width: 300,
                height: 400,
                blocks,
                revisions: Vec::new(),
                provider_provenance: Some(OcrProviderProvenance {
                    engine: "tesseract".into(),
                    model: "tessdata_best".into(),
                    version: "5".into(),
                    parameters: Default::default(),
                    input_asset_sha256: input_asset_sha256.to_owned(),
                    execution_location: "local".into(),
                    language_profile: Some("auto".into()),
                    model_digest: None,
                    model_license: None,
                    model_set: None,
                }),
                provider_raw_artifact: None,
            })
        }
    }

    fn image() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::new(300, 400))
    }

    fn provider(
        transport: ScriptedTransport,
        local: Vec<&str>,
        config: CloudOcrConfig,
    ) -> CloudOcrProvider {
        CloudOcrProvider::new(
            config,
            Box::new(transport),
            Box::new(FixtureLocal(local.into_iter().map(str::to_owned).collect())),
            "tesseract-5/tessdata_best-4.1.0",
        )
    }

    fn request<'a>(image: &'a DynamicImage, key: &'a str) -> PageOcrRequest<'a> {
        PageOcrRequest {
            document_sha256: "d".repeat(64).leak(),
            page_index: 0,
            page_image_sha256: "i".repeat(64).leak(),
            page_image: image,
            language_profile: "auto",
            dpi: 300,
            local_layout_version: "tesseract-5",
            requested_output_contract: OutputContract::TranscriptionOnly,
            deadline: Duration::from_secs(60),
            idempotency_key: key,
        }
    }

    #[test]
    fn a_successful_cloud_page_carries_local_geometry_and_cloud_text() {
        let mut provider = provider(
            ScriptedTransport::with_text("Einleitung\nKapitel I\n"),
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::gemini_byok("default"),
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(
            outcome.geometry_source,
            GeometrySource::LocalLayoutAlignedText
        );
        assert!(outcome.fallback_reason.is_none());
        assert_eq!(outcome.page.blocks[0].lines[0].words[0].text, "Einleitung");
        assert_eq!(outcome.page.blocks[0].lines[0].bbox.x, 10.0);
        assert_eq!(outcome.decisions.replaced, 2);
        // The evidence records who ran it and under which prompt.
        let parameters = &outcome
            .page
            .provider_provenance
            .as_ref()
            .unwrap()
            .parameters;
        assert_eq!(parameters["provider_mode"], "gemini-byok");
        assert_eq!(parameters["geometry_source"], "local_layout_aligned_text");
        assert!(parameters.contains_key("prompt_config_digest"));
    }

    #[test]
    fn production_prompt_budget_matches_the_measured_full_page_protocol() {
        let prompt = PromptConfig::default();
        assert_eq!(prompt.max_output_tokens, 32_768);
        let structured = prompt.instruction_for(OutputContract::StructuredLines, 300, 400);
        assert!(structured.contains(structured_bbox::STRUCTURED_BBOX_SCHEMA_VERSION));
        assert!(structured.contains("\"page_width\":300"));
        assert!(structured.contains("exactly one JSON object"));
        assert!(!structured.contains("No JSON"));
        assert_ne!(
            prompt.digest_for(OutputContract::TranscriptionOnly),
            prompt.digest_for(OutputContract::StructuredLines)
        );
    }

    #[test]
    fn provider_reported_model_version_reaches_page_provenance() {
        let response = TranscriptionResponse {
            body: "Einleitung\nKapitel I\n".into(),
            model_version: "provider-build-42".into(),
            input_tokens: 10,
            output_tokens: 4,
        };
        let transport = ScriptedTransport::with_text("unused").answer(0, Ok(response));
        let mut provider = provider(
            transport,
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::gemini_byok("default"),
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(outcome.model_version, "provider-build-42");
        let provenance = outcome.page.provider_provenance.unwrap();
        assert_eq!(provenance.parameters["model_version"], "provider-build-42");
        assert!(provenance
            .model_set
            .as_deref()
            .unwrap()
            .contains("provider-build-42"));
    }

    #[test]
    fn a_transport_failure_falls_back_to_a_complete_local_page_and_says_so() {
        let transport = ScriptedTransport::with_text("ignored")
            .answer(0, Err(TransportError::failed("503 upstream")));
        let mut provider = provider(
            transport,
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::gemini_byok("default"),
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(outcome.geometry_source, GeometrySource::LocalLayout);
        assert_eq!(
            outcome.fallback_reason.as_ref().unwrap().code(),
            "transport_failed"
        );
        // The page is still fully recognized: no empty text layer.
        assert_eq!(outcome.page.blocks.len(), 2);
        assert_eq!(provider.fallbacks().len(), 1);
    }

    #[test]
    fn fallback_fail_stops_the_run_instead_of_silently_using_local_text() {
        let transport =
            ScriptedTransport::with_text("x").answer(0, Err(TransportError::CredentialRejected));
        let mut provider = provider(
            transport,
            vec!["a b", "c d"],
            CloudOcrConfig {
                fallback: CloudFallback::Fail,
                ..CloudOcrConfig::gemini_byok("default")
            },
        );
        let image = image();
        assert!(provider.recognize_page(&request(&image, "key")).is_err());
    }

    #[test]
    fn a_cloud_failure_never_produces_an_empty_verified_text_layer() {
        for error in [
            TransportError::CredentialMissing,
            TransportError::CredentialRejected,
            TransportError::failed("timeout"),
            TransportError::invalid("not json"),
            TransportError::Cancelled,
        ] {
            let transport = ScriptedTransport::with_text("x").answer(0, Err(error));
            let mut provider = provider(
                transport,
                vec!["alpha beta", "gamma delta"],
                CloudOcrConfig::gemini_byok("default"),
            );
            let image = image();
            let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
            let words: usize = outcome
                .page
                .blocks
                .iter()
                .flat_map(|block| block.lines.iter())
                .map(|line| line.words.len())
                .sum();
            assert!(words > 0, "a fallback page must still carry local words");
        }
    }

    #[test]
    fn an_empty_transcription_is_a_fallback_not_a_blank_page() {
        let mut provider = provider(
            ScriptedTransport::with_text("   \n  \n"),
            vec!["alpha beta"],
            CloudOcrConfig::gemini_byok("default"),
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(
            outcome.fallback_reason.as_ref().unwrap().code(),
            "empty_transcription"
        );
        assert_eq!(outcome.page.blocks[0].lines[0].words.len(), 2);
    }

    #[test]
    fn the_raw_transcription_is_digested_but_never_stored_on_the_page() {
        let mut provider = provider(
            ScriptedTransport::with_text("Einleitung\nKapitel I\n"),
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::gemini_byok("default"),
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert!(outcome.raw_transcription_digest.is_some());
        assert_eq!(outcome.page.provider_raw_artifact, None);
    }

    #[test]
    fn byok_and_credits_fingerprints_differ_for_the_same_model_and_prompt() {
        let byok = CloudOcrConfig::gemini_byok("default");
        let credits = CloudOcrConfig::mpdf_credits(3);
        assert_eq!(byok.model, credits.model);
        assert_eq!(byok.prompt.digest(), credits.prompt.digest());
        assert_ne!(byok.fingerprint(), credits.fingerprint());
    }

    #[test]
    fn changing_the_prompt_or_the_alignment_changes_the_fingerprint() {
        let base = CloudOcrConfig::gemini_byok("default");
        let other_prompt = CloudOcrConfig {
            prompt: PromptConfig {
                instruction: format!("{} extra", base.prompt.instruction),
                ..base.prompt.clone()
            },
            ..base.clone()
        };
        let other_alignment = CloudOcrConfig {
            alignment: AlignmentConfig {
                min_coverage: 0.9,
                ..base.alignment.clone()
            },
            ..base.clone()
        };
        let other_model = CloudOcrConfig {
            model_version: "2026-09".into(),
            ..base.clone()
        };
        let fingerprints = [
            base.fingerprint(),
            other_prompt.fingerprint(),
            other_alignment.fingerprint(),
            other_model.fingerprint(),
        ];
        let unique: std::collections::HashSet<_> = fingerprints.iter().collect();
        assert_eq!(unique.len(), fingerprints.len());
    }

    #[test]
    fn no_fingerprint_contains_anything_derived_from_a_key() {
        // Rotating the key inside a slot must not change evidence identity,
        // and the slot name must be the only credential-ish thing present.
        let provider = provider(
            ScriptedTransport::with_text("x"),
            vec!["a"],
            CloudOcrConfig::gemini_byok("my-slot"),
        );
        let fingerprint = provider.fingerprint_contribution();
        assert!(fingerprint.contains("credential_slot=my-slot"));
        assert!(!fingerprint.to_lowercase().contains("authorization"));
        assert!(!fingerprint.contains("AIza"));
    }

    #[test]
    fn a_credits_run_reserves_before_any_page_and_settles_what_it_used() {
        let backend = credits::FakeCreditsBackend::new(1_000, b"secret".to_vec());
        let mut provider = provider(
            ScriptedTransport::with_text("Einleitung\nKapitel I\n"),
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::mpdf_credits(5),
        )
        .with_credits(Box::new(backend));
        let ticket = provider
            .prepare_job(&JobPreparation {
                job_id: "job".into(),
                document_sha256: "d".repeat(64),
                page_indices: vec![0, 1, 2, 3],
                language_profile: "auto".into(),
                dpi: 300,
                max_credits: 100,
            })
            .unwrap();
        assert_eq!(ticket.reserved_credits, 20);
        let image = image();
        provider
            .recognize_page(&request(&image, "page-key-0"))
            .unwrap();
        let settlement = provider.finalize_job(&ticket).unwrap();
        assert_eq!(settlement.charged_credits, 5);
        assert_eq!(settlement.released_credits, 15);
    }

    #[test]
    fn a_retried_page_in_a_credits_run_is_charged_once() {
        let backend = credits::FakeCreditsBackend::new(1_000, b"secret".to_vec());
        let mut provider = provider(
            ScriptedTransport::with_text("Einleitung\nKapitel I\n"),
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::mpdf_credits(5),
        )
        .with_credits(Box::new(backend));
        let ticket = provider
            .prepare_job(&JobPreparation {
                job_id: "job".into(),
                document_sha256: "d".repeat(64),
                page_indices: vec![0, 1],
                language_profile: "auto".into(),
                dpi: 300,
                max_credits: 100,
            })
            .unwrap();
        let image = image();
        let key = credits::page_idempotency_key(&"d".repeat(64), 0, &"i".repeat(64), "cfg");
        for _ in 0..4 {
            provider.recognize_page(&request(&image, &key)).unwrap();
        }
        assert_eq!(provider.finalize_job(&ticket).unwrap().charged_credits, 5);
    }

    #[test]
    fn a_job_over_the_authorized_ceiling_never_reserves() {
        let backend = credits::FakeCreditsBackend::new(1_000, b"secret".to_vec());
        let mut provider = provider(
            ScriptedTransport::with_text("x"),
            vec!["a"],
            CloudOcrConfig::mpdf_credits(10),
        )
        .with_credits(Box::new(backend));
        let error = provider
            .prepare_job(&JobPreparation {
                job_id: "job".into(),
                document_sha256: "d".repeat(64),
                page_indices: (0..100).collect(),
                language_profile: "auto".into(),
                dpi: 300,
                max_credits: 50,
            })
            .unwrap_err();
        assert!(error.to_string().contains("authorized"));
    }

    #[test]
    fn cancelling_a_credits_run_releases_the_unused_hold() {
        let backend = credits::FakeCreditsBackend::new(1_000, b"secret".to_vec());
        let mut provider = provider(
            ScriptedTransport::with_text("Einleitung\nKapitel I\n"),
            vec!["Einleitng", "Kapitel l"],
            CloudOcrConfig::mpdf_credits(5),
        )
        .with_credits(Box::new(backend));
        let ticket = provider
            .prepare_job(&JobPreparation {
                job_id: "job".into(),
                document_sha256: "d".repeat(64),
                page_indices: vec![0, 1, 2, 3],
                language_profile: "auto".into(),
                dpi: 300,
                max_credits: 100,
            })
            .unwrap();
        provider.cancel_job(&ticket).unwrap();
        assert!(provider.finalize_job(&ticket).is_ok());
    }

    #[test]
    fn credits_are_not_production_ready_and_the_capability_says_so() {
        let provider = provider(
            ScriptedTransport::with_text("x"),
            vec!["a"],
            CloudOcrConfig::mpdf_credits(5),
        );
        let capabilities = provider.capabilities();
        assert!(!capabilities.production_ready);
        assert!(capabilities
            .not_production_ready_reason
            .as_ref()
            .unwrap()
            .contains("payment"));
        assert!(!capabilities.structured_bbox_default_enabled);
    }

    #[test]
    fn a_missing_credential_is_a_configuration_error_before_anything_runs() {
        let transport = ScriptedTransport {
            credential: false,
            ..ScriptedTransport::with_text("x")
        };
        let provider = provider(transport, vec!["a"], CloudOcrConfig::gemini_byok("slot"));
        assert!(matches!(
            provider.validate_configuration(),
            Err(ProviderConfigError::CredentialMissing { .. })
        ));
    }

    #[test]
    fn requesting_structured_lines_while_the_policy_is_disabled_is_refused() {
        let provider = provider(
            ScriptedTransport::with_text("x"),
            vec!["a"],
            CloudOcrConfig {
                output_contract: OutputContract::StructuredLines,
                ..CloudOcrConfig::gemini_byok("slot")
            },
        );
        assert!(matches!(
            provider.validate_configuration(),
            Err(ProviderConfigError::Invalid(_))
        ));
    }

    #[test]
    fn an_invalid_structured_response_falls_back_rather_than_guessing() {
        let mut provider = provider(
            ScriptedTransport::with_text("{ not json at all"),
            vec!["alpha beta", "gamma delta"],
            CloudOcrConfig {
                output_contract: OutputContract::StructuredLines,
                structured_bbox: StructuredBboxPolicy::EvaluateWithFallback,
                ..CloudOcrConfig::gemini_byok("slot")
            },
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(outcome.geometry_source, GeometrySource::LocalLayout);
        assert_eq!(
            outcome.fallback_reason.as_ref().unwrap().code(),
            "response_invalid"
        );
        assert_eq!(outcome.usage.requests, 1);
        assert!(outcome.raw_transcription_digest.is_some());
    }

    #[test]
    fn a_valid_structured_response_that_fails_geometry_never_becomes_text() {
        let body = serde_json::json!({
            "schema_version": structured_bbox::STRUCTURED_BBOX_SCHEMA_VERSION,
            "page_width": 300.0,
            "page_height": 400.0,
            "lines": [{
                "text": "invented provider text",
                "bbox": [-10.0, 20.0, 200.0, 20.0],
                "reading_order": 0
            }]
        })
        .to_string();
        let mut provider = provider(
            ScriptedTransport::with_text(body),
            vec!["alpha beta"],
            CloudOcrConfig {
                output_contract: OutputContract::StructuredLines,
                structured_bbox: StructuredBboxPolicy::EvaluateWithFallback,
                ..CloudOcrConfig::gemini_byok("slot")
            },
        );
        let image = image();
        let outcome = provider.recognize_page(&request(&image, "key")).unwrap();
        assert_eq!(outcome.geometry_source, GeometrySource::LocalLayout);
        assert_eq!(
            outcome.fallback_reason.as_ref().unwrap().code(),
            "geometry_gate_failed"
        );
        assert_eq!(outcome.page.blocks[0].lines[0].words[0].text, "alpha");
        assert_eq!(outcome.usage.requests, 1);
        assert!(outcome.raw_transcription_digest.is_some());
    }
}
