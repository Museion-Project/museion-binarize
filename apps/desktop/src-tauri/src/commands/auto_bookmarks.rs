//! The desktop entry point for automatic table-of-contents bookmarks.
//!
//! One button in the review workbench maps to `start_auto_bookmark`. The
//! command validates the request, claims the window's single automatic-work
//! slot, and hands the run to the PDFium worker thread; stages, the result,
//! failures, and cancellation arrive later on the `mpdf://auto-bookmark-*`
//! event namespace, so the UI never blocks while a long book is matched or
//! a PDF is written.
//!
//! Nothing here re-implements a core rule: staleness, digest binding,
//! no-clobber output, and reopen verification all live in `mpdf_core`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::dto::{
    AutoBookmarkFailedDto, AutoBookmarkRequestDto, AutoBookmarkResultDto, AutoBookmarkStageDto,
    AutoBookmarkStartedDto, BookmarkCandidateDto, UiErrorDto,
};
use crate::errors::{classify_core_error, request_error};
use crate::state::{AppState, AutoBookmarkState};
use crate::worker::{AutoBookmarkWork, WorkerCommand};

pub const EVENT_STAGE: &str = "mpdf://auto-bookmark-stage";
pub const EVENT_COMPLETED: &str = "mpdf://auto-bookmark-completed";
pub const EVENT_FAILED: &str = "mpdf://auto-bookmark-failed";
pub const EVENT_CANCELLED: &str = "mpdf://auto-bookmark-cancelled";

/// Claims this window's single automatic-bookmark slot atomically: a second
/// request while one is in flight is refused, never queued behind it.
fn claim_slot(
    state: &AppState,
    document_id: &str,
) -> Result<(String, Arc<AtomicBool>), UiErrorDto> {
    let mut slot = state.auto_bookmark.lock().unwrap();
    if slot.is_some() {
        return Err(request_error(
            "auto_bookmark_active",
            "an automatic bookmark run is already in progress",
        ));
    }
    let job_id = state.new_id("auto-bookmark");
    let cancelled = Arc::new(AtomicBool::new(false));
    slot.replace(AutoBookmarkState {
        job_id: job_id.clone(),
        document_id: document_id.to_owned(),
        cancelled: cancelled.clone(),
    });
    Ok((job_id, cancelled))
}

fn validated_path(value: &str, what: &str) -> Result<PathBuf, UiErrorDto> {
    if value.trim().is_empty() || value.len() > 4096 || value.bytes().any(|byte| byte == 0) {
        return Err(request_error(
            "invalid_parameter",
            format!("the {what} path is empty or out of range"),
        ));
    }
    Ok(PathBuf::from(value))
}

