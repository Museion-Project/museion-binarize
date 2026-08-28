//! Reusable, bounded transport for `mpdf-api` 0.1.
//!
//! The client never logs or formats the bearer token.  Production callers
//! provide an OS-backed [`SecretStore`]; tests use [`MemorySecretStore`].

use std::fmt;
use std::io::Read;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use zeroize::Zeroize;

use mpdf_core::remote_api::{
    sha256_hex, validate_consent, ApiPlan, ApiTaskReceipt, Consent, RemoteApiError,
    RemoteOcrResult, MAX_ARTIFACT_BYTES,
};
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CREDENTIAL_SERVICE: &str = "org.mpdf.api";
pub const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_RETRIES: u32 = 3;

/// Non-secret metadata for one actual HTTP attempt. Callers persist these in
/// their append-only audit store after binding them to a local/remote task.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RequestTrace {
    pub kind: String,
    pub attempt: u32,
    pub request_digest: Option<String>,
    pub response_digest: Option<String>,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub http_status: Option<u16>,
    pub outcome: String,
}

fn valid_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn request_kind(method: &Method, suffix: &str) -> &'static str {
    if method == Method::POST && suffix == "v1/tasks" {
        "create"
    } else if method == Method::PUT {
        "upload"
    } else if method == Method::POST && suffix.ends_with("/start") {
        "start"
    } else if method == Method::DELETE && suffix.ends_with("/content") {
        "delete_content"
    } else if method == Method::POST && suffix.ends_with("/cancel") {
        "cancel"
    } else if method == Method::GET && suffix.ends_with("/result") {
        "result"
    } else {
        "status"
    }
}

#[derive(Debug, Error)]
pub enum ApiClientError {
    #[error("invalid API endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("credential store unavailable")]
    CredentialStoreUnavailable,
    #[error("credential operation failed")]
    Credential(String),
    #[error("API request failed: {0}")]
    Transport(String),
    #[error("API returned HTTP status {status}")]
    Http { status: u16 },
    #[error("API response is invalid: {0}")]
    Response(String),
    #[error("API request cancelled")]
    Cancelled,
    #[error("API policy rejected request: {0}")]
    Policy(String),
}
pub type Result<T> = std::result::Result<T, ApiClientError>;

/// A secret is intentionally not serializable and has a redacted Debug impl.
pub struct Secret(String);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub trait SecretStore: Send + Sync {
    fn get(&self, profile_id: &str) -> Result<Option<Secret>>;
    fn set(&self, profile_id: &str, secret: Secret) -> Result<()>;
    fn delete(&self, profile_id: &str) -> Result<()>;
}

/// Deterministic test store. It is process-memory-only and never accesses a
/// developer keychain.
#[derive(Default)]
pub struct MemorySecretStore {
    values: std::sync::Mutex<std::collections::BTreeMap<String, Secret>>,
}
impl SecretStore for MemorySecretStore {
    fn get(&self, id: &str) -> Result<Option<Secret>> {
        self.values
            .lock()
            .map_err(|_| ApiClientError::Credential("store poisoned".into()))
            .map(|m| m.get(id).map(|s| Secret::new(s.as_str())))
    }
    fn set(&self, id: &str, secret: Secret) -> Result<()> {
        if id.is_empty() || id.len() > 256 {
            return Err(ApiClientError::Credential("invalid profile id".into()));
        }
        self.values
            .lock()
            .map_err(|_| ApiClientError::Credential("store poisoned".into()))
            .map(|mut m| {
                m.insert(id.into(), secret);
            })
    }
    fn delete(&self, id: &str) -> Result<()> {
        self.values
            .lock()
            .map_err(|_| ApiClientError::Credential("store poisoned".into()))
            .map(|mut m| {
                m.remove(id);
            })
    }
}

#[derive(Debug, Default)]
pub struct NativeSecretStore;
impl NativeSecretStore {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    fn entry(profile: &str) -> Result<keyring::Entry> {
        if profile.is_empty()
            || profile.len() > 256
            || !profile
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(ApiClientError::Credential("invalid profile id".into()));
        }
        keyring::Entry::new(CREDENTIAL_SERVICE, profile)
            .map_err(|_| ApiClientError::CredentialStoreUnavailable)
    }
}

