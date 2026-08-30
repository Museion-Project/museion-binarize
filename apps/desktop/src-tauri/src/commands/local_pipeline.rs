//! The desktop main flow: one button, one final PDF.
//!
//! `start_local_pipeline` runs `mpdf_core::orchestrator` on the PDFium worker
//! thread. Everything about the order — original pages are OCR'd first,
//! bookmarks are compiled from that evidence, and only then are the visible
//! pages binarized — lives in the core. This file starts the run, reports
//! stages, cancels, and reads durable status; it decides none of the rules.
//!
//! Resume is not a separate command: the run is keyed to a durable workspace,
//! so starting again with the same workspace reuses every OCR page that was
//! already committed and verified, and picks up where it stopped.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mpdf_core::jobs::JobStore;
use mpdf_core::ocr::{self, OcrEngine};
use mpdf_core::ocr_provider::{credits, OcrProviderMode, BYOK_DISABLED_MESSAGE};
use mpdf_core::orchestrator::{Halt, PipelineStage, ReviewPolicy};
use mpdf_core::progress::{ProgressEvent, ProgressReporter};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::dto::{
    LocalOcrReadinessDto, LocalOcrReadinessRequestDto, LocalPipelineCompletedDto,
    LocalPipelineFailedDto, LocalPipelineRequestDto, LocalPipelineStageDto,
    LocalPipelineStartedDto, UiErrorDto,
};
use crate::errors::{classify_core_error, request_error};
use crate::settings::to_processing_settings;
use crate::state::{AppState, AutoBookmarkState, OperationKind};
use crate::worker::{FinalPdfWork, WorkerCommand};

pub const EVENT_STAGE: &str = "mpdf://pipeline-stage";
pub const EVENT_COMPLETED: &str = "mpdf://pipeline-completed";
pub const EVENT_FAILED: &str = "mpdf://pipeline-failed";
pub const EVENT_CANCELLED: &str = "mpdf://pipeline-cancelled";

fn validated_path(value: &str, what: &str) -> Result<PathBuf, UiErrorDto> {
    if value.trim().is_empty() || value.len() > 4096 || value.bytes().any(|byte| byte == 0) {
        return Err(request_error(
            "invalid_parameter",
            format!("the {what} path is empty or out of range"),
        ));
    }
    Ok(PathBuf::from(value))
}

fn optional_path(value: &Option<String>, what: &str) -> Result<Option<PathBuf>, UiErrorDto> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => validated_path(text, what).map(Some),
    }
}

fn bundled_ocr_runtime(
    app: &AppHandle,
    language_profile: &str,
) -> Result<Option<(PathBuf, PathBuf, PathBuf)>, UiErrorDto> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64-apple-darwin"
    } else {
        return Ok(None);
    };
    let root = app
        .path()
        .resolve("ocr-runtime", tauri::path::BaseDirectory::Resource)
        .map_err(|error| request_error("ocr_runtime_unavailable", error.to_string()))?;
    if !root.is_dir() {
        return Ok(None);
    }
    let runtime = mpdf_core::ocr_runtime::resolve_under(&root, target, language_profile)
        .map_err(|error| request_error("ocr_runtime_unavailable", error))?;
    Ok(Some((runtime.engine, runtime.sidecar, runtime.model_dir)))
}

fn engine_of(value: &Option<String>) -> Result<OcrEngine, UiErrorDto> {
    match value.as_deref() {
        None | Some("") => Ok(OcrEngine::default()),
        Some(name) => OcrEngine::parse(name).ok_or_else(|| {
            request_error("invalid_parameter", format!("unknown OCR engine: {name}"))
        }),
    }
}

fn provider_mode_of(value: &Option<String>) -> Result<OcrProviderMode, UiErrorDto> {
    match value.as_deref() {
        // An absent field is local. An old frontend build, or a request that
        // simply omits the field, can never select a mode that uploads.
        None | Some("") => Ok(OcrProviderMode::Local),
        Some(name) => OcrProviderMode::parse(name).ok_or_else(|| {
            request_error("invalid_parameter", format!("unknown OCR provider: {name}"))
        }),
    }
}

