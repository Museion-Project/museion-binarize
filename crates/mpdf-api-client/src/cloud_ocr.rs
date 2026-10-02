//! HTTPS transports and provider factories for the two cloud OCR modes.
//!
//! Everything that decides *what OCR means* — the prompt, the alignment, the
//! geometry gates, the fallback rules, the credits state machine — lives in
//! `mpdf_core::ocr_provider`. This module is the part that cannot: sockets,
//! TLS, retries, and reading a secret out of the OS credential store.
//!
//! # The two secrecy rules this file exists to keep
//!
//! 1. **A key is read as late as possible and never travels.** The BYOK
//!    transport fetches the secret from the credential store immediately
//!    before a request, puts it in a header, and drops it. It is never a
//!    field on a struct that derives `Debug` or `Serialize`, never a command
//!    line argument, never part of a URL, and never in a trace.
//! 2. **A provider's own words are never repeated.** Google's error bodies
//!    echo request content and sometimes the key itself. Every failure path
//!    here constructs its message through `TransportError::failed`, which
//!    redacts, so no caller can print a raw body even by accident.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use mpdf_core::error::{CoreError, Result as CoreResult};
use mpdf_core::ocr::PageOcrProvider;
use mpdf_core::ocr_provider::credits::{
    CreditsBackend, CreditsBalance, CreditsError, PageCharge, ReleaseReason, Reservation,
    ReservationRequest, Settlement,
};
use mpdf_core::ocr_provider::gemini::{
    CloudOcrConfig, CloudOcrProvider, PageTranscriptionTransport, TranscriptionRequest,
    TranscriptionResponse, TransportError,
};
use mpdf_core::ocr_provider::redaction::{redact, MaskedCredential};
use mpdf_core::ocr_provider::{
    CloudFallback, ConnectionTest, JobTicket, OcrProvider, OcrProviderMode,
};
use mpdf_core::orchestrator::CloudProviderFactory;
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{ApiClientError, Result, Secret, SecretStore};

/// Keychain service for user-supplied model-provider keys.
///
/// Deliberately not [`crate::CREDENTIAL_SERVICE`]: an M PDF task token and a
/// user's Google key have different lifetimes, different blast radii, and
/// different deletion semantics, and putting them in one service makes
/// "delete my key" ambiguous.
pub const GEMINI_CREDENTIAL_SERVICE: &str = "org.mpdf.model-provider";

/// The only endpoint this build talks to for BYOK.
pub const GEMINI_ENDPOINT: &str = "https://generativelanguage.googleapis.com";

pub const MAX_GEMINI_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
/// A 300-dpi page PNG. Larger than this is not a page.
pub const MAX_PAGE_IMAGE_BYTES: usize = 24 * 1024 * 1024;

/// A credential store scoped to [`GEMINI_CREDENTIAL_SERVICE`].
#[derive(Debug, Default)]
pub struct ModelProviderSecretStore;

impl ModelProviderSecretStore {
    fn entry(slot: &str) -> Result<keyring::Entry> {
        if slot.is_empty()
            || slot.len() > 256
            || !slot
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(ApiClientError::Credential("invalid credential slot".into()));
        }
        keyring::Entry::new(GEMINI_CREDENTIAL_SERVICE, slot)
            .map_err(|_| ApiClientError::CredentialStoreUnavailable)
    }

    /// The only thing a UI is ever told about a stored key.
    pub fn describe(&self, slot: &str) -> MaskedCredential {
        MaskedCredential::new(slot, self.get(slot).ok().flatten().is_some())
    }
}

impl SecretStore for ModelProviderSecretStore {
    fn get(&self, slot: &str) -> Result<Option<Secret>> {
        match Self::entry(slot)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ApiClientError::CredentialStoreUnavailable),
        }
    }

    fn set(&self, slot: &str, secret: Secret) -> Result<()> {
        Self::entry(slot)?
            .set_password(secret.as_str())
            .map_err(|_| ApiClientError::CredentialStoreUnavailable)
    }

    fn delete(&self, slot: &str) -> Result<()> {
        match Self::entry(slot)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ApiClientError::CredentialStoreUnavailable),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CloudTransportPolicy {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_retries: u32,
    pub max_response_bytes: u64,
    /// Loopback HTTP for a local mock server. Never true in a shipped build's
    /// default configuration.
    pub allow_loopback_http: bool,
}