/// Explicit CI/development-only one-shot source. It is never used by the
/// credential commands and is never persisted or included in diagnostics.
#[derive(Debug, Default)]
pub struct EnvironmentSecretStore;
impl SecretStore for EnvironmentSecretStore {
    fn get(&self, _: &str) -> Result<Option<Secret>> {
        Ok(std::env::var("MPDF_API_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .map(Secret::new))
    }
    fn set(&self, _: &str, _: Secret) -> Result<()> {
        Err(ApiClientError::CredentialStoreUnavailable)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(ApiClientError::CredentialStoreUnavailable)
    }
}

#[derive(Debug, Default)]
pub struct RuntimeSecretStore {
    native: NativeSecretStore,
    environment: EnvironmentSecretStore,
}
impl SecretStore for RuntimeSecretStore {
    fn get(&self, profile: &str) -> Result<Option<Secret>> {
        // The environment source is an explicit headless/CI profile. An
        // ambient variable must never override a named native-keychain
        // profile silently.
        if profile == "env" {
            return self.environment.get(profile);
        }
        self.native.get(profile)
    }
    fn set(&self, profile: &str, secret: Secret) -> Result<()> {
        self.native.set(profile, secret)
    }
    fn delete(&self, profile: &str) -> Result<()> {
        self.native.delete(profile)
    }
}
impl SecretStore for NativeSecretStore {
    fn get(&self, profile: &str) -> Result<Option<Secret>> {
        match Self::entry(profile)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ApiClientError::CredentialStoreUnavailable),
        }
    }
    fn set(&self, profile: &str, secret: Secret) -> Result<()> {
        Self::entry(profile)?
            .set_password(secret.as_str())
            .map_err(|_| ApiClientError::CredentialStoreUnavailable)
    }
    fn delete(&self, profile: &str) -> Result<()> {
        match Self::entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ApiClientError::CredentialStoreUnavailable),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ApiClientConfig {
    pub endpoint: String,
    pub profile_id: String,
    pub allow_loopback_http: bool,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_response_bytes: u64,
    pub max_retries: u32,
}
impl ApiClientConfig {
    pub fn validate(&self) -> Result<Url> {
        let url = Url::parse(&self.endpoint)
            .map_err(|_| ApiClientError::InvalidEndpoint("malformed URL".into()))?;
        if url.username() != ""
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
        {
            return Err(ApiClientError::InvalidEndpoint(
                "credentials, query, and fragments are not allowed".into(),
            ));
        }
        if !mpdf_core::remote_api::validate_origin(&self.endpoint) {
            return Err(ApiClientError::InvalidEndpoint(
                "endpoint must be a strict HTTPS origin".into(),
            ));
        }
        if url.scheme() == "http" && !self.allow_loopback_http {
            return Err(ApiClientError::InvalidEndpoint(
                "HTTPS is required; loopback HTTP requires explicit development mode".into(),
            ));
        }
        if self.max_response_bytes == 0
            || self.max_response_bytes > MAX_RESPONSE_BYTES
            || self.max_retries > MAX_RETRIES
        {
            return Err(ApiClientError::Policy(
                "resource limits out of range".into(),
            ));
        }
        Ok(url)
    }
}
pub struct ApiClient<S: SecretStore> {
    config: ApiClientConfig,
    secrets: Arc<S>,
    client: Client,
    cancelled: Arc<AtomicBool>,
    traces: Arc<Mutex<Vec<RequestTrace>>>,
}
impl<S: SecretStore> ApiClient<S> {
    pub fn new(config: ApiClientConfig, secrets: Arc<S>) -> Result<Self> {
        config.validate()?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .timeout(config.total_timeout)
            .build()
            .map_err(|e| ApiClientError::Transport(e.to_string()))?;
        Ok(Self {
            config,
            secrets,
            client,
            cancelled: Arc::new(AtomicBool::new(false)),
            traces: Arc::new(Mutex::new(Vec::new())),
        })
    }
    pub fn drain_traces(&self) -> Vec<RequestTrace> {
        self.traces
            .lock()
            .map(|mut traces| std::mem::take(&mut *traces))
            .unwrap_or_default()
    }
    pub fn cancellation(&self) -> Cancellation {
        Cancellation(self.cancelled.clone())
    }
    fn auth(&self) -> Result<Option<Secret>> {
        self.secrets.get(&self.config.profile_id)
    }
    fn url(&self, suffix: &str) -> Result<Url> {
        let origin = self.config.validate()?;
        origin
            .join(suffix)
            .map_err(|_| ApiClientError::InvalidEndpoint("invalid API path".into()))
    }
    fn send(
        &self,
        method: Method,
        suffix: &str,
        body: Option<Vec<u8>>,
        idempotency: Option<&str>,
    ) -> Result<Vec<u8>> {
        let url = self.url(suffix)?;
        let kind = request_kind(&method, suffix).to_owned();
        let request_digest = body.as_deref().map(sha256_hex);
        let request_bytes = body.as_ref().map_or(0, |value| value.len() as u64);
        let secret = self.auth()?;
        let mut attempt = 0;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(ApiClientError::Cancelled);
            }
            attempt += 1;
            let mut req = self.client.request(method.clone(), url.clone());
            if let Some(s) = &secret {
                req = req.bearer_auth(s.as_str());
            }
            if let Some(key) = idempotency {
                req = req.header("Idempotency-Key", key);
            }
            if let Some(b) = &body {
                let content_type = if method == Method::PUT {
                    "application/octet-stream"
                } else {
                    "application/json"
                };
                req = req.header("Content-Type", content_type).body(b.clone());
            }
            match req.send() {
                Ok(r) => {
                    let status = r.status();
                    if retryable(status) && attempt < self.config.max_retries {
                        self.push_trace(RequestTrace {
                            kind: kind.clone(),
                            attempt,
                            request_digest: request_digest.clone(),
                            response_digest: None,
                            request_bytes,
                            response_bytes: 0,
                            http_status: Some(status.as_u16()),
                            outcome: "retry".into(),
                        });
                        wait_retry(&self.cancelled, retry_after(&r))?;
                        continue;
                    }
                    if !status.is_success() {
                        self.push_trace(RequestTrace {
                            kind: kind.clone(),
                            attempt,
                            request_digest: request_digest.clone(),
                            response_digest: None,
                            request_bytes,
                            response_bytes: 0,
                            http_status: Some(status.as_u16()),
                            outcome: "http_error".into(),
                        });
                        return Err(ApiClientError::Http {
                            status: status.as_u16(),
                        });
                    }
                    let bytes = bounded_bytes(r, self.config.max_response_bytes)?;
                    self.push_trace(RequestTrace {
                        kind: kind.clone(),
                        attempt,
                        request_digest: request_digest.clone(),
                        response_digest: Some(sha256_hex(&bytes)),
                        request_bytes,
                        response_bytes: bytes.len() as u64,
                        http_status: Some(status.as_u16()),
                        outcome: "success".into(),
                    });
                    return Ok(bytes);
                }
                Err(e) => {
                    let will_retry = e.is_timeout() && attempt < self.config.max_retries;
                    self.push_trace(RequestTrace {
                        kind: kind.clone(),
                        attempt,
                        request_digest: request_digest.clone(),
                        response_digest: None,
                        request_bytes,
                        response_bytes: 0,
                        http_status: None,
                        outcome: if will_retry {
                            "retry"
                        } else {
                            "transport_error"
                        }
                        .into(),
                    });
                    if will_retry {
                        wait_retry(&self.cancelled, None)?;
                        continue;
                    }
                    return Err(if e.is_timeout() {
                        ApiClientError::Transport("request timed out".into())
                    } else {
                        ApiClientError::Transport("request failed".into())
                    });
                }
            }
        }
    }
    fn push_trace(&self, trace: RequestTrace) {
        if let Ok(mut traces) = self.traces.lock() {
            traces.push(trace);
        }
    }
    pub fn create_task(&self, plan: &ApiPlan, consent: &Consent) -> Result<CreateTaskResponse> {
        validate_consent(plan, consent)
            .map_err(|_| ApiClientError::Policy("consent digest is stale".into()))?;
        let body = serde_json::to_vec(&CreateTaskRequest {
            protocol: "mpdf-api".into(),
            protocol_version: "0.1".into(),
            operation: plan.operation,
            source_sha256: plan.source_sha256.clone(),
            source_bytes: plan.source_bytes,
            page_count: plan.page_count,
            provider: plan.provider.clone(),
            model: plan.model.clone(),
            max_cost_micros: plan.max_cost_micros,
            currency: plan.currency.clone(),
            retention: plan.retention,
            plan_digest: plan.plan_digest.clone(),
        })
        .map_err(|e| ApiClientError::Response(e.to_string()))?;
        let bytes = self.send(
            Method::POST,
            "v1/tasks",
            Some(body),
            Some(
                &plan
                    .idempotency_key("create")
                    .map_err(|e| ApiClientError::Policy(e.to_string()))?,
            ),
        )?;
        let response: CreateTaskResponse = serde_json::from_slice(&bytes)
            .map_err(|_| ApiClientError::Response("invalid task response".into()))?;
        if !valid_task_id(&response.task_id)
            || response.request_id
                != plan
                    .request_id()
                    .map_err(|e| ApiClientError::Policy(e.to_string()))?
        {
            return Err(ApiClientError::Response(
                "task response binding mismatch".into(),
            ));
        }
        Ok(response)
    }
    pub fn upload_blob(&self, plan: &ApiPlan, bytes: &[u8], consent: &Consent) -> Result<()> {
        validate_consent(plan, consent)
            .map_err(|_| ApiClientError::Policy("consent digest is stale".into()))?;
        if bytes.len() as u64 > MAX_SOURCE_BYTES
            || bytes.len() as u64 != plan.source_bytes
            || sha256_hex(bytes) != plan.source_sha256
        {
            return Err(ApiClientError::Policy("source does not match plan".into()));
        }
        let key = plan
            .idempotency_key("upload")
            .map_err(|e| ApiClientError::Policy(e.to_string()))?;
        self.send(
            Method::PUT,
            &format!("v1/blobs/{}", plan.source_sha256),
            Some(bytes.to_vec()),
            Some(&key),
        )
        .map(|_| ())
    }
    pub fn start(&self, plan: &ApiPlan, task_id: &str) -> Result<()> {
        if !valid_task_id(task_id) {
            return Err(ApiClientError::Policy("invalid task ID".into()));
        }
        self.send(
            Method::POST,
            &format!("v1/tasks/{task_id}/start"),
            Some(Vec::new()),
            Some(
                &plan
                    .idempotency_key("start")
                    .map_err(|e| ApiClientError::Policy(e.to_string()))?,
            ),
        )
        .map(|_| ())
    }
    pub fn status(&self, task_id: &str) -> Result<TaskStatusResponse> {
        if !valid_task_id(task_id) {
            return Err(ApiClientError::Policy("invalid task ID".into()));
        }
        let bytes = self.send(Method::GET, &format!("v1/tasks/{task_id}"), None, None)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ApiClientError::Response("invalid status response".into()))
    }
    /// Polls status before fetching a result. This bounded loop is deliberately
    /// conservative: a result is never requested while the task is still
    /// queued/running, and cost regressions fail closed.
    pub fn poll_until_terminal(&self, plan: &ApiPlan, task_id: &str) -> Result<TaskStatusResponse> {
        let mut previous = 0_u64;
        for _ in 0..60 {
            let status = self.status(task_id)?;
            if status.task_id != task_id
                || status.used_cost_micros < previous
                || status.used_cost_micros > plan.max_cost_micros
            {
                return Err(ApiClientError::Response(
                    "status binding or budget regression".into(),
                ));
            }
            previous = status.used_cost_micros;
            match status.state {
                mpdf_core::remote_api::TaskState::Completed
                | mpdf_core::remote_api::TaskState::ResultInstalled => return Ok(status),
                mpdf_core::remote_api::TaskState::PausedBudget => {
                    return Err(ApiClientError::Policy(
                        "task paused: budget exhausted".into(),
                    ))
                }
                mpdf_core::remote_api::TaskState::PausedService => {
                    return Err(ApiClientError::Transport(
                        "task paused: service unavailable".into(),
                    ))
                }
                mpdf_core::remote_api::TaskState::Cancelled
                | mpdf_core::remote_api::TaskState::Failed => {
                    return Err(ApiClientError::Response("task did not complete".into()))
                }
                _ => wait_retry(&self.cancelled, Some(Duration::from_millis(25)))?,
            }
        }
        Err(ApiClientError::Transport(
            "status polling limit exceeded".into(),
        ))
    }
    pub fn result(&self, plan: &ApiPlan, task_id: &str) -> Result<RemoteOcrResult> {
        if !valid_task_id(task_id) {
            return Err(ApiClientError::Policy("invalid task ID".into()));
        }
        let bytes = self.send(
            Method::GET,
            &format!("v1/tasks/{task_id}/result"),
            None,
            None,
        )?;
        let wire: ResultWire = serde_json::from_slice(&bytes)
            .map_err(|_| ApiClientError::Response("invalid result response".into()))?;
        if wire.source_sha256 != plan.source_sha256
            || wire.task_id != task_id
            || wire.protocol != "mpdf-api"
            || wire.protocol_version != "0.1"
            || wire.pages.len() != plan.page_count as usize
            || wire
                .pages
                .iter()
                .map(|p| p.page_index)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != wire.pages.len()
        {
            return Err(ApiClientError::Response(
                "result binding or protocol mismatch".into(),
            ));
        }
        let raw = wire.raw_artifact.into_bytes();
        if raw.len() as u64 > MAX_ARTIFACT_BYTES || sha256_hex(&raw) != wire.result_digest {
            return Err(ApiClientError::Response("result digest mismatch".into()));
        }
        let result = RemoteOcrResult {
            protocol: wire.protocol,
            protocol_version: wire.protocol_version,
            task_id: wire.task_id,
            source_sha256: wire.source_sha256,
            result_digest: wire.result_digest,
            raw_artifact: raw,
            pages: wire.pages,
        };
        result
            .validate(task_id, &plan.source_sha256)
            .map_err(|e| ApiClientError::Response(e.to_string()))?;
        Ok(result)
    }
    pub fn delete_content(&self, plan: &ApiPlan, task_id: &str) -> Result<()> {
        if !valid_task_id(task_id) {
            return Err(ApiClientError::Policy("invalid task ID".into()));
        }
        self.send(
            Method::DELETE,
            &format!("v1/tasks/{task_id}/content"),
            Some(Vec::new()),
            Some(
                &plan
                    .idempotency_key("delete-content")
                    .map_err(|e| ApiClientError::Policy(e.to_string()))?,
            ),
        )
        .map(|_| ())
    }
    /// Requests cancellation without introducing a new protocol resource;
    /// the receipt binding supplies the stable mutation key.
    pub fn cancel(&self, receipt: &ApiTaskReceipt) -> Result<()> {
        receipt
            .validate()
            .map_err(|e| ApiClientError::Policy(e.to_string()))?;
        let key = sha256_hex(
            format!("{}\ncancel\n{}", receipt.request_id, receipt.receipt_digest).as_bytes(),
        );
        self.send(
            Method::POST,
            &format!("v1/tasks/{}/cancel", receipt.task_id),
            Some(Vec::new()),
            Some(&key),
        )
        .map(|_| ())
    }
    pub fn delete_content_receipt(&self, receipt: &ApiTaskReceipt) -> Result<()> {
        receipt
            .validate()
            .map_err(|e| ApiClientError::Policy(e.to_string()))?;
        let key = sha256_hex(
            format!(
                "{}\ndelete-content\n{}",
                receipt.request_id, receipt.receipt_digest
            )
            .as_bytes(),
        );
        self.send(
            Method::DELETE,
            &format!("v1/tasks/{}/content", receipt.task_id),
            Some(Vec::new()),
            Some(&key),
        )
        .map(|_| ())
    }
    pub fn receipt(&self, plan: &ApiPlan, task_id: impl Into<String>) -> Result<ApiTaskReceipt> {
        ApiTaskReceipt::from_plan(plan, task_id).map_err(|e| ApiClientError::Policy(e.to_string()))
    }
}

