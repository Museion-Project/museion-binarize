//! API route/credential presence commands.  The UI receives policy and
//! presence only; bearer values never cross this IPC boundary.

use std::sync::Arc;
use std::time::Duration;

use crate::dto::{
    ApiConsentSummaryDto, ApiCredentialPresenceDto, ApiPlanRequestDto, ApiRouteOptionsDto,
    ApiRunRequestDto, ApiTaskProgressDto,
};
use crate::state::{AppState, OpenDocumentState};
use mpdf_api_client::{ApiClient, ApiClientConfig, NativeSecretStore, SecretStore};
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::remote_api::{
    now_unix_seconds, ApiPlan, ApiStore, ApiTaskRecord, Consent, Retention, RetentionState,
    TaskState,
};
use sha2::{Digest, Sha256};
use tauri::{Manager, State};

#[tauri::command]
pub fn api_route_options() -> ApiRouteOptionsDto {
    ApiRouteOptionsDto {
        routes: vec!["local".into(), "api".into(), "api_then_local".into()],
        default_route: "local".into(),
    }
}

#[tauri::command]
pub fn api_credential_presence(profile_id: String) -> Result<ApiCredentialPresenceDto, String> {
    if profile_id.is_empty() || profile_id.len() > 256 {
        return Err("invalid credential profile id".into());
    }
    let present = NativeSecretStore
        .get(&profile_id)
        .map_err(|e| e.to_string())?
        .is_some();
    Ok(ApiCredentialPresenceDto {
        profile_id,
        present,
    })
}

fn retention(value: &str) -> Result<Retention, String> {
    match value {
        "delete_after_result" => Ok(Retention::DeleteAfterResult),
        "keep_until_deleted" => Ok(Retention::KeepUntilDeleted),
        _ => Err("invalid retention policy".into()),
    }
}

fn open_document(
    state: &State<'_, AppState>,
    document_id: &str,
) -> Result<OpenDocumentState, String> {
    state
        .document
        .lock()
        .map_err(|_| "document state is unavailable".to_owned())?
        .clone()
        .filter(|document| document.document_id == document_id)
        .ok_or_else(|| "document is no longer open".to_owned())
}