impl Default for CloudTransportPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            total_timeout: Duration::from_secs(180),
            max_retries: 3,
            max_response_bytes: MAX_GEMINI_RESPONSE_BYTES,
            allow_loopback_http: false,
        }
    }
}

fn build_client(policy: &CloudTransportPolicy) -> Result<Client> {
    Client::builder()
        // A redirect is how a credential-bearing request ends up at a host
        // the user never authorized.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(policy.connect_timeout)
        .timeout(policy.total_timeout)
        .build()
        .map_err(|error| ApiClientError::Transport(redact(&error.to_string())))
}

fn bounded_body(response: Response, limit: u64) -> std::result::Result<Vec<u8>, TransportError> {
    use std::io::Read as _;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(TransportError::invalid("response exceeds the size limit"));
    }
    let mut out = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(|_| TransportError::failed("response read failed"))?;
    if out.len() as u64 > limit {
        return Err(TransportError::invalid("response exceeds the size limit"));
    }
    Ok(out)
}

fn retryable(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn completed_finish_reason(reason: Option<&str>) -> bool {
    reason == Some("STOP")
}

fn retry_delay(response: &Response, attempt: u32) -> Duration {
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(30)))
        // Deterministic backoff, no jitter: a resumable per-page job does not
        // need decorrelation, and a fixed schedule is testable.
        .unwrap_or_else(|| Duration::from_millis(250 * u64::from(attempt + 1)))
}

