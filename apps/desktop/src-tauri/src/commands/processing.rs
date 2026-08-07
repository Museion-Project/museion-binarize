//! `start_processing` / `cancel_processing`, and the bridge from the core
//! [`ProgressReporter`] to Tauri events.
//!
//! `start_processing` returns as soon as the job is handed to the PDFium
//! worker thread — it does not wait for the conversion to finish. Progress,
//! completion, cancellation, and failure are all delivered later as events
//! on the `museion://processing-*` namespace (see `docs/desktop.md`).
//! Only one job may be active at a time; a second `start_processing` call
//! while one is running is rejected immediately with a structured error,
//! never silently queued.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use museion_binarize_core::progress::{ProcessingStage, ProgressEvent, ProgressReporter};

use crate::dto::{
    ProcessingCancelledDto, ProcessingCompletedDto, ProcessingFailedDto, ProcessingProgressDto,
    ProcessingStartedDto, StartProcessingRequestDto, UiErrorDto,
};
use crate::errors::{classify_core_error, request_error};
use crate::settings::to_processing_settings;
use crate::state::{AppState, JobState};
use crate::worker::WorkerCommand;

pub const EVENT_PROGRESS: &str = "museion://processing-progress";
pub const EVENT_COMPLETED: &str = "museion://processing-completed";
pub const EVENT_CANCELLED: &str = "museion://processing-cancelled";
pub const EVENT_FAILED: &str = "museion://processing-failed";

#[tauri::command]
pub async fn start_processing(
    request: StartProcessingRequestDto,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ProcessingStartedDto, UiErrorDto> {
    let page_count = {
        let document = state.document.lock().unwrap();
        match document.as_ref() {
            Some(doc) if doc.document_id == request.document_id => {
                // Fast, best-effort pre-check before the worker thread is
                // ever involved. This is a convenience, not the actual
                // guarantee: `process_with_open_session`'s own
                // `check_destination` (run against the session's captured
                // source identity) is what actually protects the source,
                // including the not-yet-existing-path case the M3 review
                // found — see docs/desktop.md, "Path safety".
                let output_path = PathBuf::from(&request.output_path);
                if paths_look_the_same(&doc.input_path, &output_path) {
                    return Err(request_error(
                        "destination_conflict",
                        format!(
                            "the output path must not be the same file as the input ({})",
                            doc.file_name
                        ),
                    ));
                }
                doc.page_count
            }
            Some(_) => {
                return Err(request_error(
                    "document_stale",
                    "that document is no longer open",
                ))
            }
            None => {
                return Err(request_error(
                    "document_not_open",
                    "no document is currently open",
                ))
            }
        }
    };

    {
        let mut job = state.job.lock().unwrap();
        if job.is_some() {
            return Err(request_error(
                "job_active",
                "a processing job is already running",
            ));
        }
        let job_id = state.new_id("job");
        let cancelled = Arc::new(AtomicBool::new(false));
        job.replace(JobState {
            job_id: job_id.clone(),
            cancelled: cancelled.clone(),
        });
        drop(job);

        let settings = match to_processing_settings(&request.settings) {
            Ok(s) => s,
            Err(message) => {
                state.job.lock().unwrap().take();
                return Err(request_error("invalid_settings", message));
            }
        };

        let output = PathBuf::from(&request.output_path);
        let reporter = Box::new(TauriProgressReporter::new(
            app.clone(),
            job_id.clone(),
            page_count,
            cancelled,
        ));

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        state.worker.send(WorkerCommand::Process {
            output: output.clone(),
            settings,
            overwrite: request.overwrite,
            progress: reporter,
            reply: reply_tx,
        });

        let app_for_task = app.clone();
        let job_id_for_task = job_id.clone();
        let output_path_for_task = request.output_path.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = tauri::async_runtime::spawn_blocking(move || reply_rx.recv())
                .await
                .ok()
                .and_then(|r| r.ok());

            // The job is no longer active regardless of outcome, so a new
            // one can start right away.
            if let Some(state) = app_for_task.try_state::<AppState>() {
                state.job.lock().unwrap().take();
            }

            match outcome {
                Some(Ok(report)) => {
                    let _ = app_for_task.emit(
                        EVENT_COMPLETED,
                        ProcessingCompletedDto::from_report(
                            job_id_for_task,
                            output_path_for_task,
                            &report,
                        ),
                    );
                }
                Some(Err(museion_binarize_core::error::CoreError::Cancelled)) => {
                    let _ = app_for_task.emit(
                        EVENT_CANCELLED,
                        ProcessingCancelledDto {
                            job_id: job_id_for_task,
                        },
                    );
                }
                Some(Err(e)) => {
                    let _ = app_for_task.emit(
                        EVENT_FAILED,
                        ProcessingFailedDto {
                            job_id: job_id_for_task,
                            error: classify_core_error(&e),
                        },
                    );
                }
                None => {
                    let _ = app_for_task.emit(
                        EVENT_FAILED,
                        ProcessingFailedDto {
                            job_id: job_id_for_task,
                            error: request_error(
                                "internal_error",
                                "the PDFium worker thread stopped responding",
                            ),
                        },
                    );
                }
            }
        });

        Ok(ProcessingStartedDto { job_id, page_count })
    }
}

