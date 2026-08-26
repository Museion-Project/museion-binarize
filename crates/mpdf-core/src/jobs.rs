//! Persistent local job orchestration and the provider-neutral M2 protocol.
//!
//! This module deliberately contains no OCR implementation. Providers receive
//! an asset digest and return typed status/provenance; the durable store is the
//! source of truth for recovery and cancellation.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const JOB_PROTOCOL: &str = "mpdf-job";
pub const JOB_PROTOCOL_VERSION: &str = "0.1";
pub const DEFAULT_LEASE_SECONDS: i64 = 60;
pub const MAX_PAGES_PER_JOB: u32 = 100_000;
pub const MAX_PAGE_ATTEMPTS: u32 = 3;
pub const MAX_SIDECAR_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SIDECAR_RECORDS: usize = 100_000;
pub const MAX_SIDECAR_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_PAYLOAD_ITEMS: usize = 128;
pub const MAX_PARAMETER_BYTES: usize = 4096;
pub const MAX_CHECKPOINT_BYTES: usize = 1024 * 1024;
pub const MAX_IDENTIFIER_BYTES: usize = 256;
pub const MAX_LEASE_SECONDS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: String,
    pub status: JobStatus,
    pub page_count: u32,
    pub completed_pages: u32,
    pub cancel_requested: bool,
    pub heartbeat_at: Option<i64>,
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageRecord {
    pub job_id: String,
    pub page_index: u32,
    pub status: PageStatus,
    pub attempts: u32,
    pub checkpoint: Option<String>,
    pub artifact_digest: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRunRecord {
    pub run_id: i64,
    pub job_id: String,
    pub page_index: u32,
    pub engine: String,
    pub model: String,
    pub version: String,
    pub parameters: BTreeMap<String, String>,
    pub input_asset_sha256: String,
    pub output_digest: Option<String>,
    pub execution_location: ExecutionLocation,
    pub outcome: ProviderOutcome,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobProgress {
    pub job_id: String,
    pub status: JobStatus,
    pub page_count: u32,
    pub completed_pages: u32,
    pub failed_pages: u32,
    pub cancelled_pages: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("job storage error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid job: {0}")]
    Invalid(String),
    #[error("unsupported job protocol: {0}")]
    UnsupportedProtocol(String),
    #[error("provider error: {0}")]
    Provider(String),
}
pub type JobResult<T> = Result<T, JobError>;

pub struct JobStore {
    connection: Connection,
}

impl JobStore {
    pub fn open(path: &Path) -> JobResult<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                JobError::Invalid(format!("cannot create job directory: {error}"))
            })?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let schema_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if schema_version > 1 {
            return Err(JobError::Invalid(format!(
                "unsupported job schema version: {schema_version}"
            )));
        }
        if schema_version == 0 {
            connection.pragma_update(None, "user_version", 1_i64)?;
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
               job_id TEXT PRIMARY KEY, status TEXT NOT NULL, page_count INTEGER NOT NULL,
               completed_pages INTEGER NOT NULL DEFAULT 0, cancel_requested INTEGER NOT NULL DEFAULT 0,
               heartbeat_at INTEGER, lease_owner TEXT, lease_until INTEGER, retries INTEGER NOT NULL DEFAULT 0,
               last_error TEXT
             );
             CREATE TABLE IF NOT EXISTS pages (
               job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
               page_index INTEGER NOT NULL, status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
               checkpoint TEXT, artifact_digest TEXT, error TEXT, lease_owner TEXT, lease_until INTEGER,
               PRIMARY KEY(job_id, page_index)
             );
             CREATE INDEX IF NOT EXISTS pages_claim ON pages(job_id, status, lease_until);",
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_runs (
               run_id INTEGER PRIMARY KEY AUTOINCREMENT,
               job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
               page_index INTEGER NOT NULL,
               engine TEXT NOT NULL, model TEXT NOT NULL, version TEXT NOT NULL,
               parameters_json TEXT NOT NULL, input_asset_sha256 TEXT NOT NULL,
               output_digest TEXT, execution_location TEXT NOT NULL,
               outcome TEXT NOT NULL, error TEXT, started_at INTEGER NOT NULL,
               finished_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS provider_runs_page ON provider_runs(job_id, page_index);",
        )?;
        Ok(Self { connection })
    }

    pub fn create_job(&self, job_id: &str, page_count: u32) -> JobResult<JobRecord> {
        if !valid_identifier(job_id) || page_count == 0 || page_count > MAX_PAGES_PER_JOB {
            return Err(JobError::Invalid("job ID/page count out of range".into()));
        }
        let tx = self.connection.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO jobs(job_id,status,page_count) VALUES(?1,'queued',?2)",
            params![job_id, page_count],
        )?;
        for index in 0..page_count {
            tx.execute(
                "INSERT INTO pages(job_id,page_index,status) VALUES(?1,?2,'queued')",
                params![job_id, index],
            )?;
        }
        tx.commit()?;
        self.job(job_id)?
            .ok_or_else(|| JobError::Invalid("created job disappeared".into()))
    }

    pub fn job(&self, job_id: &str) -> JobResult<Option<JobRecord>> {
        self.connection.query_row("SELECT job_id,status,page_count,completed_pages,cancel_requested,heartbeat_at,retries FROM jobs WHERE job_id=?1", params![job_id], row_job).optional().map_err(Into::into)
    }
    pub fn page(&self, job_id: &str, page_index: u32) -> JobResult<Option<PageRecord>> {
        self.connection.query_row("SELECT job_id,page_index,status,attempts,checkpoint,artifact_digest,error FROM pages WHERE job_id=?1 AND page_index=?2", params![job_id,page_index], row_page).optional().map_err(Into::into)
    }

    pub fn provider_runs(&self, job_id: &str) -> JobResult<Vec<ProviderRunRecord>> {
        let mut statement = self.connection.prepare("SELECT run_id,job_id,page_index,engine,model,version,parameters_json,input_asset_sha256,output_digest,execution_location,outcome,error,started_at,finished_at FROM provider_runs WHERE job_id=?1 ORDER BY run_id")?;
        let rows = statement.query_map(params![job_id], row_provider_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn progress(&self, job_id: &str) -> JobResult<Option<JobProgress>> {
        let Some(job) = self.job(job_id)? else {
            return Ok(None);
        };
        let (failed_pages, cancelled_pages): (u32, u32) = self.connection.query_row(
            "SELECT COALESCE(SUM(status='failed'),0), COALESCE(SUM(status='cancelled'),0) FROM pages WHERE job_id=?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(Some(JobProgress {
            job_id: job.job_id,
            status: job.status,
            page_count: job.page_count,
            completed_pages: job.completed_pages,
            failed_pages,
            cancelled_pages,
        }))
    }

    pub fn claim_page(
        &self,
        job_id: &str,
        owner: &str,
        now: i64,
        lease_seconds: i64,
    ) -> JobResult<Option<PageRecord>> {
        if !valid_identifier(owner)
            || now < 0
            || lease_seconds <= 0
            || lease_seconds > MAX_LEASE_SECONDS
        {
            return Err(JobError::Invalid("owner/lease out of range".into()));
        }
        let lease_until = now
            .checked_add(lease_seconds)
            .ok_or_else(|| JobError::Invalid("lease timestamp overflow".into()))?;
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> JobResult<Option<u32>> {
            let job_state: Option<(String, i64)> = self
                .connection
                .query_row(
                    "SELECT status,cancel_requested FROM jobs WHERE job_id=?1",
                    params![job_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((status, cancel_requested)) = job_state else {
                return Err(JobError::Invalid("job does not exist".into()));
            };
            if cancel_requested != 0
                || matches!(status.as_str(), "completed" | "failed" | "cancelled")
            {
                return Ok(None);
            }
            let active_owner: Option<String> = self
                .connection
                .query_row(
                    "SELECT lease_owner FROM pages WHERE job_id=?1 AND status='running' AND lease_until>?2 LIMIT 1",
                    params![job_id, now],
                    |row| row.get(0),
                )
                .optional()?;
            if active_owner
                .as_deref()
                .is_some_and(|active| active != owner)
            {
                return Err(JobError::Invalid(
                    "job is already leased by another worker".into(),
                ));
            }
            let found: Option<u32> = self.connection.query_row("SELECT page_index FROM pages WHERE job_id=?1 AND (status='queued' OR (status='running' AND lease_until<=?2)) ORDER BY page_index LIMIT 1", params![job_id,now], |row| row.get(0)).optional()?;
            let Some(index) = found else {
                return Ok(None);
            };
            let changed = self.connection.execute("UPDATE pages SET status='running',attempts=attempts+1,lease_owner=?3,lease_until=?2 WHERE job_id=?1 AND page_index=?4 AND (status='queued' OR (status='running' AND lease_until<=?5))", params![job_id,lease_until,owner,index,now])?;
            if changed != 1 {
                return Ok(None);
            }
            self.connection.execute("UPDATE jobs SET status='running',heartbeat_at=?2,lease_owner=?3,lease_until=?4 WHERE job_id=?1", params![job_id,now,owner,lease_until])?;
            Ok(Some(index))
        })();
        match result {
            Ok(index) => {
                self.connection.execute_batch("COMMIT")?;
                match index {
                    Some(index) => self.page(job_id, index),
                    None => Ok(None),
                }
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn heartbeat(
        &self,
        job_id: &str,
        owner: &str,
        now: i64,
        lease_seconds: i64,
    ) -> JobResult<()> {
        if !valid_identifier(owner)
            || now < 0
            || lease_seconds <= 0
            || lease_seconds > MAX_LEASE_SECONDS
        {
            return Err(JobError::Invalid("owner/lease out of range".into()));
        }
        let lease_until = now
            .checked_add(lease_seconds)
            .ok_or_else(|| JobError::Invalid("lease timestamp overflow".into()))?;
        let tx = self.connection.unchecked_transaction()?;
        let changed = tx.execute("UPDATE jobs SET heartbeat_at=?3,lease_until=?4 WHERE job_id=?1 AND lease_owner=?2 AND status IN ('running','cancelling') AND lease_until>?3", params![job_id,owner,now,lease_until])?;
        if changed == 0 {
            return Err(JobError::Invalid("job lease is not owned".into()));
        }
        tx.execute("UPDATE pages SET lease_until=?4 WHERE job_id=?1 AND lease_owner=?2 AND status='running' AND lease_until>?3", params![job_id,owner,now,lease_until])?;
        tx.commit()?;
        Ok(())
    }

    pub fn checkpoint_page(
        &self,
        job_id: &str,
        page_index: u32,
        owner: &str,
        checkpoint: &str,
        artifact_digest: &str,
        now: i64,
    ) -> JobResult<()> {
        if now < 0
            || !valid_identifier(owner)
            || checkpoint.is_empty()
            || checkpoint.len() > MAX_CHECKPOINT_BYTES
            || !is_sha256_hex(artifact_digest)
        {
            return Err(JobError::Invalid("checkpoint/digest invalid".into()));
        }
        let tx = self.connection.unchecked_transaction()?;
        let changed = tx.execute("UPDATE pages SET status='completed',checkpoint=?4,artifact_digest=?5,lease_owner=NULL,lease_until=NULL,error=NULL WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3 AND lease_until>?6", params![job_id,page_index,owner,checkpoint,artifact_digest,now])?;
        if changed == 0 {
            return Err(JobError::Invalid("page checkpoint is not owned".into()));
        }
        tx.execute("UPDATE jobs SET completed_pages=(SELECT COUNT(*) FROM pages WHERE job_id=?1 AND status='completed'),heartbeat_at=?2 WHERE job_id=?1", params![job_id,now])?;
        let remaining: i64 = tx.query_row("SELECT COUNT(*) FROM pages WHERE job_id=?1 AND status NOT IN ('completed','cancelled')", params![job_id], |row| row.get(0))?;
        let cancelled: i64 = tx.query_row(
            "SELECT cancel_requested FROM jobs WHERE job_id=?1",
            params![job_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            tx.execute(
                "UPDATE jobs SET status=?2 WHERE job_id=?1",
                params![
                    job_id,
                    if cancelled != 0 {
                        "cancelled"
                    } else {
                        "completed"
                    }
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_provider_success_and_checkpoint(
        &self,
        job_id: &str,
        page_index: u32,
        owner: &str,
        checkpoint: &str,
        response: &ProviderResponse,
        started_at: i64,
        finished_at: i64,
    ) -> JobResult<()> {
        validate_provider_response(response)?;
        if !valid_identifier(owner)
            || checkpoint.is_empty()
            || checkpoint.len() > MAX_CHECKPOINT_BYTES
            || started_at < 0
            || finished_at < 0
            || finished_at < started_at
        {
            return Err(JobError::Invalid("checkpoint/time is invalid".into()));
        }
        let tx = self.connection.unchecked_transaction()?;
        let changed = tx.execute("UPDATE pages SET status='completed',checkpoint=?4,artifact_digest=?5,lease_owner=NULL,lease_until=NULL,error=NULL WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3 AND lease_until>?6", params![job_id,page_index,owner,checkpoint,response.output_digest,finished_at])?;
        if changed == 0 {
            return Err(JobError::Invalid("page checkpoint is not owned".into()));
        }
        insert_provider_run(
            &tx,
            job_id,
            page_index,
            &response.provenance,
            Some(&response.output_digest),
            ProviderOutcome::Succeeded,
            None,
            started_at,
            finished_at,
        )?;
        finalize_page_transaction(&tx, job_id, finished_at)?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_provider_failure(
        &self,
        job_id: &str,
        page_index: u32,
        owner: &str,
        provenance: &ProviderProvenance,
        error: &str,
        retryable: bool,
        started_at: i64,
        finished_at: i64,
    ) -> JobResult<PageStatus> {
        validate_provenance(provenance)?;
        if !valid_identifier(owner)
            || error.is_empty()
            || error.len() > MAX_PARAMETER_BYTES
            || started_at < 0
            || finished_at < 0
            || finished_at < started_at
        {
            return Err(JobError::Invalid(
                "provider failure is out of bounds".into(),
            ));
        }
        let tx = self.connection.unchecked_transaction()?;
        let row: Option<(u32, i64)> = tx.query_row("SELECT attempts,(SELECT cancel_requested FROM jobs WHERE jobs.job_id=pages.job_id) FROM pages WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3 AND lease_until>?4", params![job_id,page_index,owner,finished_at], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        let Some((attempts, cancel_requested)) = row else {
            return Err(JobError::Invalid("page failure is not owned".into()));
        };
        let next = if cancel_requested != 0 {
            "cancelled"
        } else if retryable && attempts < MAX_PAGE_ATTEMPTS {
            "queued"
        } else {
            "failed"
        };
        tx.execute("UPDATE pages SET status=?4,error=?5,lease_owner=NULL,lease_until=NULL WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3 AND lease_until>?6", params![job_id,page_index,owner,next,error,finished_at])?;
        insert_provider_run(
            &tx,
            job_id,
            page_index,
            provenance,
            None,
            ProviderOutcome::Failed,
            Some(error),
            started_at,
            finished_at,
        )?;
        if next == "queued" {
            tx.execute(
                "UPDATE jobs SET retries=retries+1,heartbeat_at=?2,status='queued' WHERE job_id=?1",
                params![job_id, finished_at],
            )?;
        } else if next == "failed" {
            tx.execute(
                "UPDATE jobs SET status='failed',last_error=?2,heartbeat_at=?3 WHERE job_id=?1",
                params![job_id, error, finished_at],
            )?;
        } else {
            tx.execute(
                "UPDATE jobs SET heartbeat_at=?2,status='cancelled' WHERE job_id=?1",
                params![job_id, finished_at],
            )?;
        }
        tx.commit()?;
        parse_page(next.to_string()).map_err(|_| JobError::Invalid("invalid page state".into()))
    }

    /// Atomically records a provider failure. Retryable failures return the
    /// page to the queue until `MAX_PAGE_ATTEMPTS`; terminal failures mark the
    /// job failed. A cancellation always wins over another attempt.
    pub fn fail_page(
        &self,
        job_id: &str,
        page_index: u32,
        owner: &str,
        error: &str,
        retryable: bool,
        now: i64,
    ) -> JobResult<PageStatus> {
        if now < 0
            || !valid_identifier(owner)
            || error.is_empty()
            || error.len() > MAX_PARAMETER_BYTES
        {
            return Err(JobError::Invalid("owner/error is required".into()));
        }
        let tx = self.connection.unchecked_transaction()?;
        let row: Option<(u32, i64)> = tx.query_row(
            "SELECT attempts,(SELECT cancel_requested FROM jobs WHERE jobs.job_id=pages.job_id) FROM pages WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3 AND lease_until>?4",
            params![job_id, page_index, owner, now], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((attempts, cancel_requested)) = row else {
            return Err(JobError::Invalid("page failure is not owned".into()));
        };
        let next = if cancel_requested != 0 {
            "cancelled"
        } else if retryable && attempts < MAX_PAGE_ATTEMPTS {
            "queued"
        } else {
            "failed"
        };
        tx.execute("UPDATE pages SET status=?4,error=?5,lease_owner=NULL,lease_until=NULL WHERE job_id=?1 AND page_index=?2 AND status='running' AND lease_owner=?3", params![job_id,page_index,owner,next,error])?;
        if next == "queued" {
            tx.execute(
                "UPDATE jobs SET retries=retries+1,heartbeat_at=?2,status='queued' WHERE job_id=?1",
                params![job_id, now],
            )?;
        } else if next == "failed" {
            tx.execute(
                "UPDATE jobs SET status='failed',last_error=?2,heartbeat_at=?3 WHERE job_id=?1",
                params![job_id, error, now],
            )?;
        } else {
            tx.execute(
                "UPDATE jobs SET heartbeat_at=?2 WHERE job_id=?1",
                params![job_id, now],
            )?;
        }
        tx.commit()?;
        parse_page(next.to_string()).map_err(|_| JobError::Invalid("invalid page state".into()))
    }

    pub fn request_cancel(&self, job_id: &str) -> JobResult<()> {
        let tx = self.connection.unchecked_transaction()?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM jobs WHERE job_id=?1",
                params![job_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Err(JobError::Invalid("job does not exist".into()));
        };
        if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
            tx.commit()?;
            return Ok(());
        }
        let changed = tx.execute("UPDATE jobs SET cancel_requested=CASE WHEN status IN ('queued','running','cancelling') THEN 1 ELSE cancel_requested END,status=CASE WHEN status IN ('queued','running') THEN 'cancelling' ELSE status END WHERE job_id=?1", params![job_id])?;
        if changed == 0 {
            return Err(JobError::Invalid("job does not exist".into()));
        }
        // Queued work can be cancelled immediately. Running work remains
        // leased so a concurrently finishing checkpoint can commit; expiry
        // recovery turns it into `cancelled` if it does not finish.
        tx.execute("UPDATE pages SET status='cancelled',error='cancelled',lease_owner=NULL,lease_until=NULL WHERE job_id=?1 AND status='queued'", params![job_id])?;
        tx.execute("UPDATE jobs SET status='cancelled' WHERE job_id=?1 AND status IN ('queued','running','cancelling') AND NOT EXISTS (SELECT 1 FROM pages WHERE job_id=?1 AND status='running')", params![job_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn recover_expired(&self, now: i64) -> JobResult<u32> {
        if now < 0 {
            return Err(JobError::Invalid("timestamp must be non-negative".into()));
        }
        let tx = self.connection.unchecked_transaction()?;
        let count = tx.execute("UPDATE pages SET status=CASE WHEN (SELECT cancel_requested FROM jobs WHERE jobs.job_id=pages.job_id) != 0 THEN 'cancelled' WHEN attempts >= ?2 THEN 'failed' ELSE 'queued' END, error=CASE WHEN attempts >= ?2 THEN 'retry limit exceeded' ELSE error END, lease_owner=NULL,lease_until=NULL WHERE status='running' AND lease_until<=?1", params![now, MAX_PAGE_ATTEMPTS])? as u32;
        tx.execute("UPDATE jobs SET status=CASE WHEN cancel_requested != 0 THEN 'cancelled' WHEN EXISTS (SELECT 1 FROM pages WHERE pages.job_id=jobs.job_id AND pages.status='failed') THEN 'failed' ELSE 'queued' END,lease_owner=NULL,lease_until=NULL WHERE status IN ('running','cancelling') AND (lease_until IS NULL OR lease_until<=?1)", params![now])?;
        tx.commit()?;
        Ok(count)
    }
}

fn row_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    Ok(JobRecord {
        job_id: row.get(0)?,
        status: parse_job(row.get::<_, String>(1)?)?,
        page_count: row.get(2)?,
        completed_pages: row.get(3)?,
        cancel_requested: row.get::<_, i64>(4)? != 0,
        heartbeat_at: row.get(5)?,
        retries: row.get(6)?,
    })
}
fn row_page(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageRecord> {
    Ok(PageRecord {
        job_id: row.get(0)?,
        page_index: row.get(1)?,
        status: parse_page(row.get::<_, String>(2)?)?,
        attempts: row.get(3)?,
        checkpoint: row.get(4)?,
        artifact_digest: row.get(5)?,
        error: row.get(6)?,
    })
}
#[allow(clippy::too_many_arguments)]
fn insert_provider_run(
    tx: &rusqlite::Transaction<'_>,
    job_id: &str,
    page_index: u32,
    provenance: &ProviderProvenance,
    output_digest: Option<&str>,
    outcome: ProviderOutcome,
    error: Option<&str>,
    started_at: i64,
    finished_at: i64,
) -> JobResult<()> {
    let parameters_json = serde_json::to_string(&provenance.parameters)
        .map_err(|error| JobError::Invalid(error.to_string()))?;
    let execution_location = serde_json::to_string(&provenance.execution_location)
        .map_err(|error| JobError::Invalid(error.to_string()))?;
    let outcome =
        serde_json::to_string(&outcome).map_err(|error| JobError::Invalid(error.to_string()))?;
    tx.execute("INSERT INTO provider_runs(job_id,page_index,engine,model,version,parameters_json,input_asset_sha256,output_digest,execution_location,outcome,error,started_at,finished_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![job_id,page_index,provenance.engine,provenance.model,provenance.version,parameters_json,provenance.input_asset_sha256,output_digest,execution_location,outcome,error,started_at,finished_at])?;
    Ok(())
}
fn finalize_page_transaction(
    tx: &rusqlite::Transaction<'_>,
    job_id: &str,
    now: i64,
) -> JobResult<()> {
    tx.execute("UPDATE jobs SET completed_pages=(SELECT COUNT(*) FROM pages WHERE job_id=?1 AND status='completed'),heartbeat_at=?2 WHERE job_id=?1", params![job_id,now])?;
    let remaining: i64 = tx.query_row(
        "SELECT COUNT(*) FROM pages WHERE job_id=?1 AND status NOT IN ('completed','cancelled')",
        params![job_id],
        |row| row.get(0),
    )?;
    let cancelled: i64 = tx.query_row(
        "SELECT cancel_requested FROM jobs WHERE job_id=?1",
        params![job_id],
        |row| row.get(0),
    )?;
    if remaining == 0 {
        tx.execute(
            "UPDATE jobs SET status=?2 WHERE job_id=?1",
            params![
                job_id,
                if cancelled != 0 {
                    "cancelled"
                } else {
                    "completed"
                }
            ],
        )?;
    }
    Ok(())
}
fn row_provider_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderRunRecord> {
    Ok(ProviderRunRecord {
        run_id: row.get(0)?,
        job_id: row.get(1)?,
        page_index: row.get(2)?,
        engine: row.get(3)?,
        model: row.get(4)?,
        version: row.get(5)?,
        parameters: serde_json::from_str(&row.get::<_, String>(6)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        input_asset_sha256: row.get(7)?,
        output_digest: row.get(8)?,
        execution_location: serde_json::from_str(&row.get::<_, String>(9)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        outcome: serde_json::from_str(&row.get::<_, String>(10)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        error: row.get(11)?,
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}
fn parse_job(value: String) -> rusqlite::Result<JobStatus> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn parse_page(value: String) -> rusqlite::Result<PageStatus> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRequest {
    pub protocol: String,
    pub protocol_version: String,
    pub job_id: String,
    pub page_id: String,
    pub page_index: u32,
    pub input_asset_sha256: String,
    pub parameters: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProvenance {
    pub engine: String,
    pub model: String,
    pub version: String,
    pub parameters: BTreeMap<String, String>,
    pub input_asset_sha256: String,
    pub execution_location: ExecutionLocation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLocation {
    Local,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderResponse {
    pub protocol: String,
    pub protocol_version: String,
    pub output_digest: String,
    pub provenance: ProviderProvenance,
}

pub trait Provider {
    fn process(&mut self, request: &ProviderRequest) -> JobResult<ProviderResponse>;
}

pub fn validate_provider_request(request: &ProviderRequest) -> JobResult<()> {
    validate_protocol(&request.protocol, &request.protocol_version)?;
    if !valid_identifier(&request.job_id) || !valid_identifier(&request.page_id) {
        return Err(JobError::Invalid("provider job/page ID is required".into()));
    }
    validate_sha256(&request.input_asset_sha256, "input asset digest")?;
    validate_parameters(&request.parameters)?;
    Ok(())
}

pub fn validate_provider_response(response: &ProviderResponse) -> JobResult<()> {
    validate_protocol(&response.protocol, &response.protocol_version)?;
    validate_sha256(&response.output_digest, "output digest")?;
    validate_provenance(&response.provenance)
}

fn validate_protocol(protocol: &str, version: &str) -> JobResult<()> {
    if protocol != JOB_PROTOCOL {
        return Err(JobError::UnsupportedProtocol(protocol.to_string()));
    }
    let mut pieces = version.split('.');
    let major = pieces.next().and_then(|value| value.parse::<u16>().ok());
    let minor = pieces.next().and_then(|value| value.parse::<u16>().ok());
    if major != Some(0)
        || minor.is_none()
        || pieces.next().is_some()
        || format!(
            "{}.{}",
            major.unwrap_or_default(),
            minor.unwrap_or_default()
        ) != version
    {
        return Err(JobError::UnsupportedProtocol(version.to_string()));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> JobResult<()> {
    if !is_sha256_hex(value) || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(JobError::Invalid(format!(
            "{label} must be lowercase SHA-256"
        )));
    }
    Ok(())
}

fn validate_parameters(parameters: &BTreeMap<String, String>) -> JobResult<()> {
    if parameters.len() > MAX_PAYLOAD_ITEMS {
        return Err(JobError::Invalid("too many provider parameters".into()));
    }
    for (key, value) in parameters {
        if key.is_empty() || key.len() > MAX_PARAMETER_BYTES || value.len() > MAX_PARAMETER_BYTES {
            return Err(JobError::Invalid(
                "provider parameter is out of bounds".into(),
            ));
        }
    }
    Ok(())
}

fn validate_provenance(provenance: &ProviderProvenance) -> JobResult<()> {
    if !valid_identifier(&provenance.engine)
        || !valid_identifier(&provenance.model)
        || !valid_identifier(&provenance.version)
    {
        return Err(JobError::Invalid(
            "provider provenance identity is required".into(),
        ));
    }
    validate_parameters(&provenance.parameters)?;
    validate_sha256(&provenance.input_asset_sha256, "input asset digest")
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeBehavior {
    Success,
    PartialFailure,
    Timeout,
    Crash,
    OutOfOrder,
    ProtocolMismatch,
}
pub struct FakeProvider {
    pub behavior: FakeBehavior,
    pub calls: u32,
    pub last_page_index: Option<u32>,
}
impl Provider for FakeProvider {
    fn process(&mut self, request: &ProviderRequest) -> JobResult<ProviderResponse> {
        self.calls += 1;
        validate_provider_request(request)?;
        if request.protocol_version != JOB_PROTOCOL_VERSION
            || self.behavior == FakeBehavior::ProtocolMismatch
        {
            return Err(JobError::UnsupportedProtocol(
                request.protocol_version.clone(),
            ));
        }
        if self.behavior == FakeBehavior::OutOfOrder
            && self
                .last_page_index
                .is_some_and(|last| request.page_index <= last)
        {
            return Err(JobError::Provider("out-of-order page result".into()));
        }
        match self.behavior {
            FakeBehavior::Success | FakeBehavior::OutOfOrder => {}
            FakeBehavior::PartialFailure => {
                if self.calls % 2 == 0 {
                    return Err(JobError::Provider("partial failure".into()));
                }
            }
            FakeBehavior::Timeout => return Err(JobError::Provider("timeout".into())),
            FakeBehavior::Crash => return Err(JobError::Provider("provider crashed".into())),
            FakeBehavior::ProtocolMismatch => unreachable!(),
        }
        self.last_page_index = Some(request.page_index);
        let mut hasher = Sha256::new();
        hasher.update(request.input_asset_sha256.as_bytes());
        hasher.update(request.page_id.as_bytes());
        Ok(ProviderResponse {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            output_digest: hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            provenance: ProviderProvenance {
                engine: "reference".into(),
                model: "fake".into(),
                version: "0.1".into(),
                parameters: request.parameters.clone(),
                input_asset_sha256: request.input_asset_sha256.clone(),
                execution_location: ExecutionLocation::Local,
            },
        })
        .and_then(|response| {
            validate_provider_response(&response)?;
            Ok(response)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SidecarKind {
    JobStarted,
    PageResult,
    PageFailed,
    JobFinished,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SidecarMessage {
    pub protocol: String,
    pub protocol_version: String,
    pub kind: SidecarKind,
    pub job_id: String,
    pub page_index: Option<u32>,
    pub payload: BTreeMap<String, String>,
}
pub fn encode_sidecar(messages: &[SidecarMessage]) -> JobResult<String> {
    validate_transcript(messages)?;
    let mut out = String::new();
    for message in messages {
        let line = serde_json::to_string(message).map_err(|e| JobError::Invalid(e.to_string()))?;
        if line.len() > MAX_SIDECAR_LINE_BYTES {
            return Err(JobError::Invalid(
                "sidecar record exceeds byte limit".into(),
            ));
        }
        out.push_str(&line);
        out.push('\n');
        if out.len() > MAX_SIDECAR_BYTES {
            return Err(JobError::Invalid("sidecar exceeds byte limit".into()));
        }
    }
    Ok(out)
}
pub fn decode_sidecar(input: &str) -> JobResult<Vec<SidecarMessage>> {
    if input.is_empty() || input.len() > MAX_SIDECAR_BYTES {
        return Err(JobError::Invalid("sidecar byte limit exceeded".into()));
    }
    if !input.is_empty() && !input.ends_with('\n') {
        return Err(JobError::Invalid(
            "sidecar does not end with a complete record".into(),
        ));
    }
    let mut result = Vec::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            return Err(JobError::Invalid("sidecar contains an empty record".into()));
        }
        if line.len() > MAX_SIDECAR_LINE_BYTES {
            return Err(JobError::Invalid(
                "sidecar record exceeds byte limit".into(),
            ));
        }
        let message: SidecarMessage =
            serde_json::from_str(line).map_err(|e| JobError::Invalid(e.to_string()))?;
        result.push(message);
        if result.len() > MAX_SIDECAR_RECORDS {
            return Err(JobError::Invalid("sidecar record limit exceeded".into()));
        }
    }
    validate_transcript(&result)?;
    Ok(result)
}

pub fn validate_transcript(messages: &[SidecarMessage]) -> JobResult<()> {
    if messages.is_empty() || messages.len() > MAX_SIDECAR_RECORDS {
        return Err(JobError::Invalid(
            "sidecar record count is out of bounds".into(),
        ));
    }
    let started = messages
        .iter()
        .filter(|message| matches!(message.kind, SidecarKind::JobStarted))
        .count();
    let finished = messages
        .iter()
        .filter(|message| matches!(message.kind, SidecarKind::JobFinished))
        .count();
    if started != 1
        || finished != 1
        || !matches!(
            messages.first().map(|message| &message.kind),
            Some(SidecarKind::JobStarted)
        )
        || !matches!(
            messages.last().map(|message| &message.kind),
            Some(SidecarKind::JobFinished)
        )
    {
        return Err(JobError::Invalid(
            "sidecar must start once and finish once".into(),
        ));
    }
    let job_id = &messages[0].job_id;
    if job_id.is_empty() || job_id.len() > MAX_PARAMETER_BYTES {
        return Err(JobError::Invalid("sidecar job ID is required".into()));
    }
    let mut pages = std::collections::BTreeSet::new();
    for message in messages {
        validate_protocol(&message.protocol, &message.protocol_version)?;
        if message.job_id != *job_id || message.job_id.is_empty() {
            return Err(JobError::Invalid("sidecar job IDs must agree".into()));
        }
        if message.payload.len() > MAX_PAYLOAD_ITEMS {
            return Err(JobError::Invalid(
                "sidecar payload has too many items".into(),
            ));
        }
        validate_parameters(&message.payload)?;
        match message.kind {
            SidecarKind::JobStarted | SidecarKind::JobFinished => {
                if message.page_index.is_some() {
                    return Err(JobError::Invalid(
                        "job boundary cannot have page index".into(),
                    ));
                }
            }
            SidecarKind::PageResult | SidecarKind::PageFailed => {
                let Some(page_index) = message.page_index else {
                    return Err(JobError::Invalid("page record needs page index".into()));
                };
                if !pages.insert(page_index) {
                    return Err(JobError::Invalid("page result is duplicated".into()));
                }
            }
        }
    }
    if pages.is_empty() {
        return Err(JobError::Invalid("sidecar has no page result".into()));
    }
    Ok(())
}

/// Writes a sidecar as a single atomic replacement. A missing final newline or
/// malformed record is rejected by readers; callers therefore cannot infer a
/// successful job from a partially written sidecar.
pub fn write_sidecar(path: &Path, messages: &[SidecarMessage]) -> JobResult<()> {
    let encoded = encode_sidecar(messages)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| JobError::Invalid(format!("cannot create sidecar directory: {error}")))?;
    let temp_path = sidecar_temp_path(path);
    let result = (|| -> JobResult<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| {
                JobError::Invalid(format!("cannot create sidecar temporary file: {error}"))
            })?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(encoded.as_bytes())
            .map_err(|error| JobError::Invalid(format!("cannot write sidecar: {error}")))?;
        writer
            .flush()
            .map_err(|error| JobError::Invalid(format!("cannot flush sidecar: {error}")))?;
        writer
            .into_inner()
            .map_err(|error| JobError::Invalid(format!("cannot finalize sidecar: {error}")))?
            .sync_all()
            .map_err(|error| JobError::Invalid(format!("cannot sync sidecar: {error}")))?;
        // `hard_link` is a no-clobber directory entry creation on supported
        // local filesystems. Unlike exists()+rename(), it cannot replace a
        // destination that appeared after the preflight check (including a
        // dangling symlink).
        fs::hard_link(&temp_path, path)
            .map_err(|error| JobError::Invalid(format!("cannot install sidecar: {error}")))?;
        fs::remove_file(&temp_path).map_err(|error| {
            JobError::Invalid(format!("cannot remove sidecar temporary file: {error}"))
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn sidecar_temp_path(path: &Path) -> PathBuf {
    let suffix = format!("{}.{}.tmp", std::process::id(), now_seconds());
    path.with_file_name(format!(
        "{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sidecar"),
        suffix
    ))
}
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_wal_and_500_page_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        let job = store.create_job("j", 500).unwrap();
        assert_eq!(job.page_count, 500);
        let page = store.claim_page("j", "w", 0, 1).unwrap().unwrap();
        store.recover_expired(2).unwrap();
        assert_eq!(
            store.page("j", page.page_index).unwrap().unwrap().status,
            PageStatus::Queued
        );
    }

    #[test]
    fn five_hundred_page_cancel_restart_preserves_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&db).unwrap();
        store.create_job("large", 500).unwrap();
        let digest = "b".repeat(64);
        for index in 0..100 {
            store
                .claim_page("large", "worker", index as i64, 60)
                .unwrap();
            store
                .checkpoint_page("large", index, "worker", "committed", &digest, index as i64)
                .unwrap();
        }
        store.request_cancel("large").unwrap();
        assert_eq!(
            store.page("large", 0).unwrap().unwrap().status,
            PageStatus::Completed
        );
        assert_eq!(
            store.page("large", 100).unwrap().unwrap().status,
            PageStatus::Cancelled
        );
        drop(store);
        let restarted = JobStore::open(&db).unwrap();
        let progress = restarted.progress("large").unwrap().unwrap();
        assert_eq!(progress.completed_pages, 100);
        assert_eq!(progress.cancelled_pages, 400);
        assert_eq!(progress.status, JobStatus::Cancelled);
    }
    #[test]
    fn cancellation_preserves_completed_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        store.create_job("j", 2).unwrap();
        let digest = "a".repeat(64);
        store.claim_page("j", "w", 0, 60).unwrap();
        store
            .checkpoint_page("j", 0, "w", "checkpoint", &digest, 1)
            .unwrap();
        store.request_cancel("j").unwrap();
        assert_eq!(
            store.page("j", 0).unwrap().unwrap().status,
            PageStatus::Completed
        );
    }
    #[test]
    fn fake_provider_and_sidecar_reject_protocol_mismatch() {
        let mut provider = FakeProvider {
            behavior: FakeBehavior::Success,
            calls: 0,
            last_page_index: None,
        };
        let request = ProviderRequest {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            job_id: "j".into(),
            page_id: "p".into(),
            page_index: 0,
            input_asset_sha256: "a".repeat(64),
            parameters: BTreeMap::new(),
        };
        assert!(provider.process(&request).is_ok());
        let mut bad = request.clone();
        bad.protocol_version = "9.0".into();
        assert!(provider.process(&bad).is_err());
        let mut bad_identity = request.clone();
        bad_identity.protocol = "other-provider".into();
        assert!(validate_provider_request(&bad_identity).is_err());
        let mut message = SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind: SidecarKind::PageResult,
            job_id: "j".into(),
            page_index: Some(0),
            payload: BTreeMap::new(),
        };
        let started = SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind: SidecarKind::JobStarted,
            job_id: "j".into(),
            page_index: None,
            payload: BTreeMap::new(),
        };
        let finished = SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind: SidecarKind::JobFinished,
            job_id: "j".into(),
            page_index: None,
            payload: BTreeMap::new(),
        };
        let transcript = vec![started, message.clone(), finished];
        let encoded = encode_sidecar(&transcript).unwrap();
        assert_eq!(decode_sidecar(&encoded).unwrap(), transcript);
        assert!(decode_sidecar(&encoded[..encoded.len() - 1]).is_err());
        message.protocol = "9.0".into();
        assert!(encode_sidecar(&[
            SidecarMessage {
                protocol: JOB_PROTOCOL.into(),
                protocol_version: JOB_PROTOCOL_VERSION.into(),
                kind: SidecarKind::JobStarted,
                job_id: "j".into(),
                page_index: None,
                payload: BTreeMap::new()
            },
            message,
            SidecarMessage {
                protocol: JOB_PROTOCOL.into(),
                protocol_version: JOB_PROTOCOL_VERSION.into(),
                kind: SidecarKind::JobFinished,
                job_id: "j".into(),
                page_index: None,
                payload: BTreeMap::new()
            }
        ])
        .is_err());
    }

    #[test]
    fn retry_and_out_of_order_are_explicit_failures() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        store.create_job("j", 1).unwrap();
        store.claim_page("j", "w", 0, 60).unwrap();
        assert_eq!(
            store.fail_page("j", 0, "w", "temporary", true, 1).unwrap(),
            PageStatus::Queued
        );
        store.claim_page("j", "w", 2, 60).unwrap();
        assert_eq!(
            store.fail_page("j", 0, "w", "temporary", true, 3).unwrap(),
            PageStatus::Queued
        );
        store.claim_page("j", "w", 4, 60).unwrap();
        assert_eq!(
            store.fail_page("j", 0, "w", "temporary", true, 5).unwrap(),
            PageStatus::Failed
        );
        let mut provider = FakeProvider {
            behavior: FakeBehavior::OutOfOrder,
            calls: 0,
            last_page_index: None,
        };
        let request = ProviderRequest {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            job_id: "j".into(),
            page_id: "p".into(),
            page_index: 2,
            input_asset_sha256: "a".repeat(64),
            parameters: BTreeMap::new(),
        };
        provider.process(&request).unwrap();
        let mut out_of_order = request;
        out_of_order.page_index = 1;
        assert!(provider.process(&out_of_order).is_err());
    }

    #[test]
    fn fake_provider_covers_failure_modes_and_provenance() {
        let request = ProviderRequest {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            job_id: "j".into(),
            page_id: "p".into(),
            page_index: 0,
            input_asset_sha256: "c".repeat(64),
            parameters: BTreeMap::new(),
        };
        let mut success = FakeProvider {
            behavior: FakeBehavior::Success,
            calls: 0,
            last_page_index: None,
        };
        let response = success.process(&request).unwrap();
        assert_eq!(
            response.provenance.input_asset_sha256,
            request.input_asset_sha256
        );
        for behavior in [
            FakeBehavior::Timeout,
            FakeBehavior::Crash,
            FakeBehavior::ProtocolMismatch,
        ] {
            let mut provider = FakeProvider {
                behavior,
                calls: 0,
                last_page_index: None,
            };
            assert!(provider.process(&request).is_err());
        }
        let mut partial = FakeProvider {
            behavior: FakeBehavior::PartialFailure,
            calls: 0,
            last_page_index: None,
        };
        assert!(partial.process(&request).is_ok());
        assert!(partial.process(&request).is_err());
    }

    #[test]
    fn sidecar_writer_is_atomic_and_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.ndjson");
        let message = SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind: SidecarKind::PageResult,
            job_id: "j".into(),
            page_index: Some(0),
            payload: BTreeMap::new(),
        };
        let transcript = vec![
            SidecarMessage {
                protocol: JOB_PROTOCOL.into(),
                protocol_version: JOB_PROTOCOL_VERSION.into(),
                kind: SidecarKind::JobStarted,
                job_id: "j".into(),
                page_index: None,
                payload: BTreeMap::new(),
            },
            message.clone(),
            SidecarMessage {
                protocol: JOB_PROTOCOL.into(),
                protocol_version: JOB_PROTOCOL_VERSION.into(),
                kind: SidecarKind::JobFinished,
                job_id: "j".into(),
                page_index: None,
                payload: BTreeMap::new(),
            },
        ];
        write_sidecar(&path, &transcript).unwrap();
        let original = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            decode_sidecar(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            transcript
        );
        assert!(write_sidecar(&path, &transcript).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(!dir.path().join("job.ndjson.unknown.tmp").exists());
        #[cfg(unix)]
        {
            let dangling = dir.path().join("dangling.ndjson");
            std::os::unix::fs::symlink(dir.path().join("missing-target"), &dangling).unwrap();
            assert!(write_sidecar(&dangling, &transcript).is_err());
            assert!(dangling.read_link().is_ok());
        }
    }

    #[test]
    fn sidecar_transcript_rules_reject_incomplete_or_reordered_records() {
        let boundary = |kind| SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind,
            job_id: "j".into(),
            page_index: None,
            payload: BTreeMap::new(),
        };
        let page = |kind, index| SidecarMessage {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            kind,
            job_id: "j".into(),
            page_index: index,
            payload: BTreeMap::new(),
        };
        let valid = vec![
            boundary(SidecarKind::JobStarted),
            page(SidecarKind::PageResult, Some(0)),
            boundary(SidecarKind::JobFinished),
        ];
        assert!(validate_transcript(&valid).is_ok());
        assert!(validate_transcript(&valid[..1]).is_err());
        assert!(
            validate_transcript(&[valid[0].clone(), valid[2].clone(), valid[1].clone()]).is_err()
        );
        assert!(validate_transcript(&[
            valid[0].clone(),
            page(SidecarKind::PageResult, None),
            valid[2].clone()
        ])
        .is_err());
        assert!(validate_transcript(&[
            valid[0].clone(),
            page(SidecarKind::PageResult, Some(0)),
            page(SidecarKind::PageFailed, Some(0)),
            valid[2].clone()
        ])
        .is_err());
        let mut wrong_job = valid.clone();
        wrong_job[1].job_id = "other".into();
        assert!(validate_transcript(&wrong_job).is_err());
        let mut bad_version = valid.clone();
        bad_version[1].protocol_version = "0.foo".into();
        assert!(validate_transcript(&bad_version).is_err());
        let mut too_many = valid.clone();
        too_many[1].payload = (0..=MAX_PAYLOAD_ITEMS)
            .map(|index| (index.to_string(), "v".into()))
            .collect();
        assert!(validate_transcript(&too_many).is_err());
        assert!(decode_sidecar("{\"protocol\":\"mpdf-job\"}\n").is_err());
    }

    #[test]
    fn terminal_cancel_is_idempotent_and_does_not_set_cancel_flag() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        store.create_job("completed", 1).unwrap();
        store.claim_page("completed", "w", 0, 60).unwrap();
        store
            .checkpoint_page("completed", 0, "w", "done", &"d".repeat(64), 1)
            .unwrap();
        store.request_cancel("completed").unwrap();
        let job = store.job("completed").unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert!(!job.cancel_requested);
        store.create_job("failed", 2).unwrap();
        store.claim_page("failed", "w", 0, 60).unwrap();
        store
            .fail_page("failed", 0, "w", "fatal", false, 1)
            .unwrap();
        store.request_cancel("failed").unwrap();
        let job = store.job("failed").unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert!(!job.cancel_requested);
        assert_eq!(
            store.page("failed", 1).unwrap().unwrap().status,
            PageStatus::Queued
        );
    }

    #[test]
    fn immediate_claim_is_single_owner_and_expired_owner_cannot_commit() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("jobs.sqlite");
        let first = JobStore::open(&db).unwrap();
        first.create_job("j", 1).unwrap();
        let second = JobStore::open(&db).unwrap();
        assert!(first.claim_page("j", "a", 0, 1).unwrap().is_some());
        assert!(second.claim_page("j", "b", 0, 60).is_err());
        assert!(first.heartbeat("j", "a", 0, 1).is_ok());
        assert!(second.claim_page("j", "b", 2, 60).unwrap().is_some());
        assert!(first.heartbeat("j", "a", 2, 60).is_err());
        assert!(first
            .checkpoint_page("j", 0, "a", "stale", &"e".repeat(64), 2)
            .is_err());
        assert!(first.fail_page("j", 0, "a", "stale", true, 2).is_err());
    }

    #[test]
    fn provider_success_and_failure_records_survive_database_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&db).unwrap();
        store.create_job("j", 2).unwrap();
        let mut provider = FakeProvider {
            behavior: FakeBehavior::Success,
            calls: 0,
            last_page_index: None,
        };
        let request = ProviderRequest {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            job_id: "j".into(),
            page_id: "p".into(),
            page_index: 0,
            input_asset_sha256: "a".repeat(64),
            parameters: BTreeMap::new(),
        };
        let response = provider.process(&request).unwrap();
        store.claim_page("j", "w", 0, 60).unwrap();
        store
            .record_provider_success_and_checkpoint("j", 0, "w", "checkpoint", &response, 0, 1)
            .unwrap();
        let mut failure_provenance = response.provenance.clone();
        store.claim_page("j", "w", 2, 60).unwrap();
        assert_eq!(
            store
                .record_provider_failure("j", 1, "w", &failure_provenance, "temporary", true, 2, 3)
                .unwrap(),
            PageStatus::Queued
        );
        drop(store);
        let reopened = JobStore::open(&db).unwrap();
        let runs = reopened.provider_runs("j").unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].outcome, ProviderOutcome::Succeeded);
        assert_eq!(runs[1].outcome, ProviderOutcome::Failed);
        assert_eq!(runs[1].error.as_deref(), Some("temporary"));
        failure_provenance.engine.clear();
        assert!(validate_provenance(&failure_provenance).is_err());
    }

    #[test]
    fn schema_version_and_identifier_time_bounds_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("jobs.sqlite");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .pragma_update(None, "user_version", 2_i64)
            .unwrap();
        drop(connection);
        assert!(JobStore::open(&db).is_err());

        let store = JobStore::open(&dir.path().join("valid.sqlite")).unwrap();
        assert!(store
            .create_job(&"j".repeat(MAX_IDENTIFIER_BYTES + 1), 1)
            .is_err());
        store.create_job("j", 1).unwrap();
        assert!(store.claim_page("j", "w", -1, 1).is_err());
        assert!(store
            .claim_page("j", "w", 0, MAX_LEASE_SECONDS + 1)
            .is_err());
        assert!(store.claim_page("j", "w", i64::MAX, 1).is_err());
        let request = ProviderRequest {
            protocol: JOB_PROTOCOL.into(),
            protocol_version: JOB_PROTOCOL_VERSION.into(),
            job_id: "j".into(),
            page_id: "p".repeat(MAX_IDENTIFIER_BYTES + 1),
            page_index: 0,
            input_asset_sha256: "a".repeat(64),
            parameters: BTreeMap::new(),
        };
        assert!(validate_provider_request(&request).is_err());
        let provenance = ProviderProvenance {
            engine: "e".repeat(MAX_IDENTIFIER_BYTES + 1),
            model: "m".into(),
            version: "v".into(),
            parameters: BTreeMap::new(),
            input_asset_sha256: "a".repeat(64),
            execution_location: ExecutionLocation::Local,
        };
        assert!(validate_provenance(&provenance).is_err());
    }
}