fn sleep_cancellable(
    cancelled: &AtomicBool,
    delay: Duration,
) -> std::result::Result<(), TransportError> {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransportError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

// ---------------------------------------------------------------- Gemini BYOK

/// Talks to Google's Generative Language API with the user's own key.
pub struct GeminiTransport<S: SecretStore> {
    endpoint: String,
    slot: String,
    secrets: Arc<S>,
    client: Client,
    policy: CloudTransportPolicy,
    cancelled: Arc<AtomicBool>,
}

impl<S: SecretStore> GeminiTransport<S> {
    pub fn new(
        endpoint: impl Into<String>,
        slot: impl Into<String>,
        secrets: Arc<S>,
        policy: CloudTransportPolicy,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        let url = reqwest::Url::parse(&endpoint)
            .map_err(|_| ApiClientError::InvalidEndpoint("malformed URL".into()))?;
        if url.username() != "" || url.password().is_some() || url.query().is_some() {
            return Err(ApiClientError::InvalidEndpoint(
                "credentials and query strings are not allowed in an endpoint".into(),
            ));
        }
        let loopback_ok = policy.allow_loopback_http
            && matches!(
                url.host_str(),
                Some("127.0.0.1") | Some("localhost") | Some("[::1]")
            );
        if url.scheme() != "https" && !loopback_ok {
            return Err(ApiClientError::InvalidEndpoint(
                "HTTPS is required; loopback HTTP needs explicit development mode".into(),
            ));
        }
        if !loopback_ok
            && (url.scheme() != "https"
                || url.host_str() != Some("generativelanguage.googleapis.com")
                || url.port().is_some()
                || url.path() != "/"
                || url.fragment().is_some())
        {
            return Err(ApiClientError::InvalidEndpoint(
                "Gemini BYOK is restricted to the Google Generative Language API origin".into(),
            ));
        }
        Ok(Self {
            client: build_client(&policy)?,
            endpoint,
            slot: slot.into(),
            secrets,
            policy,
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancelled = cancelled;
        self
    }

    /// Reads the key for the duration of one request and no longer.
    fn secret(&self) -> std::result::Result<Secret, TransportError> {
        self.secrets
            .get(&self.slot)
            .map_err(|_| TransportError::failed("credential store unavailable"))?
            .ok_or(TransportError::CredentialMissing)
    }

    fn model_url(&self, model: &str, action: &str) -> std::result::Result<String, TransportError> {
        if !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        {
            return Err(TransportError::invalid("model name is not well formed"));
        }
        Ok(format!(
            "{}/v1beta/models/{model}{action}",
            self.endpoint.trim_end_matches('/')
        ))
    }
}

#[derive(Serialize)]
struct GeminiRequest<'a> {
    contents: Vec<GeminiContent<'a>>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiContent<'a> {
    role: &'static str,
    parts: Vec<GeminiPart<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GeminiPart<'a> {
    Text {
        text: &'a str,
    },
    Inline {
        #[serde(rename = "inline_data")]
        inline_data: GeminiInlineData,
    },
}

#[derive(Serialize)]
struct GeminiInlineData {
    mime_type: &'static str,
    data: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    #[serde(rename = "candidateCount")]
    candidate_count: u32,
    #[serde(rename = "thinkingConfig")]
    thinking_config: GeminiThinkingConfig,
}

#[derive(Serialize)]
struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget")]
    thinking_budget: u32,
}

#[derive(Deserialize)]
struct GeminiResponseWire {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata", default)]
    usage: Option<GeminiUsage>,
    #[serde(rename = "modelVersion", default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiCandidateContent>,
    #[serde(rename = "finishReason", default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiCandidateContent {
    #[serde(default)]
    parts: Vec<GeminiTextPart>,
}

#[derive(Deserialize)]
struct GeminiTextPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    prompt_tokens: u64,
    #[serde(rename = "candidatesTokenCount", default)]
    candidate_tokens: u64,
}

impl<S: SecretStore> PageTranscriptionTransport for GeminiTransport<S> {
    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> std::result::Result<TranscriptionResponse, TransportError> {
        if request.page_png.len() > MAX_PAGE_IMAGE_BYTES {
            return Err(TransportError::invalid(
                "page image exceeds the upload limit",
            ));
        }
        let url = self.model_url(request.model, ":generateContent")?;
        let instruction = request.prompt.instruction_for(
            request.output_contract,
            request.page_width,
            request.page_height,
        );
        let body = GeminiRequest {
            contents: vec![GeminiContent {
                role: "user",
                parts: vec![
                    GeminiPart::Text { text: &instruction },
                    GeminiPart::Inline {
                        inline_data: GeminiInlineData {
                            mime_type: "image/png",
                            data: base64::engine::general_purpose::STANDARD
                                .encode(request.page_png),
                        },
                    },
                ],
            }],
            generation_config: GeminiGenerationConfig {
                temperature: f32::from(request.prompt.temperature_milli) / 1000.0,
                max_output_tokens: request.prompt.max_output_tokens,
                candidate_count: 1,
                thinking_config: GeminiThinkingConfig { thinking_budget: 0 },
            },
        };
        let payload = serde_json::to_vec(&body)
            .map_err(|_| TransportError::invalid("could not encode the request"))?;

        let mut attempt = 0;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(TransportError::Cancelled);
            }
            let secret = self.secret()?;
            let response = self
                .client
                .post(&url)
                // The key goes in a header, never in the URL: a query string
                // is logged by proxies, stored in histories, and shows up in
                // referrers.
                .header("x-goog-api-key", secret.as_str())
                .header("content-type", "application/json")
                .timeout(request.deadline)
                .body(payload.clone())
                .send();
            drop(secret);

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    // A connect error happened before an HTTP exchange was
                    // established and is safe to retry. A timeout or other
                    // transport error is outcome-ambiguous: Google does not
                    // document an idempotency contract for generateContent,
                    // so silently retrying it could duplicate paid work.
                    if error.is_connect() && attempt < self.policy.max_retries {
                        attempt += 1;
                        sleep_cancellable(
                            &self.cancelled,
                            Duration::from_millis(250 * u64::from(attempt)),
                        )?;
                        continue;
                    }
                    return if error.is_connect() {
                        Err(TransportError::failed(error.to_string()))
                    } else {
                        Err(TransportError::outcome_unknown(error.to_string()))
                    };
                }
            };
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                // The body of a 401/403 from this provider frequently quotes
                // the key back. It is dropped unread.
                return Err(TransportError::CredentialRejected);
            }
            if retryable(status) && attempt < self.policy.max_retries {
                let delay = retry_delay(&response, attempt);
                attempt += 1;
                sleep_cancellable(&self.cancelled, delay)?;
                continue;
            }
            if !status.is_success() {
                return Err(TransportError::failed(format!(
                    "provider returned HTTP {}",
                    status.as_u16()
                )));
            }
            let bytes = bounded_body(response, self.policy.max_response_bytes)?;
            let wire: GeminiResponseWire = serde_json::from_slice(&bytes)
                .map_err(|error| TransportError::invalid(error.to_string()))?;
            let candidate = wire
                .candidates
                .first()
                .ok_or_else(|| TransportError::invalid("no candidate was returned"))?;
            if !completed_finish_reason(candidate.finish_reason.as_deref()) {
                // A safety-blocked or recitation-blocked page is a refusal,
                // not a transcription. MAX_TOKENS is also a refusal: applying
                // a prefix of a dense page would produce mixed evidence while
                // presenting the cloud result as complete.
                return Err(TransportError::invalid(
                    "the model did not complete the page",
                ));
            }
            let text = candidate
                .content
                .as_ref()
                .map(|content| {
                    content
                        .parts
                        .iter()
                        .filter_map(|part| part.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            let usage = wire.usage.unwrap_or_default();
            return Ok(TranscriptionResponse {
                body: text,
                model_version: wire
                    .model_version
                    .unwrap_or_else(|| request.model_version.to_owned()),
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.candidate_tokens,
            });
        }
    }

    fn probe(&mut self) -> std::result::Result<bool, TransportError> {
        // A metadata GET: it costs nothing and bills nothing, which is what
        // makes it safe to run from a settings screen.
        let url = self.model_url(mpdf_core::ocr_provider::gemini::DEFAULT_MODEL, "")?;
        let secret = self.secret()?;
        let response = self
            .client
            .get(&url)
            .header("x-goog-api-key", secret.as_str())
            .send()
            .map_err(|error| TransportError::failed(error.to_string()))?;
        drop(secret);
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(TransportError::CredentialRejected)
            }
            status if status.is_success() => Ok(true),
            status => Err(TransportError::failed(format!(
                "provider returned HTTP {}",
                status.as_u16()
            ))),
        }
    }

    fn credential_present(&self) -> bool {
        self.secrets.get(&self.slot).ok().flatten().is_some()
    }
}

