//! CLI façade for the consented API protocol.  Secrets are read only from
//! stdin and delegated to the credential store; they are never arguments.

use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mpdf_api_client::{
    ApiClient, ApiClientConfig, NativeSecretStore, RuntimeSecretStore, Secret, SecretStore,
};
use mpdf_core::remote_api::{
    install_artifact, now_unix_seconds, ApiPlan, ApiStore, ApiTaskReceipt, ApiTaskRecord, Consent,
    RetentionState, TaskState,
};
use sha2::{Digest, Sha256};

use crate::cli::{
    ApiCommand, ApiCredentialCommand, ApiImportArgs, ApiPlanArgs, ApiRunArgs, ApiStatusArgs,
    ApiTaskArgs,
};
use crate::errors::ExitReason;
use crate::output;

pub fn run(command: ApiCommand) -> ExitCode {
    match command {
        ApiCommand::Credential(c) => credential(c),
        ApiCommand::Plan(a) => plan(a),
        ApiCommand::Run(a) => run_remote(a),
        ApiCommand::Status(a) => status(a),
        ApiCommand::Cancel(a) => cancel(a),
        ApiCommand::Import(a) => import(a),
        ApiCommand::DeleteContent(a) => delete_content(a),
    }
}

fn credential(command: ApiCredentialCommand) -> ExitCode {
    let store = NativeSecretStore;
    let result = match command {
        ApiCredentialCommand::Set(a) => {
            let mut input = String::new();
            if io::stdin().read_to_string(&mut input).is_err() {
                return fail("credential input unavailable");
            }
            let value = input.trim_end_matches(['\r', '\n']);
            if value.is_empty() {
                return fail("credential must not be empty");
            }
            let result = store.set(&a.profile, Secret::new(value));
            if result.is_ok() && a.output_mode.json {
                output::print_json(
                    &serde_json::json!({"profile":a.profile,"stored":true}),
                    a.output_mode.pretty,
                );
            }
            result
        }
        ApiCredentialCommand::Status(a) => match store.get(&a.profile) {
            Ok(value) => {
                let report = serde_json::json!({"profile":a.profile,"present":value.is_some()});
                if a.output_mode.json || !a.output_mode.quiet {
                    output::print_json(&report, a.output_mode.pretty);
                }
                return ExitReason::Success.exit_code();
            }
            Err(e) => Err(e),
        },
        ApiCredentialCommand::Delete(a) => {
            let result = store.delete(&a.profile);
            if result.is_ok() && a.output_mode.json {
                output::print_json(
                    &serde_json::json!({"profile":a.profile,"deleted":true}),
                    a.output_mode.pretty,
                );
            }
            result
        }
    };
    result
        .map(|_| ExitReason::Success.exit_code())
        .unwrap_or_else(|e| fail(&e.to_string()))
}