/// Requests cancellation of `job_id`. Flips the shared cancellation flag
/// directly — no channel round-trip through the worker thread — so it
/// takes effect at the pipeline's very next cancellation check, per
/// `docs/desktop.md`'s cancellation guarantees.
#[tauri::command]
pub fn cancel_processing(job_id: String, state: State<'_, AppState>) -> Result<(), UiErrorDto> {
    let guard = state.job.lock().unwrap();
    match guard.as_ref() {
        Some(job) if job.job_id == job_id => {
            job.cancelled.store(true, Ordering::SeqCst);
            Ok(())
        }
        _ => Err(request_error(
            "job_not_found",
            "that processing job is no longer active",
        )),
    }
}

/// Bridges the core [`ProgressReporter`] trait to `museion://processing-*`
/// Tauri events. Runs on the PDFium worker thread (see `worker.rs`), so
/// `report`/`is_cancelled` must stay cheap and non-blocking.
struct TauriProgressReporter {
    app: AppHandle,
    job_id: String,
    page_count: u32,
    cancelled: Arc<AtomicBool>,
    current_page: AtomicU32,
}

impl TauriProgressReporter {
    fn new(app: AppHandle, job_id: String, page_count: u32, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            app,
            job_id,
            page_count,
            cancelled,
            current_page: AtomicU32::new(0),
        }
    }

    fn emit_progress(&self, stage: &str, page_number: Option<u32>, stage_fraction: f64) {
        let page_number = page_number.unwrap_or_else(|| self.current_page.load(Ordering::SeqCst));
        let completed_pages = page_number.saturating_sub(1);
        let fraction = if self.page_count == 0 {
            0.0
        } else {
            ((f64::from(completed_pages) + stage_fraction) / f64::from(self.page_count))
                .clamp(0.0, 1.0)
        };
        let _ = self.app.emit(
            EVENT_PROGRESS,
            ProcessingProgressDto {
                job_id: self.job_id.clone(),
                stage: stage.to_string(),
                page_number: Some(page_number).filter(|&n| n > 0),
                page_count: self.page_count,
                fraction,
            },
        );
    }
}

impl ProgressReporter for TauriProgressReporter {
    fn report(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { .. } => {
                self.emit_progress("queued", None, 0.0);
            }
            ProgressEvent::PageStarted { page } => {
                self.current_page.store(page, Ordering::SeqCst);
                self.emit_progress("rendering", Some(page), 0.0);
            }
            ProgressEvent::StageChanged { page, stage } => {
                let (label, fraction) = match stage {
                    ProcessingStage::Opening => ("opening", 0.0),
                    ProcessingStage::Rendering => ("rendering", 0.05),
                    ProcessingStage::Grayscale
                    | ProcessingStage::Preprocessing
                    | ProcessingStage::Binarization => ("binarizing", 0.35),
                    ProcessingStage::Cleanup => ("binarizing", 0.55),
                    ProcessingStage::Encoding => ("encoding", 0.75),
                    ProcessingStage::Writing => ("writing", 0.9),
                    ProcessingStage::Validating => ("validating", 0.95),
                };
                self.emit_progress(label, Some(page), fraction);
            }
            ProgressEvent::PageFinished { page, .. } => {
                self.emit_progress("writing", Some(page), 1.0);
            }
            ProgressEvent::Validating => {
                self.emit_progress("validating", None, 1.0);
            }
            // Finished and Cancelled are reported as terminal events by
            // the async task that awaits the worker's reply, using the
            // ProcessingReport / CoreError it receives — not from here,
            // so the payload can carry the actual result or error.
            ProgressEvent::Finished | ProgressEvent::Cancelled => {}
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Best-effort identity comparison for the fast pre-check above. Not the
/// source of truth — see the comment at its call site.
fn paths_look_the_same(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}