#[derive(Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn retryable(s: StatusCode) -> bool {
    s == StatusCode::REQUEST_TIMEOUT || s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error()
}
fn retry_after(r: &Response) -> Option<Duration> {
    r.headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| Duration::from_secs(s.min(30)))
}
fn wait_retry(cancelled: &AtomicBool, delay: Option<Duration>) -> Result<()> {
    let end = std::time::Instant::now() + delay.unwrap_or(Duration::from_millis(25));
    while std::time::Instant::now() < end {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ApiClientError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn bounded_bytes(r: Response, limit: u64) -> Result<Vec<u8>> {
    if r.content_length().is_some_and(|n| n > limit) {
        return Err(ApiClientError::Response("response exceeds limit".into()));
    }
    let mut out = Vec::new();
    r.take(limit + 1)
        .read_to_end(&mut out)
        .map_err(|_| ApiClientError::Transport("response read failed".into()))?;
    if out.len() as u64 > limit {
        return Err(ApiClientError::Response("response exceeds limit".into()));
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
struct CreateTaskRequest {
    protocol: String,
    protocol_version: String,
    operation: mpdf_core::remote_api::RemoteOperation,
    source_sha256: String,
    source_bytes: u64,
    page_count: u32,
    provider: String,
    model: String,
    max_cost_micros: u64,
    currency: String,
    retention: mpdf_core::remote_api::Retention,
    plan_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub request_id: String,
    #[serde(default)]
    pub deduplicated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStatusResponse {
    pub task_id: String,
    pub state: mpdf_core::remote_api::TaskState,
    #[serde(default)]
    pub used_cost_micros: u64,
    #[serde(default)]
    pub retention: mpdf_core::remote_api::RetentionState,
}
#[derive(Debug, Clone, Deserialize)]
struct ResultWire {
    protocol: String,
    protocol_version: String,
    task_id: String,
    source_sha256: String,
    result_digest: String,
    raw_artifact: String,
    pages: Vec<mpdf_core::ocr::OcrPage>,
}

impl From<RemoteApiError> for ApiClientError {
    fn from(e: RemoteApiError) -> Self {
        ApiClientError::Policy(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpdf_core::remote_api::{ApiPlan, Retention};
    use std::io::Write as _;
    use std::net::TcpListener;
    #[test]
    fn endpoint_policy() {
        let base = ApiClientConfig {
            endpoint: "http://127.0.0.1:1234".into(),
            profile_id: "p".into(),
            allow_loopback_http: false,
            connect_timeout: Duration::from_secs(1),
            total_timeout: Duration::from_secs(2),
            max_response_bytes: 1024,
            max_retries: 3,
        };
        assert!(base.validate().is_err());
        let mut a = base.clone();
        a.allow_loopback_http = true;
        assert!(a.validate().is_ok());
        let mut b = a.clone();
        b.endpoint = "https://u:p@example.test".into();
        assert!(b.validate().is_err());
        let mut localhost = a.clone();
        localhost.endpoint = "http://localhost:1234".into();
        assert!(localhost.validate().is_err());
        let mut path = a.clone();
        path.endpoint = "https://example.test/v1".into();
        assert!(path.validate().is_err());
        let mut fragment = a;
        fragment.endpoint = "https://example.test/#x".into();
        assert!(fragment.validate().is_err());
    }
    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("never-print");
        assert!(!format!("{s:?}").contains("never-print"));
    }

    #[test]
    fn https_client_builds_with_production_tls_backend() {
        let config = ApiClientConfig {
            endpoint: "https://api.example.test/".into(),
            profile_id: "p".into(),
            allow_loopback_http: false,
            connect_timeout: Duration::from_secs(1),
            total_timeout: Duration::from_secs(2),
            max_response_bytes: 1024,
            max_retries: 3,
        };
        assert!(ApiClient::new(config, Arc::new(MemorySecretStore::default())).is_ok());
    }

    #[test]
    fn cancellation_interrupts_retry_after_without_another_attempt() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let source = b"retry source";
        let plan = ApiPlan::new(
            format!("http://127.0.0.1:{}", address.port()),
            "fixture",
            "ocr-1",
            sha256_hex(source),
            source.len() as u64,
            1,
            1000,
            "USD",
            Retention::DeleteAfterResult,
        )
        .unwrap();
        let consent = plan.consent(1).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });
        let client = ApiClient::new(
            ApiClientConfig {
                endpoint: plan.origin.clone(),
                profile_id: "fixture".into(),
                allow_loopback_http: true,
                connect_timeout: Duration::from_secs(1),
                total_timeout: Duration::from_secs(4),
                max_response_bytes: 1024,
                max_retries: 3,
            },
            Arc::new(MemorySecretStore::default()),
        )
        .unwrap();
        let cancellation = client.cancellation();
        let worker = std::thread::spawn(move || client.create_task(&plan, &consent));
        std::thread::sleep(Duration::from_millis(50));
        cancellation.cancel();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ApiClientError::Cancelled)
        ));
        server.join().unwrap();
    }

    #[test]
    fn loopback_fixture_exercises_real_http_create_upload_start() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let source = b"fixture source";
        let plan = ApiPlan::new(
            format!("http://127.0.0.1:{}", address.port()),
            "fixture",
            "ocr-1",
            sha256_hex(source),
            source.len() as u64,
            1,
            1000,
            "USD",
            Retention::DeleteAfterResult,
        )
        .unwrap();
        let expected_request_id = plan.request_id().unwrap();
        let server = std::thread::spawn(move || {
            for (index, (method, path)) in [
                ("POST", "/v1/tasks"),
                ("PUT", "/v1/blobs/"),
                ("POST", "/v1/tasks/task-1/start"),
            ]
            .into_iter()
            .enumerate()
            {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut one = [0_u8; 1];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    stream.read_exact(&mut one).unwrap();
                    request.push(one[0]);
                }
                let header_text = String::from_utf8_lossy(&request);
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    })
                    .unwrap_or(0);
                let mut body = vec![0_u8; content_length];
                stream.read_exact(&mut body).unwrap();
                let text = String::from_utf8_lossy(&request);
                assert!(text.starts_with(&format!("{method} {path}")));
                if index == 0 {
                    write_response(
                        &mut stream,
                        format!(r#"{{"task_id":"task-1","request_id":"{expected_request_id}","deduplicated":false}}"#).as_bytes(),
                    );
                } else {
                    write_response(&mut stream, b"{}");
                }
            }
        });
        let store = Arc::new(MemorySecretStore::default());
        store.set("fixture", Secret::new("test-token")).unwrap();
        let config = ApiClientConfig {
            endpoint: format!("http://127.0.0.1:{}", address.port()),
            profile_id: "fixture".into(),
            allow_loopback_http: true,
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(4),
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_retries: 3,
        };
        let client = ApiClient::new(config, store).unwrap();
        let consent = plan.consent(1).unwrap();
        let created = client.create_task(&plan, &consent).unwrap();
        assert!(!created.deduplicated);
        client.upload_blob(&plan, source, &consent).unwrap();
        client.start(&plan, &created.task_id).unwrap();
        let traces = client.drain_traces();
        assert_eq!(traces.len(), 3);
        assert_eq!(traces[0].kind, "create");
        assert_eq!(traces[1].kind, "upload");
        assert_eq!(traces[2].kind, "start");
        assert!(!serde_json::to_string(&traces)
            .unwrap()
            .contains("test-token"));
        server.join().unwrap();
    }

    #[test]
    fn loopback_fixture_full_receipt_poll_result_install_and_delete_chain() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let source = b"portable source";
        let plan = ApiPlan::new(
            format!("http://127.0.0.1:{}", address.port()),
            "fixture",
            "ocr-1",
            sha256_hex(source),
            source.len() as u64,
            1,
            1000,
            "USD",
            Retention::DeleteAfterResult,
        )
        .unwrap();
        let expected_request_id = plan.request_id().unwrap();
        let expected_source_digest = plan.source_sha256.clone();
        let artifact_digest = sha256_hex(b"raw-artifact");
        let page = mpdf_core::ocr::OcrPage {
            page_index: 0,
            route: mpdf_core::ocr::OcrRoute::Ocr {
                reason: mpdf_core::ocr::OcrRouteReason::MissingText,
            },
            width: 100,
            height: 100,
            blocks: vec![mpdf_core::ocr::OcrBlock {
                bbox: mpdf_core::ocr::OcrBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 20.0,
                },
                confidence: 1.0,
                reading_order: 0,
                lines: vec![mpdf_core::ocr::OcrLine {
                    bbox: mpdf_core::ocr::OcrBox {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 20.0,
                    },
                    confidence: 1.0,
                    reading_order: 0,
                    words: vec![mpdf_core::ocr::OcrWord {
                        text: "ἄνθρωπος".into(),
                        normalized_text: "ἄνθρωπος".into(),
                        bbox: mpdf_core::ocr::OcrBox {
                            x: 0.0,
                            y: 0.0,
                            width: 50.0,
                            height: 20.0,
                        },
                        confidence: 1.0,
                        reading_order: 0,
                    }],
                }],
            }],
            revisions: vec![],
            provider_provenance: None,
            provider_raw_artifact: Some(artifact_digest.clone()),
        };
        let page_json = serde_json::to_string(&page).unwrap();
        let server = std::thread::spawn(move || {
            for index in 0..7 {
                let (mut stream, _) = listener.accept().unwrap();
                let (request, body) = read_http_request(&mut stream);
                let first = request.lines().next().unwrap_or_default();
                match index {
                    0 => {
                        assert!(first.starts_with("POST /v1/tasks"));
                        write_response(&mut stream, format!(r#"{{"task_id":"portable-task","request_id":"{expected_request_id}","deduplicated":false}}"#).as_bytes());
                    }
                    1 => {
                        assert!(first.starts_with("PUT /v1/blobs/"));
                        assert_eq!(body, source);
                        write_response(&mut stream, b"{}");
                    }
                    2 => {
                        assert!(first.starts_with("POST /v1/tasks/portable-task/start"));
                        write_response(&mut stream, b"{}");
                    }
                    3 | 5 => {
                        assert!(first.starts_with("GET /v1/tasks/portable-task"));
                        write_response(&mut stream, br#"{"task_id":"portable-task","state":"completed","used_cost_micros":2,"retention":"pending"}"#);
                    }
                    4 => {
                        assert!(first.starts_with("GET /v1/tasks/portable-task/result"));
                        write_response(&mut stream, format!(r#"{{"protocol":"mpdf-api","protocol_version":"0.1","task_id":"portable-task","source_sha256":"{}","result_digest":"{}","raw_artifact":"raw-artifact","pages":[{}]}}"#, expected_source_digest, artifact_digest, page_json).as_bytes());
                    }
                    6 => {
                        assert!(first.starts_with("DELETE /v1/tasks/portable-task/content"));
                        write_response(&mut stream, b"{}");
                    }
                    _ => unreachable!(),
                }
            }
        });
        let store = Arc::new(MemorySecretStore::default());
        store.set("fixture", Secret::new("test-token")).unwrap();
        let config = ApiClientConfig {
            endpoint: format!("http://127.0.0.1:{}", address.port()),
            profile_id: "fixture".into(),
            allow_loopback_http: true,
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(4),
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_retries: 3,
        };
        let client = ApiClient::new(config.clone(), store.clone()).unwrap();
        let consent = plan.consent(1).unwrap();
        let created = client.create_task(&plan, &consent).unwrap();
        client.upload_blob(&plan, source, &consent).unwrap();
        client.start(&plan, &created.task_id).unwrap();
        let receipt = client.receipt(&plan, &created.task_id).unwrap();
        assert_eq!(
            client.status(&created.task_id).unwrap().state,
            mpdf_core::remote_api::TaskState::Completed
        );
        let result = client.result(&plan, &created.task_id).unwrap();
        let root = tempfile::tempdir().unwrap();
        let page_id = format!(
            "page-{}",
            sha256_hex(format!("{}:0", plan.source_sha256).as_bytes())
        );
        let package = mpdf_core::document_package::DocumentPackage {
            manifest: mpdf_core::document_package::Manifest {
                schema: mpdf_core::document_package::MDP_SCHEMA.into(),
                schema_version: mpdf_core::document_package::MDP_SCHEMA_VERSION.into(),
                document_id: mpdf_core::document_package::document_id_for_sha256(
                    &plan.source_sha256,
                ),
                source_id: mpdf_core::document_package::source_id_for_sha256(&plan.source_sha256),
                page_count: 1,
                asset_count: 0,
                tool: mpdf_core::document_package::ToolInfo {
                    name: "mpdf-test".into(),
                    version: "0.1".into(),
                },
            },
            source: mpdf_core::document_package::Source {
                source_id: mpdf_core::document_package::source_id_for_sha256(&plan.source_sha256),
                kind: mpdf_core::document_package::SourceKind::Pdf,
                content_sha256: plan.source_sha256.clone(),
                byte_len: source.len() as u64,
                page_count: 1,
                external_reference: Some("fixture.pdf".into()),
                packaged_path: None,
            },
            pages: vec![mpdf_core::document_package::Page {
                page_id,
                physical_index: 0,
                order: 0,
                rotation_degrees: 0,
                master_space: mpdf_core::document_package::CoordinateSpace {
                    id: "master".into(),
                    unit: mpdf_core::document_package::CoordinateUnit::Pixels,
                    width: 100.0,
                    height: 100.0,
                    origin: mpdf_core::document_package::Origin::TopLeft,
                    pixels_per_inch: Some(mpdf_core::document_package::CANONICAL_MASTER_DPI),
                },
                source_space: mpdf_core::document_package::CoordinateSpace {
                    id: "pdf".into(),
                    unit: mpdf_core::document_package::CoordinateUnit::PdfPoints,
                    width: 100.0,
                    height: 100.0,
                    origin: mpdf_core::document_package::Origin::BottomLeft,
                    pixels_per_inch: None,
                },
                transforms: vec![mpdf_core::document_package::AffineTransform {
                    from_space: "pdf".into(),
                    to_space: "master".into(),
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: -1.0,
                    e: 0.0,
                    f: 100.0,
                }],
                printed_page_label: None,
                existing_outline_evidence: vec![],
                typography_evidence: vec![],
                region_evidence: vec![],
                asset_ids: vec![],
            }],
            assets: vec![],
            provenance: vec![],
            validation: mpdf_core::document_package::ValidationSummary {
                schema: "mpdf-validation".into(),
                schema_version: "0.1".into(),
                valid: true,
                checked_pages: 1,
                checked_assets: 0,
                errors: vec![],
            },
        };
        let package_root = root.path().join("mdp");
        package.write_to(&package_root).unwrap();
        let artifact =
            mpdf_core::remote_api::install_remote_ocr_result(&package_root, &result).unwrap();
        assert!(artifact.exists());
        let fresh = ApiClient::new(config, store).unwrap();
        assert_eq!(
            fresh.status(&receipt.task_id).unwrap().task_id,
            receipt.task_id
        );
        fresh.delete_content_receipt(&receipt).unwrap();
        server.join().unwrap();
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        let mut request = Vec::new();
        let mut one = [0_u8; 1];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            stream.read_exact(&mut one).unwrap();
            request.push(one[0]);
        }
        let text = String::from_utf8_lossy(&request).into_owned();
        let length = text
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        let mut body = vec![0_u8; length];
        stream.read_exact(&mut body).unwrap();
        (text, body)
    }

    fn write_response(stream: &mut std::net::TcpStream, body: &[u8]) {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    }
}