fn plan(args: ApiPlanArgs) -> ExitCode {
    let bytes = match read_regular_file(&args.source) {
        Ok(b) => b,
        Err(e) => return fail(&format!("cannot read source: {e}")),
    };
    let digest = hex(&Sha256::digest(&bytes));
    let p = match ApiPlan::new(
        args.endpoint,
        args.provider,
        args.model,
        digest,
        bytes.len() as u64,
        args.page_count,
        args.budget_micros,
        args.currency,
        args.retention.into(),
    ) {
        Ok(p) => p,
        Err(e) => return fail(&e.to_string()),
    };
    let json = match p.canonical_json() {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    if let Err(e) = write_bytes_new(&args.output, &json) {
        return fail_as(&e.to_string(), ExitReason::OutputError);
    }
    if args.output_mode.json {
        output::print_json(&p, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        println!("Created API plan: {}", args.output.display());
    }
    ExitReason::Success.exit_code()
}

fn run_remote(args: ApiRunArgs) -> ExitCode {
    let plan: ApiPlan = match read_json(&args.plan) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if let Err(e) = plan.validate() {
        return fail(&e.to_string());
    }
    let source = match read_regular_file(&args.source) {
        Ok(b) => b,
        Err(e) => return fail(&e.to_string()),
    };
    let consent = Consent {
        plan_digest: args.consent,
        consented_at: now_unix_seconds(),
    };
    if consent.plan_digest != plan.plan_digest {
        return fail("consent digest does not match plan");
    }
    let config = client_config(&plan.origin, &args.profile, args.allow_loopback_http);
    let client = match ApiClient::new(config, Arc::new(RuntimeSecretStore::default())) {
        Ok(c) => c,
        Err(e) => return fail(&e.to_string()),
    };
    let created = match client.create_task(&plan, &consent) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    if !created.deduplicated {
        if let Err(e) = client.upload_blob(&plan, &source, &consent) {
            return fail(&e.to_string());
        }
    }
    if let Err(e) = client.start(&plan, &created.task_id) {
        return fail(&e.to_string());
    }
    let status = match client.poll_until_terminal(&plan, &created.task_id) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    let receipt = match client.receipt(&plan, &created.task_id) {
        Ok(r) => r,
        Err(e) => return fail(&e.to_string()),
    };
    if let Err(e) = receipt.matches_plan(&plan) {
        return fail(&e.to_string());
    }
    if args.receipt.exists() {
        return fail_as(
            "receipt destination already exists; refusing to overwrite",
            ExitReason::OutputError,
        );
    }
    if let Err(e) = write_new(&args.receipt, &receipt) {
        return fail_as(&e, ExitReason::OutputError);
    }
    let result = match client.result(&plan, &created.task_id) {
        Ok(r) => r,
        Err(e) => return fail(&e.to_string()),
    };
    let artifact_result = match args.mdp_root.as_deref() {
        Some(root) => mpdf_core::remote_api::install_remote_ocr_result(root, &result),
        None => install_artifact(
            &args.artifact_dir,
            &result.result_digest,
            &result.raw_artifact,
        ),
    };
    let artifact = match artifact_result {
        Ok(p) => p,
        Err(e) => return fail_as(&e.to_string(), ExitReason::OutputError),
    };
    if let Err(e) = ApiStore::open(&args.jobs_db).and_then(|s| {
        let task = ApiTaskRecord {
            receipt: receipt.clone(),
            state: TaskState::ResultInstalled,
            retention: RetentionState::Pending,
            used_cost_micros: status.used_cost_micros,
            attempts: 1,
            artifact_digest: Some(result.result_digest.clone()),
        };
        s.put_task(&task)?;
        s.append_audit(&mpdf_core::remote_api::ApiAuditEvent {
            schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
            schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
            event_id: format!("{}-result", receipt.task_id),
            task_id: receipt.task_id.clone(),
            kind: "result".into(),
            state: TaskState::ResultInstalled,
            retention: RetentionState::Pending,
            request_digest: Some(receipt.request_id.clone()),
            response_digest: Some(result.result_digest.clone()),
            bytes: result.raw_artifact.len() as u64,
            cost_micros: status.used_cost_micros,
            attempt: 1,
            at: now_unix_seconds(),
            message: None,
        })
    }) {
        return fail(&e.to_string());
    }
    if matches!(
        plan.retention,
        mpdf_core::remote_api::Retention::DeleteAfterResult
    ) {
        let retention = client
            .delete_content(&plan, &created.task_id)
            .map(|_| RetentionState::Acknowledged)
            .unwrap_or(RetentionState::Failed);
        if let Ok(store) = ApiStore::open(&args.jobs_db) {
            let _ = store.put_task(&ApiTaskRecord {
                receipt: receipt.clone(),
                state: TaskState::ResultInstalled,
                retention,
                used_cost_micros: status.used_cost_micros,
                attempts: 1,
                artifact_digest: Some(result.result_digest.clone()),
            });
            let _ = store.append_audit(&mpdf_core::remote_api::ApiAuditEvent {
                schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
                schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
                event_id: format!("{}-retention", receipt.task_id),
                task_id: receipt.task_id.clone(),
                kind: "retention".into(),
                state: TaskState::ResultInstalled,
                retention,
                request_digest: Some(receipt.request_id.clone()),
                response_digest: None,
                bytes: 0,
                cost_micros: 0,
                attempt: 1,
                at: now_unix_seconds(),
                message: None,
            });
        }
    }
    if let Err(error) = persist_network_traces(
        &args.jobs_db,
        &receipt.task_id,
        client.drain_traces(),
        status.used_cost_micros,
    ) {
        return fail(&error);
    }
    if args.output_mode.json {
        output::print_json(
            &serde_json::json!({"task_id":created.task_id,"receipt":args.receipt,"artifact":artifact,"pages":result.pages.len()}),
            args.output_mode.pretty,
        );
    } else if !args.output_mode.quiet {
        println!(
            "Remote OCR task {} installed {}",
            created.task_id,
            artifact.display()
        );
    }
    ExitReason::Success.exit_code()
}

fn status(args: ApiStatusArgs) -> ExitCode {
    let receipt: ApiTaskReceipt = match read_json(&args.receipt) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Err(e) = receipt.validate() {
        return fail(&e.to_string());
    }
    let c = match ApiClient::new(
        client_config(&receipt.origin, &args.profile, args.allow_loopback_http),
        Arc::new(RuntimeSecretStore::default()),
    ) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    match c.status(&receipt.task_id) {
        Ok(v) => {
            if args.output_mode.json || !args.output_mode.quiet {
                output::print_json(&v, args.output_mode.pretty);
            }
            ExitReason::Success.exit_code()
        }
        Err(e) => fail(&e.to_string()),
    }
}
fn import(args: ApiImportArgs) -> ExitCode {
    let receipt: ApiTaskReceipt = match read_json(&args.receipt) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Err(e) = receipt.validate() {
        return fail(&e.to_string());
    };
    let c = match ApiClient::new(
        client_config(&receipt.origin, &args.profile, args.allow_loopback_http),
        Arc::new(RuntimeSecretStore::default()),
    ) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    let status_result = c.status(&receipt.task_id);
    if let (Ok(v), Some(dir)) = (&status_result, args.artifact_dir) {
        if matches!(v.state, TaskState::Completed | TaskState::ResultInstalled) {
            let plan = match ApiPlan::new(
                receipt.origin.clone(),
                receipt.provider.clone(),
                receipt.model.clone(),
                receipt.source_sha256.clone(),
                receipt.source_bytes,
                receipt.page_count,
                receipt.max_cost_micros,
                receipt.currency.clone(),
                receipt.retention,
            ) {
                Ok(p) if p.plan_digest == receipt.plan_digest => p,
                _ => return fail("receipt plan binding is invalid"),
            };
            let result = match c.result(&plan, &receipt.task_id) {
                Ok(v) => v,
                Err(e) => return fail(&e.to_string()),
            };
            if let Err(e) = install_artifact(&dir, &result.result_digest, &result.raw_artifact) {
                return fail(&e.to_string());
            }
        }
    }
    match status_result {
        Ok(v) => {
            if args.output_mode.json || !args.output_mode.quiet {
                output::print_json(&v, args.output_mode.pretty);
            }
            ExitReason::Success.exit_code()
        }
        Err(e) => fail(&e.to_string()),
    }
}
fn cancel(args: ApiTaskArgs) -> ExitCode {
    let receipt: ApiTaskReceipt = match read_json(&args.receipt) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Err(error) = receipt.validate() {
        return fail(&error.to_string());
    }
    let c = match ApiClient::new(
        client_config(&receipt.origin, &args.profile, args.allow_loopback_http),
        Arc::new(RuntimeSecretStore::default()),
    ) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    match c.cancel(&receipt) {
        Ok(()) => {
            if let Some(path) = args.jobs_db {
                if let Err(error) = persist_task_action(
                    &path,
                    &receipt,
                    TaskState::Cancelled,
                    RetentionState::NotRequested,
                    "cancel",
                    c.drain_traces(),
                ) {
                    return fail(&error);
                }
            }
            ExitReason::Success.exit_code()
        }
        Err(e) => fail(&e.to_string()),
    }
}
fn delete_content(args: ApiTaskArgs) -> ExitCode {
    let receipt: ApiTaskReceipt = match read_json(&args.receipt) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Err(error) = receipt.validate() {
        return fail(&error.to_string());
    }
    let c = match ApiClient::new(
        client_config(&receipt.origin, &args.profile, args.allow_loopback_http),
        Arc::new(RuntimeSecretStore::default()),
    ) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
    };
    match c.delete_content_receipt(&receipt) {
        Ok(()) => {
            if let Some(path) = args.jobs_db {
                if let Err(error) = persist_task_action(
                    &path,
                    &receipt,
                    TaskState::ResultInstalled,
                    RetentionState::Acknowledged,
                    "delete_content",
                    c.drain_traces(),
                ) {
                    return fail(&error);
                }
            }
            ExitReason::Success.exit_code()
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn persist_task_action(
    jobs_db: &Path,
    receipt: &ApiTaskReceipt,
    state: TaskState,
    retention: RetentionState,
    kind: &str,
    traces: Vec<mpdf_api_client::RequestTrace>,
) -> Result<(), String> {
    let store = ApiStore::open(jobs_db).map_err(|e| e.to_string())?;
    let existing = store.task(&receipt.task_id).map_err(|e| e.to_string())?;
    let mut task = existing.unwrap_or(ApiTaskRecord {
        receipt: receipt.clone(),
        state,
        retention,
        used_cost_micros: 0,
        attempts: 0,
        artifact_digest: None,
    });
    task.state = state;
    task.retention = retention;
    store.put_task(&task).map_err(|e| e.to_string())?;
    let offset = store
        .audit(&receipt.task_id)
        .map_err(|e| e.to_string())?
        .len();
    store
        .append_audit(&mpdf_core::remote_api::ApiAuditEvent {
            schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
            schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
            event_id: format!("{}-{kind}-{offset}", receipt.task_id),
            task_id: receipt.task_id.clone(),
            kind: kind.into(),
            state,
            retention,
            request_digest: Some(receipt.request_id.clone()),
            response_digest: None,
            bytes: 0,
            cost_micros: 0,
            attempt: 1,
            at: now_unix_seconds(),
            message: None,
        })
        .map_err(|e| e.to_string())?;
    persist_network_traces(jobs_db, &receipt.task_id, traces, task.used_cost_micros)
}

fn client_config(endpoint: &str, profile: &str, allow: bool) -> ApiClientConfig {
    ApiClientConfig {
        endpoint: endpoint.into(),
        profile_id: profile.into(),
        allow_loopback_http: allow,
        connect_timeout: Duration::from_secs(5),
        total_timeout: Duration::from_secs(60),
        max_response_bytes: 8 * 1024 * 1024,
        max_retries: 3,
    }
}

fn persist_network_traces(
    jobs_db: &Path,
    task_id: &str,
    traces: Vec<mpdf_api_client::RequestTrace>,
    used_cost_micros: u64,
) -> Result<(), String> {
    let store = ApiStore::open(jobs_db).map_err(|e| e.to_string())?;
    let offset = store.audit(task_id).map_err(|e| e.to_string())?.len();
    for (index, trace) in traces.into_iter().enumerate() {
        let state = match trace.kind.as_str() {
            "create" => TaskState::Creating,
            "upload" => TaskState::UploadPending,
            "start" | "status" => TaskState::Running,
            "result" => TaskState::Completed,
            "cancel" => TaskState::Cancelling,
            "delete_content" => TaskState::ResultInstalled,
            _ => TaskState::Failed,
        };
        store
            .append_audit(&mpdf_core::remote_api::ApiAuditEvent {
                schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
                schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
                event_id: format!("{task_id}-network-{}", offset + index),
                task_id: task_id.into(),
                kind: if trace.outcome == "retry" {
                    "retry".into()
                } else {
                    trace.kind
                },
                state,
                retention: RetentionState::Pending,
                request_digest: trace.request_digest,
                response_digest: trace.response_digest,
                bytes: trace.request_bytes.saturating_add(trace.response_bytes),
                cost_micros: used_cost_micros,
                attempt: trace.attempt,
                at: now_unix_seconds(),
                message: Some(match trace.http_status {
                    Some(status) => format!("{}; http_status={status}", trace.outcome),
                    None => trace.outcome,
                }),
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    if std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return Err("input path is a symlink or missing".into());
    }
    let b = std::fs::read(p).map_err(|e| e.to_string())?;
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}
fn read_regular_file(p: &Path) -> Result<Vec<u8>, std::io::Error> {
    let metadata = std::fs::symlink_metadata(p)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source must be a regular file",
        ));
    }
    if metadata.len() > mpdf_api_client::MAX_SOURCE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "source exceeds the 512 MiB upload limit",
        ));
    }
    std::fs::read(p)
}
fn write_new<T: serde::Serialize>(p: &Path, v: &T) -> Result<(), String> {
    let b = serde_json::to_vec(v).map_err(|e| e.to_string())?;
    write_bytes_new(p, &b)
}
fn write_bytes_new(p: &Path, b: &[u8]) -> Result<(), String> {
    let parent = p
        .parent()
        .filter(|v| !v.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure_safe_output_parent(parent)?;
    if std::fs::symlink_metadata(p).is_ok() {
        return Err("destination already exists; refusing to overwrite".into());
    }
    use std::io::Write;
    let mut temporary = tempfile::Builder::new()
        .prefix(".mpdf-api-")
        .suffix(".partial")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?;
    temporary.write_all(b).map_err(|e| e.to_string())?;
    temporary.flush().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist_noclobber(p).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            "destination already exists; refusing to overwrite".into()
        } else {
            error.error.to_string()
        }
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())
}

fn ensure_safe_output_parent(parent: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut cursor = parent;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or("invalid output parent")?;
    }
    let metadata = std::fs::symlink_metadata(cursor).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("output parent is not a real directory".into());
    }
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    for created in missing.into_iter().rev() {
        let metadata = std::fs::symlink_metadata(created).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("output parent is unsafe".into());
        }
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn fail(message: &str) -> ExitCode {
    fail_as(message, ExitReason::InputError)
}
fn fail_as(message: &str, reason: ExitReason) -> ExitCode {
    eprintln!("error: {message}");
    reason.exit_code()
}