fn build_plan(
    document: &OpenDocumentState,
    request: &ApiPlanRequestDto,
) -> Result<ApiPlan, String> {
    if document.password_protected_session {
        return Err(
            "desktop remote OCR is unavailable for a password-protected session; no upload was made"
                .into(),
        );
    }
    let metadata = std::fs::symlink_metadata(&document.input_path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("source must be a real regular file".into());
    }
    if metadata.len() > mpdf_api_client::MAX_SOURCE_BYTES {
        return Err("source exceeds the 512 MiB upload limit".into());
    }
    let bytes = std::fs::read(&document.input_path).map_err(|e| e.to_string())?;
    ApiPlan::new(
        request.endpoint.clone(),
        request.provider.clone(),
        request.model.clone(),
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        bytes.len() as u64,
        document.page_count,
        request.budget_micros,
        request.currency.clone(),
        retention(&request.retention)?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn api_prepare_plan(
    request: ApiPlanRequestDto,
    state: State<'_, AppState>,
) -> Result<ApiConsentSummaryDto, String> {
    let document = open_document(&state, &request.document_id)?;
    let plan = build_plan(&document, &request)?;
    Ok(ApiConsentSummaryDto {
        plan_digest: plan.plan_digest,
        origin: plan.origin,
        provider: plan.provider,
        model: plan.model,
        source_digest: plan.source_sha256,
        source_bytes: plan.source_bytes,
        page_count: plan.page_count,
        budget_micros: plan.max_cost_micros,
        currency: plan.currency,
        retention: request.retention,
    })
}

#[tauri::command]
pub fn api_cancel_current(state: State<'_, AppState>) -> Result<(), String> {
    let cancellation = state
        .api_cancellation
        .lock()
        .map_err(|_| "API cancellation state is unavailable".to_owned())?
        .clone()
        .ok_or_else(|| "no API task is running".to_owned())?;
    cancellation.cancel();
    Ok(())
}

#[tauri::command]
pub async fn api_run_task(
    request: ApiRunRequestDto,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ApiTaskProgressDto, String> {
    if request.profile_id == "env" {
        return Err("the desktop app requires a native credential profile".into());
    }
    if !matches!(request.route.as_str(), "api" | "api_then_local") {
        return Err("remote execution requires an API route".into());
    }
    let document = open_document(&state, &request.plan.document_id)?;
    let plan = build_plan(&document, &request.plan)?;
    if plan.plan_digest != request.consent {
        return Err("upload consent is stale; review the plan again".into());
    }
    let data_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("api");
    let pdfium = crate::worker::pdfium_config(state.bundled_pdfium_path.as_deref());
    let config = ApiClientConfig {
        endpoint: plan.origin.clone(),
        profile_id: request.profile_id,
        allow_loopback_http: false,
        connect_timeout: Duration::from_secs(5),
        total_timeout: Duration::from_secs(60),
        max_response_bytes: mpdf_api_client::MAX_RESPONSE_BYTES,
        max_retries: mpdf_api_client::MAX_RETRIES,
    };
    let client = ApiClient::new(config, Arc::new(NativeSecretStore)).map_err(|e| e.to_string())?;
    let route = request.route;
    *state
        .api_cancellation
        .lock()
        .map_err(|_| "API cancellation state is unavailable".to_owned())? =
        Some(client.cancellation());
    let result = tauri::async_runtime::spawn_blocking(move || {
        match execute_remote(
            document.clone(),
            plan.clone(),
            request.consent,
            data_root.clone(),
            pdfium.clone(),
            client,
        ) {
            Ok(result) => Ok(result),
            Err(reason) if route == "api_then_local" => {
                execute_local_fallback(document, plan, data_root, pdfium, reason)
            }
            Err(reason) => Err(reason),
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    *state
        .api_cancellation
        .lock()
        .map_err(|_| "API cancellation state is unavailable".to_owned())? = None;
    result
}

fn execute_remote(
    document: OpenDocumentState,
    plan: ApiPlan,
    consent_digest: String,
    data_root: std::path::PathBuf,
    pdfium: mpdf_core::pdfium_backend::PdfiumConfig,
    client: ApiClient<NativeSecretStore>,
) -> Result<ApiTaskProgressDto, String> {
    let source = std::fs::read(&document.input_path).map_err(|e| e.to_string())?;
    let consent = Consent {
        plan_digest: consent_digest,
        consented_at: now_unix_seconds(),
    };
    let created = client
        .create_task(&plan, &consent)
        .map_err(|e| e.to_string())?;
    let receipt = client
        .receipt(&plan, &created.task_id)
        .map_err(|e| e.to_string())?;
    if !created.deduplicated {
        client
            .upload_blob(&plan, &source, &consent)
            .map_err(|e| e.to_string())?;
    }
    client
        .start(&plan, &created.task_id)
        .map_err(|e| e.to_string())?;
    let status = client
        .poll_until_terminal(&plan, &created.task_id)
        .map_err(|e| e.to_string())?;
    let remote = client
        .result(&plan, &created.task_id)
        .map_err(|e| e.to_string())?;
    let package_root = data_root.join("packages").join(&plan.source_sha256);
    if !package_root.exists() {
        let session = PdfDocumentSession::open(
            &document.input_path,
            &PdfOpenOptions {
                password: None,
                pdfium,
                compute_source_hash: true,
            },
        )
        .map_err(|e| e.to_string())?;
        let package = DocumentPackage::create_from_session(&session, Some(document.file_name))
            .map_err(|e| e.to_string())?;
        package.write_to(&package_root).map_err(|e| e.to_string())?;
    }
    let artifact = mpdf_core::remote_api::install_remote_ocr_result(&package_root, &remote)
        .map_err(|e| e.to_string())?;
    let store = ApiStore::open(&data_root.join("jobs.sqlite")).map_err(|e| e.to_string())?;
    let mut retention_state = RetentionState::NotRequested;
    if plan.retention == Retention::DeleteAfterResult {
        retention_state = if client.delete_content(&plan, &created.task_id).is_ok() {
            RetentionState::Acknowledged
        } else {
            RetentionState::Failed
        };
    }
    store
        .put_task(&ApiTaskRecord {
            receipt,
            state: TaskState::ResultInstalled,
            retention: retention_state,
            used_cost_micros: status.used_cost_micros,
            attempts: 1,
            artifact_digest: Some(remote.result_digest),
        })
        .map_err(|e| e.to_string())?;
    let audit_offset = store
        .audit(&created.task_id)
        .map_err(|e| e.to_string())?
        .len();
    for (index, trace) in client.drain_traces().into_iter().enumerate() {
        store
            .append_audit(&mpdf_core::remote_api::ApiAuditEvent {
                schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
                schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
                event_id: format!("{}-desktop-{}", created.task_id, audit_offset + index),
                task_id: created.task_id.clone(),
                kind: if trace.outcome == "retry" {
                    "retry".into()
                } else {
                    trace.kind
                },
                state: TaskState::ResultInstalled,
                retention: retention_state,
                request_digest: trace.request_digest,
                response_digest: trace.response_digest,
                bytes: trace.request_bytes.saturating_add(trace.response_bytes),
                cost_micros: status.used_cost_micros,
                attempt: trace.attempt,
                at: now_unix_seconds(),
                message: Some(trace.outcome),
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(ApiTaskProgressDto {
        task_id: created.task_id,
        state: "result_installed".into(),
        used_cost_micros: status.used_cost_micros,
        budget_micros: plan.max_cost_micros,
        retention: format!("{retention_state:?}").to_lowercase(),
        artifact_path: artifact.display().to_string(),
        fallback_reason: None,
    })
}

fn execute_local_fallback(
    document: OpenDocumentState,
    plan: ApiPlan,
    data_root: std::path::PathBuf,
    pdfium: mpdf_core::pdfium_backend::PdfiumConfig,
    reason: String,
) -> Result<ApiTaskProgressDto, String> {
    let session = PdfDocumentSession::open(
        &document.input_path,
        &PdfOpenOptions {
            password: None,
            pdfium,
            compute_source_hash: true,
        },
    )
    .map_err(|e| e.to_string())?;
    let package_root = data_root.join("packages").join(&plan.source_sha256);
    if !package_root.exists() {
        DocumentPackage::create_from_session(&session, Some(document.file_name))
            .and_then(|package| package.write_to(&package_root))
            .map_err(|e| e.to_string())?;
    }
    let jobs_db = data_root.join("jobs.sqlite");
    let jobs = mpdf_core::jobs::JobStore::open(&jobs_db).map_err(|e| e.to_string())?;
    let mut provider = mpdf_core::ocr::ReferenceOcrProvider;
    let request_id = plan.request_id().map_err(|e| e.to_string())?;
    let job_id = format!("fallback-{}", &request_id[..24]);
    let fingerprint = format!(
        "source={};provider=reference;protocol={}@{};authorized_fallback=true",
        plan.source_sha256,
        mpdf_core::ocr::OCR_PROTOCOL,
        mpdf_core::ocr::OCR_PROTOCOL_VERSION
    );
    let run = mpdf_core::ocr::run_session_durable(
        &session,
        &mut provider,
        &jobs,
        &job_id,
        &fingerprint,
        &package_root,
        "mpdf-desktop-api-fallback",
        mpdf_core::ocr::CANONICAL_OCR_DPI,
    )
    .map_err(|e| e.to_string())?;
    if !run.is_complete(document.page_count) {
        return Err("authorized local fallback did not complete every page".into());
    }
    ApiStore::open(&jobs_db)
        .and_then(|audit| {
            let offset = audit.audit(&job_id)?.len();
            audit.append_audit(&mpdf_core::remote_api::ApiAuditEvent {
                schema: mpdf_core::remote_api::AUDIT_SCHEMA.into(),
                schema_version: mpdf_core::remote_api::AUDIT_VERSION.into(),
                event_id: format!("{job_id}-fallback-{offset}"),
                task_id: job_id.clone(),
                kind: "fallback".into(),
                state: TaskState::ResultInstalled,
                retention: RetentionState::NotRequested,
                request_digest: Some(plan.plan_digest.clone()),
                response_digest: None,
                bytes: 0,
                cost_micros: 0,
                attempt: 0,
                at: now_unix_seconds(),
                message: Some(
                    reason
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(512)
                        .collect(),
                ),
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(ApiTaskProgressDto {
        task_id: job_id,
        state: "result_installed".into(),
        used_cost_micros: 0,
        budget_micros: plan.max_cost_micros,
        retention: "not_requested".into(),
        artifact_path: package_root.display().to_string(),
        fallback_reason: Some(reason),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_plan_is_digest_bound_and_deterministic() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.pdf");
        std::fs::write(&source, b"desktop fixture").unwrap();
        let document = OpenDocumentState {
            document_id: "doc-1".into(),
            file_name: "source.pdf".into(),
            input_path: source.clone(),
            page_count: 2,
            password_protected_session: false,
        };
        let request = ApiPlanRequestDto {
            document_id: "doc-1".into(),
            endpoint: "https://api.example.test/".into(),
            provider: "fixture".into(),
            model: "ocr-1".into(),
            budget_micros: 100,
            currency: "USD".into(),
            retention: "delete_after_result".into(),
        };
        let first = build_plan(&document, &request).unwrap();
        assert_eq!(first, build_plan(&document, &request).unwrap());
        std::fs::write(source, b"changed desktop fixture").unwrap();
        assert_ne!(
            first.plan_digest,
            build_plan(&document, &request).unwrap().plan_digest
        );
    }

    #[test]
    fn desktop_plan_rejects_symlink_source() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.pdf");
            let alias = root.path().join("alias.pdf");
            std::fs::write(&source, b"fixture").unwrap();
            symlink(source, &alias).unwrap();
            let document = OpenDocumentState {
                document_id: "doc-1".into(),
                file_name: "alias.pdf".into(),
                input_path: alias,
                page_count: 1,
                password_protected_session: false,
            };
            let request = ApiPlanRequestDto {
                document_id: "doc-1".into(),
                endpoint: "https://api.example.test/".into(),
                provider: "fixture".into(),
                model: "ocr-1".into(),
                budget_micros: 0,
                currency: "USD".into(),
                retention: "delete_after_result".into(),
            };
            assert!(build_plan(&document, &request).is_err());
        }
    }
}