// ------------------------------------------------------------- M PDF Credits

/// Talks to an M PDF Credits service, which holds the model credential and
/// executes on the user's behalf.
///
/// The client authenticates with a short-lived, job-scoped token. It never
/// receives, and has no protocol field capable of carrying, the platform's
/// own model key.
pub struct BrokeredTransport<S: SecretStore> {
    endpoint: String,
    slot: String,
    secrets: Arc<S>,
    client: Client,
    policy: CloudTransportPolicy,
    cancelled: Arc<AtomicBool>,
    reservation_id: String,
}

#[derive(Serialize)]
struct BrokeredPageRequest<'a> {
    protocol: &'static str,
    protocol_version: &'static str,
    reservation_id: &'a str,
    page_index: u32,
    page_image_sha256: &'a str,
    page_image_base64: String,
    language_profile: &'a str,
    prompt_digest: String,
    output_contract: &'static str,
    idempotency_key: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokeredPageResponse {
    protocol: String,
    protocol_version: String,
    text: String,
    model: String,
    model_version: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    credits_charged: u64,
}

impl<S: SecretStore> BrokeredTransport<S> {
    pub fn new(
        endpoint: impl Into<String>,
        slot: impl Into<String>,
        secrets: Arc<S>,
        policy: CloudTransportPolicy,
        reservation_id: impl Into<String>,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        if !mpdf_core::remote_api::validate_origin(&endpoint) && !policy.allow_loopback_http {
            return Err(ApiClientError::InvalidEndpoint(
                "endpoint must be a strict HTTPS origin".into(),
            ));
        }
        Ok(Self {
            client: build_client(&policy)?,
            endpoint,
            slot: slot.into(),
            secrets,
            policy,
            cancelled: Arc::new(AtomicBool::new(false)),
            reservation_id: reservation_id.into(),
        })
    }

    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancelled = cancelled;
        self
    }
}