fn review_of(value: &str) -> Result<ReviewPolicy, UiErrorDto> {
    match value {
        "pause" => Ok(ReviewPolicy::PauseForReview),
        "confirmed" => Ok(ReviewPolicy::ContinueWithConfirmed),
        "reviewed" => Ok(ReviewPolicy::UseExistingDecisions),
        other => Err(request_error(
            "invalid_parameter",
            format!("unknown review policy: {other}"),
        )),
    }
}

/// Reports whether the selected local OCR paths can run, and exactly what is
/// missing. The command wrapper below fills empty paths from the verified
/// application resource; explicit advanced paths continue to take precedence.
fn local_ocr_readiness_for(request: LocalOcrReadinessRequestDto) -> LocalOcrReadinessDto {
    let profiles: Vec<String> = ocr::OCR_LANGUAGE_PROFILES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let engine = match request.engine.as_deref() {
        None | Some("") => OcrEngine::default(),
        Some(name) => match OcrEngine::parse(name) {
            Some(engine) => engine,
            None => {
                return LocalOcrReadinessDto {
                    ready: false,
                    engine: name.to_owned(),
                    language_profile: request.language_profile,
                    cleared_for_production: false,
                    missing_files: Vec::new(),
                    model_set: None,
                    model_license: None,
                    diagnostic: "unknown OCR engine".into(),
                    available_profiles: profiles,
                }
            }
        },
    };
    let Some(required) = ocr::required_model_files(&request.language_profile) else {
        return LocalOcrReadinessDto {
            ready: false,
            engine: engine.as_str().into(),
            language_profile: request.language_profile,
            cleared_for_production: engine.cleared_for_production(),
            missing_files: Vec::new(),
            model_set: None,
            model_license: None,
            diagnostic: "unknown language profile".into(),
            available_profiles: profiles,
        };
    };
    let executable = request
        .provider_executable
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let model_dir = request
        .model_dir
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);

    let mut missing = Vec::new();
    let mut model_set = None;
    let mut model_license = None;

    let diagnostic: String = match (&executable, &model_dir) {
        (None, _) => {
            "no OCR sidecar is configured; point it at scripts/ocr/mpdf_ocr_sidecar.py".into()
        }
        (Some(path), _) if !path.is_file() => "the configured OCR sidecar does not exist".into(),
        (_, None) => "no model directory is configured; provision one with \
                      scripts/ocr/provision_models.py (nothing is downloaded automatically)"
            .into(),
        (_, Some(directory)) if !directory.is_dir() => {
            "the configured model directory does not exist".into()
        }
        (Some(_), Some(directory)) => {
            for name in &required {
                if !directory.join(name).is_file() {
                    missing.push(name.clone());
                }
            }
            let manifest = directory.join("manifest.json");
            if let Ok(bytes) = std::fs::read(&manifest) {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    model_set = value
                        .get("model_set")
                        .and_then(|item| item.as_str())
                        .zip(
                            value
                                .get("model_set_version")
                                .and_then(|item| item.as_str()),
                        )
                        .map(|(name, version)| format!("{name} {version}"));
                    model_license = value
                        .get("license")
                        .and_then(|item| item.as_str())
                        .map(str::to_owned);
                }
            }
            if missing.is_empty() {
                "local OCR is ready; no network access is used".into()
            } else {
                format!("{} model file(s) are missing", missing.len())
            }
        }
    };

    let ready = missing.is_empty()
        && executable.as_ref().is_some_and(|path| path.is_file())
        && model_dir.as_ref().is_some_and(|path| path.is_dir());
    LocalOcrReadinessDto {
        ready,
        engine: engine.as_str().into(),
        language_profile: request.language_profile,
        cleared_for_production: engine.cleared_for_production(),
        missing_files: missing,
        model_set,
        model_license,
        diagnostic,
        available_profiles: profiles,
    }
}

