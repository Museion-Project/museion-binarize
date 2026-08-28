//! Network-free representation of consented remote API OCR tasks.
//!
//! This module contains policy, durable records, canonical serialization and
//! result verification only.  HTTP and credential storage live in the
//! `mpdf-api-client` crate so the processing core remains usable offline.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::{Host, Url};

pub const PLAN_SCHEMA: &str = "mpdf-api-plan";
pub const PLAN_VERSION: &str = "0.1";
pub const RECEIPT_SCHEMA: &str = "mpdf-api-task-receipt";
pub const RECEIPT_VERSION: &str = "0.1";
pub const AUDIT_SCHEMA: &str = "mpdf-api-audit";
pub const AUDIT_VERSION: &str = "0.1";
pub const API_PROTOCOL: &str = "mpdf-api";
pub const API_PROTOCOL_VERSION: &str = "0.1";
pub const MAX_FIELD_BYTES: usize = 512;
pub const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_AUDIT_MESSAGE_BYTES: usize = 1024;
pub const MAX_TASK_ID_BYTES: usize = 256;

#[derive(Debug, Error)]
pub enum RemoteApiError {
    #[error("invalid API record: {0}")]
    Invalid(String),
    #[error("unsupported API protocol: {0}")]
    UnsupportedProtocol(String),
    #[error("API database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("API artifact error: {0}")]
    Artifact(String),
    #[error("API consent is required or stale")]
    ConsentRequired,
    #[error("API budget exceeded")]
    BudgetExceeded,
}
pub type Result<T> = std::result::Result<T, RemoteApiError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOperation {
    Ocr,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    DeleteAfterResult,
    KeepUntilDeleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    Local,
    Api,
    ApiThenLocal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Planned,
    Consented,
    Creating,
    UploadPending,
    Ready,
    Running,
    Completed,
    ResultInstalled,
    PausedBudget,
    PausedService,
    Cancelling,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetentionState {
    #[default]
    NotRequested,
    Pending,
    Acknowledged,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApiPlan {
    pub schema: String,
    pub schema_version: String,
    pub origin: String,
    pub operation: RemoteOperation,
    pub provider: String,
    pub model: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub page_count: u32,
    pub retention: Retention,
    pub max_cost_micros: u64,
    pub currency: String,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    pub plan_digest: String,
    pub consented_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiTaskReceipt {
    pub schema: String,
    pub schema_version: String,
    pub protocol: String,
    pub protocol_version: String,
    pub origin: String,
    pub task_id: String,
    pub request_id: String,
    pub operation: RemoteOperation,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub page_count: u32,
    pub plan_digest: String,
    pub provider: String,
    pub model: String,
    pub max_cost_micros: u64,
    pub currency: String,
    pub retention: Retention,
    pub receipt_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiAuditEvent {
    pub schema: String,
    pub schema_version: String,
    pub event_id: String,
    pub task_id: String,
    pub kind: String,
    pub state: TaskState,
    pub retention: RetentionState,
    pub request_digest: Option<String>,
    pub response_digest: Option<String>,
    pub bytes: u64,
    pub cost_micros: u64,
    pub attempt: u32,
    pub at: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiTaskRecord {
    pub receipt: ApiTaskReceipt,
    pub state: TaskState,
    pub retention: RetentionState,
    pub used_cost_micros: u64,
    pub attempts: u32,
    pub artifact_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteOcrResult {
    pub protocol: String,
    pub protocol_version: String,
    pub task_id: String,
    pub source_sha256: String,
    pub result_digest: String,
    pub raw_artifact: Vec<u8>,
    pub pages: Vec<crate::ocr::OcrPage>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    // Structs use declaration order; BTreeMap fields are sorted.  This is
    // intentionally compact and stable for the frozen 0.1 records.
    serde_json::to_vec(value).map_err(|e| RemoteApiError::Invalid(e.to_string()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_text(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_FIELD_BYTES || value.chars().any(|c| c.is_control()) {
        return Err(RemoteApiError::Invalid(format!(
            "{label} is empty, oversized, or contains control characters"
        )));
    }
    Ok(())
}
pub fn validate_origin(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    if !matches!(url.path(), "" | "/") || url.host().is_none() {
        return false;
    }
    match url.scheme() {
        "https" => true,
        "http" => {
            matches!(url.host(), Some(Host::Ipv4(ip)) if ip.octets()[0] == 127)
                || matches!(url.host(), Some(Host::Ipv6(ip)) if ip.is_loopback())
        }
        _ => false,
    }
}

impl ApiPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        source_sha256: impl Into<String>,
        source_bytes: u64,
        page_count: u32,
        max_cost_micros: u64,
        currency: impl Into<String>,
        retention: Retention,
    ) -> Result<Self> {
        let mut plan = Self {
            schema: PLAN_SCHEMA.into(),
            schema_version: PLAN_VERSION.into(),
            origin: origin.into(),
            operation: RemoteOperation::Ocr,
            provider: provider.into(),
            model: model.into(),
            source_sha256: source_sha256.into(),
            source_bytes,
            page_count,
            retention,
            max_cost_micros,
            currency: currency.into(),
            plan_digest: String::new(),
        };
        plan.validate_without_digest()?;
        let digest_input = canonical(&plan_without_digest(&plan))?;
        plan.plan_digest = sha256_hex(&digest_input);
        Ok(plan)
    }
    fn validate_without_digest(&self) -> Result<()> {
        if self.schema != PLAN_SCHEMA || self.schema_version != PLAN_VERSION {
            return Err(RemoteApiError::UnsupportedProtocol(format!(
                "{} {}",
                self.schema, self.schema_version
            )));
        }
        valid_text(&self.origin, "origin")?;
        valid_text(&self.provider, "provider")?;
        valid_text(&self.model, "model")?;
        valid_text(&self.currency, "currency")?;
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(RemoteApiError::Invalid(
                "currency must be a three-letter uppercase code".into(),
            ));
        }
        if !validate_origin(&self.origin) {
            return Err(RemoteApiError::Invalid(
                "origin must be an HTTPS origin without credentials or fragment".into(),
            ));
        }
        if !valid_digest(&self.source_sha256) || self.source_bytes == 0 || self.page_count == 0 {
            return Err(RemoteApiError::Invalid(
                "source digest, byte length, and page count are invalid".into(),
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_without_digest()?;
        let expected = sha256_hex(&canonical(&plan_without_digest(self))?);
        if self.plan_digest != expected {
            return Err(RemoteApiError::Invalid("plan digest mismatch".into()));
        }
        Ok(())
    }
    pub fn canonical_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical(self)
    }
    pub fn consent(&self, at: i64) -> Result<Consent> {
        self.validate()?;
        if at < 0 {
            return Err(RemoteApiError::Invalid(
                "consent timestamp is invalid".into(),
            ));
        }
        Ok(Consent {
            plan_digest: self.plan_digest.clone(),
            consented_at: at,
        })
    }
    pub fn request_id(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256_hex(
            format!(
                "{}\n{}\n{}",
                API_PROTOCOL_VERSION, self.plan_digest, self.source_sha256
            )
            .as_bytes(),
        ))
    }
    pub fn idempotency_key(&self, action: &str) -> Result<String> {
        valid_text(action, "action")?;
        Ok(sha256_hex(
            format!("{}\n{}\n{}", self.request_id()?, action, self.plan_digest).as_bytes(),
        ))
    }
}
fn plan_without_digest(plan: &ApiPlan) -> impl Serialize + '_ {
    (
        &plan.schema,
        &plan.schema_version,
        &plan.origin,
        &plan.operation,
        &plan.provider,
        &plan.model,
        &plan.source_sha256,
        &plan.source_bytes,
        &plan.page_count,
        &plan.retention,
        &plan.max_cost_micros,
        &plan.currency,
    )
}

impl ApiTaskReceipt {
    pub fn from_plan(plan: &ApiPlan, task_id: impl Into<String>) -> Result<Self> {
        plan.validate()?;
        let mut receipt = Self {
            schema: RECEIPT_SCHEMA.into(),
            schema_version: RECEIPT_VERSION.into(),
            protocol: API_PROTOCOL.into(),
            protocol_version: API_PROTOCOL_VERSION.into(),
            origin: plan.origin.clone(),
            task_id: task_id.into(),
            request_id: plan.request_id()?,
            operation: plan.operation,
            source_sha256: plan.source_sha256.clone(),
            source_bytes: plan.source_bytes,
            page_count: plan.page_count,
            plan_digest: plan.plan_digest.clone(),
            provider: plan.provider.clone(),
            model: plan.model.clone(),
            max_cost_micros: plan.max_cost_micros,
            currency: plan.currency.clone(),
            retention: plan.retention,
            receipt_digest: String::new(),
        };
        receipt.validate_without_digest()?;
        let digest_input = canonical(&receipt_without_digest(&receipt))?;
        receipt.receipt_digest = sha256_hex(&digest_input);
        Ok(receipt)
    }
    fn validate_without_digest(&self) -> Result<()> {
        if self.schema != RECEIPT_SCHEMA
            || self.schema_version != RECEIPT_VERSION
            || self.protocol != API_PROTOCOL
            || self.protocol_version != API_PROTOCOL_VERSION
        {
            return Err(RemoteApiError::UnsupportedProtocol(
                "receipt protocol".into(),
            ));
        }
        valid_text(&self.origin, "origin")?;
        valid_text(&self.task_id, "task_id")?;
        if self.task_id.len() > MAX_TASK_ID_BYTES
            || !self
                .task_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(RemoteApiError::Invalid("receipt task_id is invalid".into()));
        }
        if !validate_origin(&self.origin) {
            return Err(RemoteApiError::Invalid("receipt origin is invalid".into()));
        }
        if !valid_digest(&self.source_sha256)
            || !valid_digest(&self.request_id)
            || !valid_digest(&self.plan_digest)
            || self.source_bytes == 0
            || self.page_count == 0
        {
            return Err(RemoteApiError::Invalid(
                "receipt digest fields are invalid".into(),
            ));
        }
        Ok(())
    }
    pub fn matches_plan(&self, plan: &ApiPlan) -> Result<()> {
        self.validate()?;
        plan.validate()?;
        if self.origin != plan.origin
            || self.operation != plan.operation
            || self.source_sha256 != plan.source_sha256
            || self.source_bytes != plan.source_bytes
            || self.page_count != plan.page_count
            || self.plan_digest != plan.plan_digest
            || self.provider != plan.provider
            || self.model != plan.model
            || self.max_cost_micros != plan.max_cost_micros
            || self.currency != plan.currency
            || self.retention != plan.retention
        {
            return Err(RemoteApiError::Invalid(
                "receipt is not bound to plan".into(),
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_without_digest()?;
        if self.receipt_digest != sha256_hex(&canonical(&receipt_without_digest(self))?) {
            return Err(RemoteApiError::Invalid("receipt digest mismatch".into()));
        }
        Ok(())
    }
    pub fn canonical_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical(self)
    }
}
fn receipt_without_digest(r: &ApiTaskReceipt) -> impl Serialize + '_ {
    ReceiptDigestInput {
        schema: &r.schema,
        schema_version: &r.schema_version,
        protocol: &r.protocol,
        protocol_version: &r.protocol_version,
        origin: &r.origin,
        task_id: &r.task_id,
        request_id: &r.request_id,
        operation: r.operation,
        source_sha256: &r.source_sha256,
        source_bytes: r.source_bytes,
        page_count: r.page_count,
        plan_digest: &r.plan_digest,
        provider: &r.provider,
        model: &r.model,
        max_cost_micros: r.max_cost_micros,
        currency: &r.currency,
        retention: r.retention,
    }
}

#[derive(Serialize)]
struct ReceiptDigestInput<'a> {
    schema: &'a str,
    schema_version: &'a str,
    protocol: &'a str,
    protocol_version: &'a str,
    origin: &'a str,
    task_id: &'a str,
    request_id: &'a str,
    operation: RemoteOperation,
    source_sha256: &'a str,
    source_bytes: u64,
    page_count: u32,
    plan_digest: &'a str,
    provider: &'a str,
    model: &'a str,
    max_cost_micros: u64,
    currency: &'a str,
    retention: Retention,
}

pub fn validate_consent(plan: &ApiPlan, consent: &Consent) -> Result<()> {
    plan.validate()?;
    if consent.plan_digest != plan.plan_digest
        || consent.consented_at < 0
        || consent.consented_at > now_unix_seconds().saturating_add(300)
    {
        return Err(RemoteApiError::ConsentRequired);
    }
    Ok(())
}

impl RemoteOcrResult {
    /// Verifies all bindings before any page is admitted to the MDP OCR
    /// extension. Partial, stale, unknown-protocol, and corrupt results fail
    /// closed.
    pub fn validate(&self, expected_task: &str, expected_source: &str) -> Result<()> {
        if self.protocol != API_PROTOCOL
            || self.protocol_version != API_PROTOCOL_VERSION
            || self.task_id != expected_task
            || self.source_sha256 != expected_source
            || !valid_digest(&self.result_digest)
            || self.raw_artifact.len() as u64 > MAX_ARTIFACT_BYTES
            || sha256_hex(&self.raw_artifact) != self.result_digest
        {
            return Err(RemoteApiError::Invalid(
                "remote OCR result binding, protocol, or digest is invalid".into(),
            ));
        }
        let run = crate::ocr::OcrRun {
            protocol: crate::ocr::OCR_PROTOCOL.into(),
            protocol_version: crate::ocr::OCR_PROTOCOL_VERSION.into(),
            pages: self.pages.clone(),
            errors: Vec::new(),
        };
        run.validate()
            .map_err(|e| RemoteApiError::Invalid(e.to_string()))
    }
}

/// Installs a verified remote response before exposing its typed pages to the
/// existing MDP OCR extension. The provider artifact is retained separately,
/// while page records keep their existing source/revision semantics.
pub fn install_remote_ocr_result(package_root: &Path, result: &RemoteOcrResult) -> Result<PathBuf> {
    result.validate(&result.task_id, &result.source_sha256)?;
    let package = crate::document_package::DocumentPackage::read_from(package_root)
        .map_err(|error| RemoteApiError::Invalid(format!("invalid MDP package: {error}")))?;
    if package.source.content_sha256 != result.source_sha256
        || package.source.byte_len == 0
        || package.source.page_count != result.pages.len() as u32
        || package.source.page_count == 0
    {
        return Err(RemoteApiError::Invalid(
            "remote result does not match MDP source".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if result
        .pages
        .iter()
        .any(|p| !seen.insert(p.page_index) || p.page_index >= package.source.page_count)
    {
        return Err(RemoteApiError::Invalid(
            "remote OCR pages are incomplete or duplicated".into(),
        ));
    }
    let artifact_dir = package_root.join("ocr").join("raw-artifacts");
    let path = install_artifact(&artifact_dir, &result.result_digest, &result.raw_artifact)?;
    let run = crate::ocr::OcrRun {
        protocol: crate::ocr::OCR_PROTOCOL.into(),
        protocol_version: crate::ocr::OCR_PROTOCOL_VERSION.into(),
        pages: result.pages.clone(),
        errors: Vec::new(),
    };
    crate::ocr::write_ocr_records(package_root, &run)
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    Ok(path)
}

pub fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

/// Installs a bounded provider artifact with same-directory temp + sync +
/// digest verification + create-new persistence. Existing content never gets
/// overwritten, even when it has the same name.
pub fn install_artifact(root: &Path, digest: &str, bytes: &[u8]) -> Result<PathBuf> {
    if !valid_digest(digest)
        || bytes.len() as u64 > MAX_ARTIFACT_BYTES
        || sha256_hex(bytes) != digest
    {
        return Err(RemoteApiError::Artifact(
            "artifact digest or size mismatch".into(),
        ));
    }
    ensure_safe_directory(root)?;
    let path = root.join(format!("{digest}.artifact"));
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(RemoteApiError::Artifact("artifact path is unsafe".into()));
        }
        let mut existing = Vec::new();
        OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| RemoteApiError::Artifact(e.to_string()))?
            .take(MAX_ARTIFACT_BYTES + 1)
            .read_to_end(&mut existing)
            .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
        if existing != bytes {
            return Err(RemoteApiError::Artifact("artifact collision".into()));
        }
        return Ok(path);
    }
    let mut temp = tempfile::Builder::new()
        .prefix(".mpdf-api-")
        .suffix(".partial")
        .tempfile_in(root)
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    temp.write_all(bytes)
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    temp.flush()
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    match temp.persist_noclobber(&path) {
        Ok(_) => Ok(path),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = fs::symlink_metadata(&path)
                .map_err(|error| RemoteApiError::Artifact(error.to_string()))?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(RemoteApiError::Artifact("artifact path is unsafe".into()));
            }
            let mut existing = Vec::new();
            OpenOptions::new()
                .read(true)
                .open(&path)
                .map_err(|error| RemoteApiError::Artifact(error.to_string()))?
                .take(MAX_ARTIFACT_BYTES + 1)
                .read_to_end(&mut existing)
                .map_err(|error| RemoteApiError::Artifact(error.to_string()))?;
            if existing != bytes {
                return Err(RemoteApiError::Artifact("artifact collision".into()));
            }
            Ok(path)
        }
        Err(e) => Err(RemoteApiError::Artifact(e.error.to_string())),
    }
}

fn ensure_safe_directory(root: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut cursor = root;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor
            .parent()
            .ok_or_else(|| RemoteApiError::Artifact("invalid artifact parent".into()))?;
    }
    if fs::symlink_metadata(cursor)
        .map_err(|e| RemoteApiError::Artifact(e.to_string()))?
        .file_type()
        .is_symlink()
    {
        return Err(RemoteApiError::Artifact(
            "artifact parent is a symlink".into(),
        ));
    }
    fs::create_dir_all(root).map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
    for path in missing.into_iter().rev() {
        let metadata =
            fs::symlink_metadata(path).map_err(|e| RemoteApiError::Artifact(e.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RemoteApiError::Artifact("artifact parent is unsafe".into()));
        }
    }
    Ok(())
}

fn validate_audit_event(event: &ApiAuditEvent) -> Result<()> {
    if event.schema != AUDIT_SCHEMA
        || event.schema_version != AUDIT_VERSION
        || event.event_id.is_empty()
        || event.event_id.len() > MAX_TASK_ID_BYTES
        || event.task_id.is_empty()
        || event.task_id.len() > MAX_TASK_ID_BYTES
        || !event
            .event_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !event
            .task_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !matches!(
            event.kind.as_str(),
            "create"
                | "upload"
                | "start"
                | "status"
                | "result"
                | "cancel"
                | "delete_content"
                | "retry"
                | "fallback"
                | "state"
                | "retention"
                | "planned"
        )
        || event.at < 0
        || event.at > now_unix_seconds().saturating_add(300)
        || event.attempt > 3
        || event
            .request_digest
            .as_deref()
            .is_some_and(|d| !valid_digest(d))
        || event
            .response_digest
            .as_deref()
            .is_some_and(|d| !valid_digest(d))
        || event
            .message
            .as_deref()
            .is_some_and(|m| m.len() > MAX_AUDIT_MESSAGE_BYTES || m.chars().any(|c| c.is_control()))
    {
        return Err(RemoteApiError::Invalid("invalid audit event".into()));
    }
    Ok(())
}

pub struct ApiStore {
    connection: Connection,
}
impl ApiStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| RemoteApiError::Invalid(e.to_string()))?;
        }
        let c = Connection::open(path)?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v > 3 {
            return Err(RemoteApiError::Invalid(format!(
                "unsupported API schema version {v}"
            )));
        }
        c.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS api_tasks (task_id TEXT PRIMARY KEY, receipt_json TEXT NOT NULL, state TEXT NOT NULL, retention TEXT NOT NULL, used_cost_micros INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0, artifact_digest TEXT); CREATE TABLE IF NOT EXISTS api_audit (event_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, event_json TEXT NOT NULL); CREATE INDEX IF NOT EXISTS api_audit_task ON api_audit(task_id); PRAGMA user_version=3; COMMIT;")?;
        Ok(Self { connection: c })
    }
    pub fn put_task(&self, task: &ApiTaskRecord) -> Result<()> {
        task.receipt.validate()?;
        if task.attempts > 3 || task.used_cost_micros > task.receipt.max_cost_micros {
            return Err(RemoteApiError::Invalid(
                "task record exceeds policy limits".into(),
            ));
        }
        let json = serde_json::to_string(&task.receipt)
            .map_err(|e| RemoteApiError::Invalid(e.to_string()))?;
        let state = serde_json::to_string(&task.state)
            .map_err(|e| RemoteApiError::Invalid(e.to_string()))?;
        let retention = serde_json::to_string(&task.retention)
            .map_err(|e| RemoteApiError::Invalid(e.to_string()))?;
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("INSERT INTO api_tasks(task_id,receipt_json,state,retention,used_cost_micros,attempts,artifact_digest) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(task_id) DO UPDATE SET receipt_json=excluded.receipt_json,state=excluded.state,retention=excluded.retention,used_cost_micros=excluded.used_cost_micros,attempts=excluded.attempts,artifact_digest=excluded.artifact_digest", params![task.receipt.task_id,json,state.trim_matches('"'),retention.trim_matches('"'),task.used_cost_micros,task.attempts,task.artifact_digest])?;
        tx.commit()?;
        Ok(())
    }
    pub fn transition(&self, task_id: &str, next: TaskState) -> Result<ApiTaskRecord> {
        let mut task = self
            .task(task_id)?
            .ok_or_else(|| RemoteApiError::Invalid("task does not exist".into()))?;
        if !valid_transition(task.state, next) {
            return Err(RemoteApiError::Invalid(format!(
                "invalid task transition {:?} -> {:?}",
                task.state, next
            )));
        }
        task.state = next;
        self.put_task(&task)?;
        Ok(task)
    }
    pub fn record_cost(&self, task_id: &str, cost_micros: u64) -> Result<ApiTaskRecord> {
        let mut task = self
            .task(task_id)?
            .ok_or_else(|| RemoteApiError::Invalid("task does not exist".into()))?;
        let used = task
            .used_cost_micros
            .checked_add(cost_micros)
            .ok_or(RemoteApiError::BudgetExceeded)?;
        if used > task.receipt.max_cost_micros {
            task.state = TaskState::PausedBudget;
            self.put_task(&task)?;
            return Err(RemoteApiError::BudgetExceeded);
        }
        task.used_cost_micros = used;
        self.put_task(&task)?;
        Ok(task)
    }
    pub fn task(&self, task_id: &str) -> Result<Option<ApiTaskRecord>> {
        self.connection.query_row("SELECT receipt_json,state,retention,used_cost_micros,attempts,artifact_digest FROM api_tasks WHERE task_id=?1", params![task_id], |r| { let receipt: ApiTaskReceipt = serde_json::from_str(&r.get::<_,String>(0)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0,rusqlite::types::Type::Text,Box::new(e)))?; let state: TaskState = serde_json::from_str(&format!("\"{}\"",r.get::<_,String>(1)?)).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Text,Box::new(e)))?; let retention: RetentionState = serde_json::from_str(&format!("\"{}\"",r.get::<_,String>(2)?)).map_err(|e| rusqlite::Error::FromSqlConversionFailure(2,rusqlite::types::Type::Text,Box::new(e)))?; Ok(ApiTaskRecord { receipt,state,retention,used_cost_micros:r.get(3)?,attempts:r.get(4)?,artifact_digest:r.get(5)? }) }).optional().map_err(Into::into)
    }
    pub fn append_audit(&self, event: &ApiAuditEvent) -> Result<()> {
        validate_audit_event(event)?;
        let json =
            serde_json::to_string(event).map_err(|e| RemoteApiError::Invalid(e.to_string()))?;
        let tx = self.connection.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO api_audit(event_id,task_id,event_json) VALUES(?1,?2,?3)",
            params![event.event_id, event.task_id, json],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn audit(&self, task_id: &str) -> Result<Vec<ApiAuditEvent>> {
        let mut s = self
            .connection
            .prepare("SELECT event_json FROM api_audit WHERE task_id=?1 ORDER BY rowid")?;
        let rows = s.query_map(params![task_id], |r| {
            serde_json::from_str(&r.get::<_, String>(0)?).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn valid_transition(from: TaskState, to: TaskState) -> bool {
    use TaskState::*;
    matches!(
        (from, to),
        (Planned, Consented)
            | (Consented, Creating)
            | (Creating, UploadPending)
            | (Creating, Ready)
            | (UploadPending, Ready)
            | (Ready, Running)
            | (Running, Completed)
            | (Completed, ResultInstalled)
            | (Running, Cancelling)
            | (Cancelling, Cancelled)
            | (PausedBudget, Cancelling)
            | (PausedService, Cancelling)
            | (Running, PausedBudget)
            | (Running, PausedService)
            | (PausedBudget, Running)
            | (PausedService, Running)
            | (Ready, Cancelling)
            | (Creating, Failed)
            | (UploadPending, Failed)
            | (Running, Failed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> ApiPlan {
        ApiPlan::new(
            "https://api.example.test",
            "fixture",
            "ocr-1",
            "a".repeat(64),
            12,
            1,
            10_000,
            "USD",
            Retention::DeleteAfterResult,
        )
        .unwrap()
    }
    #[test]
    fn deterministic_plan_and_ids() {
        let a = plan();
        let b = plan();
        assert_eq!(a.plan_digest, b.plan_digest);
        assert_eq!(a.request_id().unwrap(), b.request_id().unwrap());
        assert_eq!(
            a.idempotency_key("create").unwrap(),
            b.idempotency_key("create").unwrap()
        );
    }
    #[test]
    fn consent_changes_on_policy_fields() {
        let a = plan();
        let b = ApiPlan::new(
            "https://api.example.test",
            "fixture",
            "ocr-1",
            "a".repeat(64),
            12,
            1,
            11_000,
            "USD",
            Retention::DeleteAfterResult,
        )
        .unwrap();
        assert_ne!(a.plan_digest, b.plan_digest);
    }
    #[test]
    fn artifact_is_verified_and_no_clobber() {
        let d = tempfile::tempdir().unwrap();
        let b = b"fixture";
        let h = sha256_hex(b);
        let p = install_artifact(d.path(), &h, b).unwrap();
        assert_eq!(install_artifact(d.path(), &h, b).unwrap(), p);
        assert!(install_artifact(d.path(), &h, b"other").is_err());
    }
    #[test]
    fn sqlite_store_is_append_only_audit() {
        let d = tempfile::tempdir().unwrap();
        let s = ApiStore::open(&d.path().join("jobs.sqlite")).unwrap();
        let p = plan();
        let r = ApiTaskReceipt::from_plan(&p, "task").unwrap();
        s.put_task(&ApiTaskRecord {
            receipt: r.clone(),
            state: TaskState::Planned,
            retention: RetentionState::NotRequested,
            used_cost_micros: 0,
            attempts: 0,
            artifact_digest: None,
        })
        .unwrap();
        s.append_audit(&ApiAuditEvent {
            schema: AUDIT_SCHEMA.into(),
            schema_version: AUDIT_VERSION.into(),
            event_id: "e1".into(),
            task_id: "task".into(),
            kind: "planned".into(),
            state: TaskState::Planned,
            retention: RetentionState::NotRequested,
            request_digest: None,
            response_digest: None,
            bytes: 0,
            cost_micros: 0,
            attempt: 0,
            at: 0,
            message: None,
        })
        .unwrap();
        assert_eq!(s.audit("task").unwrap().len(), 1);
    }
    #[test]
    fn api_and_job_store_share_v3_database_in_either_order() {
        for api_first in [true, false] {
            let d = tempfile::tempdir().unwrap();
            let db = d.path().join("jobs.sqlite");
            if api_first {
                ApiStore::open(&db).unwrap();
                crate::jobs::JobStore::open(&db).unwrap();
            } else {
                crate::jobs::JobStore::open(&db).unwrap();
                ApiStore::open(&db).unwrap();
            }
            let jobs = crate::jobs::JobStore::open(&db).unwrap();
            jobs.create_job("resume", 1).unwrap();
            assert!(ApiStore::open(&db)
                .unwrap()
                .task("missing")
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn budget_overrun_pauses_without_reporting_success() {
        let d = tempfile::tempdir().unwrap();
        let s = ApiStore::open(&d.path().join("jobs.sqlite")).unwrap();
        let p = plan();
        let r = ApiTaskReceipt::from_plan(&p, "budget-task").unwrap();
        s.put_task(&ApiTaskRecord {
            receipt: r,
            state: TaskState::Running,
            retention: RetentionState::NotRequested,
            used_cost_micros: 0,
            attempts: 1,
            artifact_digest: None,
        })
        .unwrap();
        assert!(matches!(
            s.record_cost("budget-task", 10_001),
            Err(RemoteApiError::BudgetExceeded)
        ));
        assert_eq!(
            s.task("budget-task").unwrap().unwrap().state,
            TaskState::PausedBudget
        );
    }
    #[test]
    fn unknown_plan_fields_are_rejected() {
        let mut value = serde_json::to_value(plan()).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ApiPlan>(value).is_err());
    }
}