impl<S: SecretStore> PageTranscriptionTransport for BrokeredTransport<S> {
    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> std::result::Result<TranscriptionResponse, TransportError> {
        if request.page_png.len() > MAX_PAGE_IMAGE_BYTES {
            return Err(TransportError::invalid(
                "page image exceeds the upload limit",
            ));
        }
        let token = self
            .secrets
            .get(&self.slot)
            .map_err(|_| TransportError::failed("credential store unavailable"))?
            .ok_or(TransportError::CredentialMissing)?;
        let url = format!("{}/v1/ocr/pages", self.endpoint.trim_end_matches('/'));
        let body = BrokeredPageRequest {
            protocol: mpdf_core::ocr_provider::credits::CREDITS_PROTOCOL,
            protocol_version: mpdf_core::ocr_provider::credits::CREDITS_PROTOCOL_VERSION,
            reservation_id: &self.reservation_id,
            page_index: request.page_index,
            page_image_sha256: request.page_image_sha256,
            page_image_base64: base64::engine::general_purpose::STANDARD.encode(request.page_png),
            language_profile: request.language_profile,
            prompt_digest: request.prompt.digest_for(request.output_contract),
            output_contract: request.output_contract.as_str(),
            idempotency_key: request.idempotency_key,
        };
        let mut attempt = 0;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(TransportError::Cancelled);
            }
            let response = self
                .client
                .post(&url)
                .bearer_auth(token.as_str())
                .header("x-mpdf-idempotency-key", request.idempotency_key)
                .timeout(request.deadline)
                .json(&body)
                .send();
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    if attempt < self.policy.max_retries {
                        attempt += 1;
                        sleep_cancellable(
                            &self.cancelled,
                            Duration::from_millis(250 * u64::from(attempt)),
                        )?;
                        continue;
                    }
                    return Err(TransportError::failed(error.to_string()));
                }
            };
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                return Err(TransportError::CredentialRejected);
            }
            if status == StatusCode::PAYMENT_REQUIRED {
                return Err(TransportError::failed(
                    "the reservation has no credits left",
                ));
            }
            if retryable(status) && attempt < self.policy.max_retries {
                let delay = retry_delay(&response, attempt);
                attempt += 1;
                sleep_cancellable(&self.cancelled, delay)?;
                continue;
            }
            if !status.is_success() {
                return Err(TransportError::failed(format!(
                    "service returned HTTP {}",
                    status.as_u16()
                )));
            }
            let bytes = bounded_body(response, self.policy.max_response_bytes)?;
            let wire: BrokeredPageResponse = serde_json::from_slice(&bytes)
                .map_err(|error| TransportError::invalid(error.to_string()))?;
            if wire.protocol != mpdf_core::ocr_provider::credits::CREDITS_PROTOCOL
                || wire.protocol_version
                    != mpdf_core::ocr_provider::credits::CREDITS_PROTOCOL_VERSION
            {
                return Err(TransportError::invalid("unsupported credits protocol"));
            }
            let _ = (wire.model, wire.credits_charged);
            return Ok(TranscriptionResponse {
                body: wire.text,
                model_version: wire.model_version,
                input_tokens: wire.input_tokens,
                output_tokens: wire.output_tokens,
            });
        }
    }

    fn probe(&mut self) -> std::result::Result<bool, TransportError> {
        Ok(self.credential_present())
    }

    fn credential_present(&self) -> bool {
        self.secrets.get(&self.slot).ok().flatten().is_some()
    }

    fn bind_job(&mut self, ticket: &JobTicket) {
        // Every subsequent page request quotes this reservation, so the
        // service can refuse work the hold does not cover.
        self.reservation_id = ticket.reservation_id.clone().unwrap_or_default();
    }
}

/// HTTP implementation of the credits ledger.
///
/// Every method maps one-to-one onto a state transition in
/// `mpdf_core::ocr_provider::credits`; the rules stay in the core and this
/// only moves them across a wire.
pub struct HttpCreditsBackend<S: SecretStore> {
    endpoint: String,
    slot: String,
    secrets: Arc<S>,
    client: Client,
    policy: CloudTransportPolicy,
    /// Shared secret the settlement signature is verified against. In a real
    /// deployment this is a published verification key; there is no such
    /// deployment, which is one of the release blockers.
    verification_secret: Vec<u8>,
}

impl<S: SecretStore> HttpCreditsBackend<S> {
    pub fn new(
        endpoint: impl Into<String>,
        slot: impl Into<String>,
        secrets: Arc<S>,
        policy: CloudTransportPolicy,
        verification_secret: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        Ok(Self {
            client: build_client(&policy)?,
            endpoint: endpoint.into(),
            slot: slot.into(),
            secrets,
            policy,
            verification_secret: verification_secret.into(),
        })
    }

    fn post<B: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        suffix: &str,
        body: &B,
    ) -> std::result::Result<R, CreditsError> {
        let token = self
            .secrets
            .get(&self.slot)
            .map_err(|_| CreditsError::Unavailable("credential store unavailable".into()))?
            .ok_or_else(|| CreditsError::Unavailable("no session token is stored".into()))?;
        let response = self
            .client
            .post(format!("{}{suffix}", self.endpoint.trim_end_matches('/')))
            .bearer_auth(token.as_str())
            .json(body)
            .send()
            .map_err(|error| CreditsError::Unavailable(redact(&error.to_string())))?;
        let status = response.status();
        if status == StatusCode::PAYMENT_REQUIRED {
            return Err(CreditsError::InsufficientCredits {
                available: 0,
                required: 0,
            });
        }
        if !status.is_success() {
            return Err(CreditsError::Unavailable(format!(
                "service returned HTTP {}",
                status.as_u16()
            )));
        }
        let bytes = bounded_body(response, self.policy.max_response_bytes)
            .map_err(|error| CreditsError::Unavailable(error.to_string()))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| CreditsError::Unavailable(redact(&error.to_string())))
    }

    fn checked(&self, settlement: Settlement) -> std::result::Result<Settlement, CreditsError> {
        // A statement that decides what the user was charged is not believed
        // until its signature matches.
        settlement.verify(&self.verification_secret)?;
        Ok(settlement)
    }
}