/// Nothing here downloads or searches PATH. A packaged build resolves the
/// same verified bundled runtime used by `start_local_pipeline`; a development
/// build with no bundle still reports the explicit configuration it needs.
#[tauri::command]
pub fn local_ocr_readiness(
    mut request: LocalOcrReadinessRequestDto,
    app: AppHandle,
) -> LocalOcrReadinessDto {
    let explicit_sidecar = request
        .provider_executable
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let explicit_models = request
        .model_dir
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if !explicit_sidecar && !explicit_models {
        if let Ok(Some((_engine, sidecar, model_dir))) =
            bundled_ocr_runtime(&app, &request.language_profile)
        {
            request.provider_executable = Some(sidecar.display().to_string());
            request.model_dir = Some(model_dir.display().to_string());
        }
    }
    local_ocr_readiness_for(request)
}

/// Emits nothing itself; it only carries cancellation into the core and
/// forwards the per-page conversion progress the binarization stage produces.
struct PipelineProgress {
    cancelled: Arc<AtomicBool>,
}

impl ProgressReporter for PipelineProgress {
    fn report(&self, _event: ProgressEvent) {}
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[tauri::command]
pub async fn start_local_pipeline(
    request: LocalPipelineRequestDto,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LocalPipelineStartedDto, UiErrorDto> {
    let operation = state
        .try_claim_operation(OperationKind::AutoBookmark)
        .ok_or_else(|| {
            request_error(
                "operation_active",
                "another document operation is running; wait for it to finish",
            )
        })?;

    let output = validated_path(&request.output_path, "output")?;
    let workspace = validated_path(&request.workspace_path, "workspace")?;
    let settings = to_processing_settings(&request.settings)
        .map_err(|message| request_error("invalid_parameter", message))?;
    let engine = engine_of(&request.engine)?;
    let review = review_of(&request.on_review)?;
    let provider_mode = provider_mode_of(&request.ocr_provider_mode)?;
    if provider_mode == OcrProviderMode::GeminiByok {
        // Legacy request compatibility only. Reject before inspecting the
        // credential slot, endpoint, plugin paths, filesystem, or transport.
        return Err(request_error("byok_disabled", BYOK_DISABLED_MESSAGE));
    }
    if provider_mode == OcrProviderMode::Local && request.cloud_consent {
        return Err(request_error(
            "invalid_parameter",
            "cloud consent was given but local processing uploads nothing",
        ));
    }
    if provider_mode.uses_network() && !request.cloud_consent {
        return Err(request_error(
            "cloud_consent_required",
            "cloud OCR uploads a rendered image of every OCR'd page; confirm that before starting",
        ));
    }
    if provider_mode == OcrProviderMode::MpdfCredits {
        if request.max_credits.unwrap_or(0) == 0 {
            return Err(request_error(
                "invalid_parameter",
                "a hard cost limit must be authorized before a paid brokered OCR run can start",
            ));
        }
        return Err(request_error(
            "not_available",
            format!(
                "M PDF Cloud OCR is unavailable: no production backend currently returns the complete coordinate OCR contract; {}",
                credits::release_blockers().join("; ")
            ),
        ));
    }
    // Validate every fallible request field before claiming the per-window
    // pipeline slot. Otherwise a malformed optional path could return below
    // with `auto_bookmark` still populated, making all later starts look busy.
    let explicit_sidecar = optional_path(&request.provider_executable, "OCR sidecar")?;
    let explicit_model_dir = optional_path(&request.model_dir, "model directory")?;
    let (engine_binary, sidecar, model_dir) = match (explicit_sidecar, explicit_model_dir) {
        (Some(sidecar), Some(model_dir)) => (None, Some(sidecar), Some(model_dir)),
        (None, None) => match bundled_ocr_runtime(&app, &request.language_profile)? {
            Some((engine, sidecar, model_dir)) => (Some(engine), Some(sidecar), Some(model_dir)),
            None => (None, None, None),
        },
        (sidecar, model_dir) => (None, sidecar, model_dir),
    };
    if ocr::required_model_files(&request.language_profile).is_none() {
        return Err(request_error(
            "invalid_parameter",
            "unknown OCR language profile",
        ));
    }

    // The source is the document the user opened: the *original* PDF. The
    // frontend never names it, so it can never be redirected at a binarized
    // output.
    let source = {
        let document = state.document.lock().unwrap();
        match document.as_ref() {
            Some(open) if open.document_id == request.document_id => open.input_path.clone(),
            Some(_) => {
                return Err(request_error(
                    "document_stale",
                    "that document is no longer open",
                ))
            }
            None => {
                return Err(request_error(
                    "document_not_open",
                    "open the source PDF before starting",
                ))
            }
        }
    };

    let resumed = workspace.join("ocr/active-run.json").is_file();
    let job_id = {
        let mut slot = state.auto_bookmark.lock().unwrap();
        if slot.is_some() {
            return Err(request_error(
                "pipeline_active",
                "a run is already in progress for this window",
            ));
        }
        // A stable id keyed to the workspace *and the provider mode*, so a
        // resumed run rejoins the same durable OCR job instead of orphaning
        // its checkpoints — and so switching modes starts a new job rather
        // than mixing two kinds of evidence into one.
        let job_id = format!(
            "pipeline-{}-{}",
            provider_mode.id(),
            request.language_profile
        );
        slot.replace(AutoBookmarkState {
            job_id: job_id.clone(),
            document_id: request.document_id.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            durable_workspace: Some(workspace.clone()),
        });
        job_id
    };
    let cancelled = state
        .auto_bookmark
        .lock()
        .unwrap()
        .as_ref()
        .map(|active| active.cancelled.clone())
        .unwrap_or_default();

    let stage_app = app.clone();
    let stage_job = job_id.clone();
    let stage_document = request.document_id.clone();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    state.worker.send(WorkerCommand::FinalPdf {
        request: Box::new(FinalPdfWork {
            source,
            output,
            workspace: workspace.clone(),
            settings,
            language_profile: request.language_profile.clone(),
            provider_mode,
            cloud_consent: request.cloud_consent,
            max_credits: request.max_credits.unwrap_or(0),
            engine,
            engine_binary,
            sidecar,
            model_dir,
            review,
            overwrite: request.overwrite,
            ocr_dpi: request.ocr_dpi.unwrap_or(mpdf_core::ocr::CANONICAL_OCR_DPI),
            job_id: job_id.clone(),
            progress: Box::new(PipelineProgress {
                cancelled: cancelled.clone(),
            }),
            stage: Box::new(move |stage| {
                let _ = stage_app.emit(
                    EVENT_STAGE,
                    LocalPipelineStageDto {
                        job_id: stage_job.clone(),
                        document_id: stage_document.clone(),
                        stage: stage.to_owned(),
                    },
                );
            }),
        }),
        reply: reply_tx,
    });

    let done_app = app.clone();
    let done_job = job_id.clone();
    let document_id = request.document_id.clone();
    tauri::async_runtime::spawn(async move {
        let _operation = operation;
        let outcome = tauri::async_runtime::spawn_blocking(move || reply_rx.recv())
            .await
            .ok()
            .and_then(Result::ok);
        if let Some(state) = done_app.try_state::<AppState>() {
            state.auto_bookmark.lock().unwrap().take();
        }
        match outcome {
            Some(Ok(result)) => {
                // A bookmark safe refusal is a normal completion carrying a
                // warning, so it must not surface as a failed or halted run.
                let status = match &result.halt {
                    None => "completed",
                    Some(Halt::AwaitingReview { .. }) => "awaiting_review",
                };
                let _ = done_app.emit(
                    EVENT_COMPLETED,
                    LocalPipelineCompletedDto {
                        job_id: done_job,
                        document_id,
                        status: status.to_owned(),
                        bookmark_status: result.bookmark_status.as_str().to_owned(),
                        stage_reached: result.stage_reached.as_str().to_owned(),
                        output_path: result.output_path.map(|path| path.display().to_string()),
                        page_count: result.page_count,
                        ocr_pages: result.ocr_pages as u32,
                        ocr_errors: result.ocr_errors as u32,
                        bookmarks_written: result.bookmarks_written as u32,
                        bookmarks_auto_confirmed: result.bookmarks_auto_confirmed as u32,
                        bookmarks_needing_review: result.bookmarks_needing_review as u32,
                        bookmarks_skipped: result.bookmarks_skipped as u32,
                        text_layer_words: result.text_layer_words as u32,
                        refusal_reason: result.safe_refusal_reason,
                        ocr_provenance: result.ocr_provenance,
                        provider_mode: result.provider_mode,
                        // 1-based, matching every other page number the UI
                        // shows.
                        cloud_fallback_pages: result
                            .cloud
                            .fallback_pages
                            .iter()
                            .map(|page| page + 1)
                            .collect(),
                        cloud_input_tokens: result.cloud.usage.input_tokens,
                        cloud_output_tokens: result.cloud.usage.output_tokens,
                        cloud_usage_may_be_incomplete: result.cloud.usage_may_be_incomplete,
                        credits_reserved: result.cloud.reserved_credits,
                        credits_charged: result.cloud.charged_credits,
                        credits_released: result.cloud.released_credits,
                        credits_refunded: result.cloud.refunded_credits,
                        credits_settled: result.cloud.settled,
                        not_production_ready_reason: result.cloud.not_production_ready_reason,
                    },
                );
            }
            Some(Err(mpdf_core::error::CoreError::Cancelled)) => {
                let _ = done_app.emit(
                    EVENT_CANCELLED,
                    LocalPipelineStageDto {
                        job_id: done_job,
                        document_id,
                        stage: "cancelled".to_owned(),
                    },
                );
            }
            Some(Err(error)) => {
                let _ = done_app.emit(
                    EVENT_FAILED,
                    LocalPipelineFailedDto {
                        job_id: done_job,
                        document_id,
                        error: classify_core_error(&error),
                    },
                );
            }
            None => {
                let _ = done_app.emit(
                    EVENT_FAILED,
                    LocalPipelineFailedDto {
                        job_id: done_job,
                        document_id,
                        error: request_error(
                            "internal_error",
                            "the PDFium worker thread stopped responding",
                        ),
                    },
                );
            }
        }
    });

    Ok(LocalPipelineStartedDto {
        job_id,
        document_id: request.document_id,
        workspace_path: workspace.display().to_string(),
        stages: PipelineStage::ORDER
            .iter()
            .map(|stage| stage.as_str().to_owned())
            .collect(),
        resumed,
    })
}

/// Requests cancellation of the in-flight run.
///
/// Two things are flipped: the in-process flag the core checks between
/// stages, and the durable OCR job's cancel bit, so a resumed run does not
/// immediately continue a job the user stopped. Pages already committed are
/// kept — that is what makes the next start a resume rather than a restart.
#[tauri::command]
pub fn cancel_local_pipeline(state: State<'_, AppState>) -> Result<(), UiErrorDto> {
    let (cancelled, durable) = {
        let active = state.auto_bookmark.lock().unwrap();
        let Some(active) = active.as_ref() else {
            return Err(request_error("no_active_run", "no run is in progress"));
        };
        (
            active.cancelled.clone(),
            active
                .durable_workspace
                .clone()
                .map(|workspace| (workspace, active.job_id.clone())),
        )
    };
    cancelled.store(true, Ordering::SeqCst);
    if let Some((workspace, job_id)) = durable {
        let store = JobStore::open(&workspace.join("jobs.sqlite3"))
            .map_err(|error| request_error("input_error", error.to_string()))?;
        store
            .request_cancel(&job_id)
            .map_err(|error| request_error("input_error", error.to_string()))?;
    }
    Ok(())
}

/// Marks the durable OCR job cancelled so a crashed or backgrounded run does
/// not resume on the next start. Separate from [`cancel_local_pipeline`]
/// because the workspace outlives the process.
#[tauri::command]
pub fn cancel_local_pipeline_job(workspace_path: String, job_id: String) -> Result<(), UiErrorDto> {
    let workspace = validated_path(&workspace_path, "workspace")?;
    if job_id.is_empty() || job_id.len() > 256 || job_id.bytes().any(|byte| byte == 0) {
        return Err(request_error("invalid_parameter", "the job id is invalid"));
    }
    let store = JobStore::open(&workspace.join("jobs.sqlite3"))
        .map_err(|error| request_error("input_error", error.to_string()))?;
    store
        .request_cancel(&job_id)
        .map_err(|error| request_error("input_error", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reports_every_missing_model_file() {
        let directory = tempfile::tempdir().unwrap();
        let sidecar = directory.path().join("sidecar.py");
        std::fs::write(&sidecar, b"#!/usr/bin/env python3\n").unwrap();
        // Only one of the profile's files is present.
        std::fs::write(directory.path().join("grc.traineddata"), b"model").unwrap();
        let status = local_ocr_readiness_for(LocalOcrReadinessRequestDto {
            language_profile: "auto".into(),
            engine: None,
            provider_executable: Some(sidecar.display().to_string()),
            model_dir: Some(directory.path().display().to_string()),
        });
        assert!(!status.ready);
        assert!(status.missing_files.contains(&"deu.traineddata".to_owned()));
        assert!(status.missing_files.contains(&"eng.traineddata".to_owned()));
        assert!(status.missing_files.contains(&"osd.traineddata".to_owned()));
        assert!(status.missing_files.contains(&"manifest.json".to_owned()));
    }

    #[test]
    fn readiness_accepts_a_complete_profile_and_reports_the_model_set() {
        let directory = tempfile::tempdir().unwrap();
        let sidecar = directory.path().join("sidecar.py");
        std::fs::write(&sidecar, b"#!/usr/bin/env python3\n").unwrap();
        for name in ocr::required_model_files("auto").unwrap() {
            std::fs::write(directory.path().join(&name), b"model").unwrap();
        }
        std::fs::write(
            directory.path().join("manifest.json"),
            br#"{"model_set":"tessdata_best","model_set_version":"4.1.0","license":"Apache-2.0"}"#,
        )
        .unwrap();
        let status = local_ocr_readiness_for(LocalOcrReadinessRequestDto {
            language_profile: "auto".into(),
            engine: Some("tesseract".into()),
            provider_executable: Some(sidecar.display().to_string()),
            model_dir: Some(directory.path().display().to_string()),
        });
        assert!(status.ready, "{}", status.diagnostic);
        assert!(status.cleared_for_production);
        assert_eq!(status.model_set.as_deref(), Some("tessdata_best 4.1.0"));
        assert_eq!(status.model_license.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn an_evaluation_engine_is_not_cleared_for_production() {
        let status = local_ocr_readiness_for(LocalOcrReadinessRequestDto {
            language_profile: "auto".into(),
            engine: Some("paddleocr".into()),
            provider_executable: None,
            model_dir: None,
        });
        assert!(!status.cleared_for_production);
    }

    #[test]
    fn an_unknown_profile_is_refused_rather_than_defaulted() {
        let status = local_ocr_readiness_for(LocalOcrReadinessRequestDto {
            language_profile: "klingon".into(),
            engine: None,
            provider_executable: None,
            model_dir: None,
        });
        assert!(!status.ready);
        assert_eq!(status.diagnostic, "unknown language profile");
        assert!(status
            .available_profiles
            .contains(&"greek-ancient".to_owned()));
    }

    #[test]
    fn review_policies_map_one_to_one_and_reject_anything_else() {
        assert_eq!(review_of("pause").unwrap(), ReviewPolicy::PauseForReview);
        assert_eq!(
            review_of("confirmed").unwrap(),
            ReviewPolicy::ContinueWithConfirmed
        );
        assert_eq!(
            review_of("reviewed").unwrap(),
            ReviewPolicy::UseExistingDecisions
        );
        assert!(review_of("whatever").is_err());
    }
}