#[tauri::command]
pub async fn start_auto_bookmark(
    request: AutoBookmarkRequestDto,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AutoBookmarkStartedDto, UiErrorDto> {
    let package_root = validated_path(&request.package_path, "package")?;
    let output = validated_path(&request.output_path, "output")?;
    // The source PDF comes from the open document's own state; the frontend
    // never names it, and it never crosses the boundary in either direction.
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
                    "open the source PDF before compiling its bookmarks",
                ))
            }
        }
    };
    if state.job.lock().unwrap().is_some() {
        return Err(request_error(
            "job_active",
            "a conversion is running; wait for it to finish before compiling bookmarks",
        ));
    }
    if state.api_cancellation.lock().unwrap().is_some() {
        return Err(request_error(
            "api_task_active",
            "a remote OCR task is running; wait for it to finish before compiling bookmarks",
        ));
    }
    let (job_id, cancelled) = claim_slot(&state, &request.document_id)?;

    let stage_app = app.clone();
    let stage_job = job_id.clone();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    state.worker.send(WorkerCommand::AutoBookmark {
        request: AutoBookmarkWork {
            package_root,
            source,
            output,
            overwrite: request.overwrite,
            regenerate: request.regenerate,
            cancelled,
            stage: Box::new(move |stage| {
                let _ = stage_app.emit(
                    EVENT_STAGE,
                    AutoBookmarkStageDto {
                        job_id: stage_job.clone(),
                        stage: stage.to_owned(),
                    },
                );
            }),
        },
        reply: reply_tx,
    });

    let done_app = app.clone();
    let done_job = job_id.clone();
    let document_id = request.document_id.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = tauri::async_runtime::spawn_blocking(move || reply_rx.recv())
            .await
            .ok()
            .and_then(Result::ok);
        if let Some(state) = done_app.try_state::<AppState>() {
            state.auto_bookmark.lock().unwrap().take();
        }
        match outcome {
            Some(Ok(result)) => {
                let _ = done_app.emit(
                    EVENT_COMPLETED,
                    AutoBookmarkResultDto {
                        job_id: done_job,
                        document_id,
                        mode: result.mode.to_owned(),
                        status: result.status.to_owned(),
                        toc_page_count: result.toc_page_count,
                        parsed_entries: result.parsed_entries,
                        auto_confirmed: result.auto_confirmed,
                        needs_review: result.needs_review,
                        skipped: result.skipped,
                        written_bookmarks: result.written_bookmarks,
                        safe_refusal_reason: result.safe_refusal_reason,
                        report_path: result.report_path.display().to_string(),
                        output_path: result.output_path.map(|path| path.display().to_string()),
                    },
                );
            }
            Some(Err(mpdf_core::error::CoreError::Cancelled)) => {
                let _ = done_app.emit(
                    EVENT_CANCELLED,
                    AutoBookmarkStageDto {
                        job_id: done_job,
                        stage: "cancelled".to_owned(),
                    },
                );
            }
            Some(Err(error)) => {
                let _ = done_app.emit(
                    EVENT_FAILED,
                    AutoBookmarkFailedDto {
                        job_id: done_job,
                        error: classify_core_error(&error),
                    },
                );
            }
            None => {
                let _ = done_app.emit(
                    EVENT_FAILED,
                    AutoBookmarkFailedDto {
                        job_id: done_job,
                        error: request_error(
                            "internal_error",
                            "the PDFium worker thread stopped responding",
                        ),
                    },
                );
            }
        }
    });

    Ok(AutoBookmarkStartedDto {
        job_id,
        document_id: request.document_id,
    })
}

/// Cancels the run named by `job_id`, but only for the document that
/// started it: a stale window must never cancel the run belonging to a
/// document opened after it.
#[tauri::command]
pub fn cancel_auto_bookmark(
    job_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> Result<(), UiErrorDto> {
    let slot = state.auto_bookmark.lock().unwrap();
    match slot.as_ref() {
        Some(active) if active.job_id == job_id && active.document_id == document_id => {
            active.cancelled.store(true, Ordering::SeqCst);
            Ok(())
        }
        _ => Err(request_error(
            "job_stale",
            "that automatic bookmark run is no longer active",
        )),
    }
}

/// Loads the effective bookmark tree for a package as project-owned DTOs.
#[tauri::command]
pub fn load_bookmark_tree(package_path: String) -> Result<Vec<BookmarkCandidateDto>, UiErrorDto> {
    let root = validated_path(&package_path, "package")?;
    let snapshot =
        mpdf_core::bookmarks::load_snapshot(&root).map_err(|e| classify_core_error(&e))?;
    let reviews = mpdf_core::bookmarks::load_reviews(&root, &snapshot)
        .map_err(|e| classify_core_error(&e))?;
    let effective = mpdf_core::bookmarks::effective(&snapshot, &reviews)
        .map_err(|e| classify_core_error(&e))?;
    Ok(effective.iter().map(BookmarkCandidateDto::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_must_be_present_and_bounded() {
        assert!(validated_path("", "package").is_err());
        assert!(validated_path("   ", "package").is_err());
        assert!(validated_path(&"a".repeat(5_000), "package").is_err());
        assert!(validated_path("/tmp/book\0.mdp", "package").is_err());
        assert!(validated_path("/tmp/book.mdp", "package").is_ok());
    }

    #[test]
    fn only_one_automatic_run_may_be_claimed_at_a_time() {
        let state = AppState::default();
        let (job_id, cancelled) = claim_slot(&state, "document-1").expect("the first run claims");
        let refused = claim_slot(&state, "document-1").expect_err("a second run is refused");
        assert_eq!(refused.code, "auto_bookmark_active");
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(state
            .auto_bookmark
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| active.job_id == job_id));
        state.auto_bookmark.lock().unwrap().take();
        assert!(claim_slot(&state, "document-2").is_ok());
    }

    #[test]
    fn the_error_for_a_stale_document_names_a_stable_code() {
        let error = request_error("document_stale", "that document is no longer open");
        assert_eq!(error.code, "document_stale");
        let json = serde_json::to_string(&error).unwrap();
        assert!(!json.contains("password"));
    }
}