#[derive(Serialize)]
struct ReservationIdBody<'a> {
    reservation_id: &'a str,
}

#[derive(Serialize)]
struct ReleaseBody<'a> {
    reservation_id: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct RecordPageBody<'a> {
    reservation_id: &'a str,
    charge: &'a PageCharge,
}

impl<S: SecretStore> CreditsBackend for HttpCreditsBackend<S> {
    fn balance(&self) -> std::result::Result<CreditsBalance, CreditsError> {
        self.post("/v1/credits/balance", &())
    }

    fn reserve(
        &mut self,
        request: &ReservationRequest,
    ) -> std::result::Result<Reservation, CreditsError> {
        request.validate()?;
        self.post("/v1/credits/reservations", request)
    }

    fn record_page(
        &mut self,
        reservation_id: &str,
        charge: &PageCharge,
    ) -> std::result::Result<(), CreditsError> {
        let _: serde_json::Value = self.post(
            "/v1/credits/pages",
            &RecordPageBody {
                reservation_id,
                charge,
            },
        )?;
        Ok(())
    }

    fn settle(&mut self, reservation_id: &str) -> std::result::Result<Settlement, CreditsError> {
        let settlement: Settlement =
            self.post("/v1/credits/settle", &ReservationIdBody { reservation_id })?;
        self.checked(settlement)
    }

    fn release(
        &mut self,
        reservation_id: &str,
        reason: ReleaseReason,
    ) -> std::result::Result<Settlement, CreditsError> {
        let settlement: Settlement = self.post(
            "/v1/credits/release",
            &ReleaseBody {
                reservation_id,
                reason: match reason {
                    ReleaseReason::Cancelled => "cancelled",
                    ReleaseReason::Failed => "failed",
                },
            },
        )?;
        self.checked(settlement)
    }

    fn refund(&mut self, reservation_id: &str) -> std::result::Result<Settlement, CreditsError> {
        let settlement: Settlement =
            self.post("/v1/credits/refund", &ReservationIdBody { reservation_id })?;
        self.checked(settlement)
    }
}

// ------------------------------------------------------------------ factories

/// Builds a BYOK provider for one run.
pub struct GeminiByokFactory {
    config: CloudOcrConfig,
    endpoint: String,
    policy: CloudTransportPolicy,
    secrets: Arc<ModelProviderSecretStore>,
    local_layout_version: String,
    cancelled: Arc<AtomicBool>,
}

impl GeminiByokFactory {
    pub fn new(
        config: CloudOcrConfig,
        endpoint: impl Into<String>,
        policy: CloudTransportPolicy,
        local_layout_version: impl Into<String>,
    ) -> Self {
        Self {
            config,
            endpoint: endpoint.into(),
            policy,
            secrets: Arc::new(ModelProviderSecretStore),
            local_layout_version: local_layout_version.into(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancelled = cancelled;
        self
    }

    pub fn credential(&self) -> MaskedCredential {
        self.secrets.describe(&self.config.credential_slot)
    }

    /// Non-billable check for a settings screen. Returns provider, model
    /// availability and a masked credential — and nothing else, whatever the
    /// provider said.
    pub fn connection_test(&self) -> ConnectionTest {
        let mut transport = match GeminiTransport::new(
            &self.endpoint,
            &self.config.credential_slot,
            self.secrets.clone(),
            self.policy.clone(),
        ) {
            Ok(transport) => transport,
            Err(error) => {
                return ConnectionTest {
                    mode: OcrProviderMode::GeminiByok,
                    provider_name: self.config.provider_name.clone(),
                    model: self.config.model.clone(),
                    model_available: false,
                    credential: self.credential(),
                    diagnostic: redact(&error.to_string()),
                }
            }
        };
        let (available, diagnostic) = match transport.probe() {
            Ok(available) => (available, "the model answered".to_owned()),
            Err(error) => (false, error.to_string()),
        };
        ConnectionTest {
            mode: OcrProviderMode::GeminiByok,
            provider_name: self.config.provider_name.clone(),
            model: self.config.model.clone(),
            model_available: available,
            credential: self.credential(),
            diagnostic: redact(&diagnostic),
        }
    }
}

impl CloudProviderFactory for GeminiByokFactory {
    fn mode(&self) -> OcrProviderMode {
        OcrProviderMode::GeminiByok
    }

    fn config_digest(&self) -> String {
        self.config.digest()
    }

    fn fingerprint_contribution(&self) -> String {
        format!(
            "{}|layout={}|credential_slot={}|credits_per_page=0|endpoint={}",
            self.config.fingerprint(),
            self.local_layout_version,
            self.config.credential_slot,
            self.endpoint
        )
    }

    fn max_credits(&self) -> u64 {
        0
    }

    fn fallback(&self) -> CloudFallback {
        self.config.fallback
    }

    fn build(&self, local: Box<dyn PageOcrProvider>) -> CoreResult<Box<dyn OcrProvider>> {
        let transport = GeminiTransport::new(
            &self.endpoint,
            &self.config.credential_slot,
            self.secrets.clone(),
            self.policy.clone(),
        )
        .map(|transport| transport.with_cancellation(self.cancelled.clone()))
        .map_err(|error| CoreError::InvalidParameter(redact(&error.to_string())))?;
        Ok(Box::new(CloudOcrProvider::new(
            self.config.clone(),
            Box::new(transport),
            local,
            self.local_layout_version.clone(),
        )))
    }
}

/// Builds an M PDF Credits provider for one run.
///
/// Constructing one is possible; using it against a production service is
/// not, because there is none. The capability this produces reports
/// `production_ready = false`, and both front ends print the blockers.
pub struct MpdfCreditsFactory {
    config: CloudOcrConfig,
    endpoint: String,
    policy: CloudTransportPolicy,
    secrets: Arc<crate::RuntimeSecretStore>,
    local_layout_version: String,
    max_credits: u64,
    verification_secret: Vec<u8>,
    reservation_hint: String,
    cancelled: Arc<AtomicBool>,
}

impl MpdfCreditsFactory {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: CloudOcrConfig,
        endpoint: impl Into<String>,
        policy: CloudTransportPolicy,
        local_layout_version: impl Into<String>,
        max_credits: u64,
        verification_secret: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            config,
            endpoint: endpoint.into(),
            policy,
            secrets: Arc::new(crate::RuntimeSecretStore::default()),
            local_layout_version: local_layout_version.into(),
            max_credits,
            verification_secret: verification_secret.into(),
            reservation_hint: String::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancelled = cancelled;
        self
    }

    pub fn release_blockers(&self) -> Vec<&'static str> {
        mpdf_core::ocr_provider::credits::release_blockers()
    }
}

impl CloudProviderFactory for MpdfCreditsFactory {
    fn mode(&self) -> OcrProviderMode {
        OcrProviderMode::MpdfCredits
    }

    fn config_digest(&self) -> String {
        self.config.digest()
    }

    fn fingerprint_contribution(&self) -> String {
        format!(
            "{}|layout={}|credential_slot={}|credits_per_page={}|endpoint={}",
            self.config.fingerprint(),
            self.local_layout_version,
            self.config.credential_slot,
            self.config.credits_per_page,
            self.endpoint
        )
    }

    fn max_credits(&self) -> u64 {
        self.max_credits
    }

    fn fallback(&self) -> CloudFallback {
        self.config.fallback
    }

    fn build(&self, local: Box<dyn PageOcrProvider>) -> CoreResult<Box<dyn OcrProvider>> {
        let transport = BrokeredTransport::new(
            &self.endpoint,
            &self.config.credential_slot,
            self.secrets.clone(),
            self.policy.clone(),
            // Filled in by `bind_job` once the reservation exists; a page
            // request is never sent before that.
            &self.reservation_hint,
        )
        .map(|transport| transport.with_cancellation(self.cancelled.clone()))
        .map_err(|error| CoreError::InvalidParameter(redact(&error.to_string())))?;
        let ledger = HttpCreditsBackend::new(
            &self.endpoint,
            &self.config.credential_slot,
            self.secrets.clone(),
            self.policy.clone(),
            self.verification_secret.clone(),
        )
        .map_err(|error| CoreError::InvalidParameter(redact(&error.to_string())))?;
        Ok(Box::new(
            CloudOcrProvider::new(
                self.config.clone(),
                Box::new(transport),
                local,
                self.local_layout_version.clone(),
            )
            .with_credits(Box::new(ledger)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySecretStore;

    #[test]
    fn a_plain_http_endpoint_is_refused_unless_development_mode_is_explicit() {
        let secrets = Arc::new(MemorySecretStore::default());
        assert!(GeminiTransport::new(
            "http://example.com",
            "slot",
            secrets.clone(),
            CloudTransportPolicy::default()
        )
        .is_err());
        assert!(GeminiTransport::new(
            "http://127.0.0.1:8080",
            "slot",
            secrets,
            CloudTransportPolicy {
                allow_loopback_http: true,
                ..Default::default()
            }
        )
        .is_ok());
    }

    #[test]
    fn an_endpoint_carrying_credentials_or_a_query_is_refused() {
        let secrets = Arc::new(MemorySecretStore::default());
        for endpoint in [
            "https://user:pass@example.com",
            "https://example.com/?key=abc",
        ] {
            assert!(
                GeminiTransport::new(
                    endpoint,
                    "slot",
                    secrets.clone(),
                    CloudTransportPolicy::default()
                )
                .is_err(),
                "{endpoint} must be refused"
            );
        }
    }

    #[test]
    fn a_non_google_https_origin_is_refused_before_any_credential_lookup() {
        let secrets = Arc::new(MemorySecretStore::default());
        assert!(GeminiTransport::new(
            "https://attacker.example",
            "stored-key",
            secrets,
            CloudTransportPolicy::default(),
        )
        .is_err());
    }

    #[test]
    fn a_missing_credential_is_reported_without_touching_the_network() {
        let secrets = Arc::new(MemorySecretStore::default());
        let transport = GeminiTransport::new(
            GEMINI_ENDPOINT,
            "absent",
            secrets,
            CloudTransportPolicy::default(),
        )
        .unwrap();
        assert!(!transport.credential_present());
    }

    #[test]
    fn a_model_name_that_is_not_a_bare_identifier_cannot_reshape_the_url() {
        let secrets = Arc::new(MemorySecretStore::default());
        let transport = GeminiTransport::new(
            GEMINI_ENDPOINT,
            "slot",
            secrets,
            CloudTransportPolicy::default(),
        )
        .unwrap();
        assert!(transport
            .model_url("../../v1/keys", ":generateContent")
            .is_err());
        assert!(transport.model_url("gemini-3.7-flash", "").is_ok());
    }

    #[test]
    fn the_byok_factory_reports_a_masked_credential_and_never_a_value() {
        let factory = GeminiByokFactory::new(
            CloudOcrConfig::gemini_byok("slot-name"),
            GEMINI_ENDPOINT,
            CloudTransportPolicy::default(),
            "tesseract-5",
        );
        let masked = factory.credential();
        assert_eq!(masked.slot, "slot-name");
        assert_eq!(masked.masked, "****");
        let json = serde_json::to_string(&masked).unwrap();
        assert!(!json.contains("AIza"));
    }

    #[test]
    fn byok_contributes_no_credits_ceiling_and_credits_contributes_one() {
        let byok = GeminiByokFactory::new(
            CloudOcrConfig::gemini_byok("slot"),
            GEMINI_ENDPOINT,
            CloudTransportPolicy::default(),
            "tesseract-5",
        );
        assert_eq!(byok.max_credits(), 0);
        let credits = MpdfCreditsFactory::new(
            CloudOcrConfig::mpdf_credits(4),
            "https://credits.example.com",
            CloudTransportPolicy::default(),
            "tesseract-5",
            400,
            b"verify".to_vec(),
        );
        assert_eq!(credits.max_credits(), 400);
        assert_ne!(
            byok.fingerprint_contribution(),
            credits.fingerprint_contribution()
        );
        assert!(!credits.release_blockers().is_empty());
    }

    #[test]
    fn a_truncated_or_unclassified_candidate_is_not_a_completed_page() {
        assert!(completed_finish_reason(Some("STOP")));
        assert!(!completed_finish_reason(Some("MAX_TOKENS")));
        assert!(!completed_finish_reason(Some("SAFETY")));
        assert!(!completed_finish_reason(None));
    }
}
