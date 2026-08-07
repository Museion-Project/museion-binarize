//! `start_estimate`: experimental output-size estimation.
//!
//! Unlike `start_processing`, this command awaits the worker thread's
//! reply directly and returns the result as its own return value — no
//! `museion://estimate-*` events. Estimation is bounded (a handful of
//! sampled pages, not the whole document) and fast enough that the same
//! request/response pattern `render_preview` already uses is a better
//! fit than `start_processing`'s fire-and-forget/event bridge. Staleness
//! is handled the same way preview staleness is: the frontend keeps a
//! generation counter and discards a response whose `requestId` is no
//! longer the latest one it issued.
//!
//! A *new* estimate request cancels whichever estimate is currently
//! in-flight on the worker thread (see `AppState::estimate_job`), so a
//! settings change does not leave a stale computation running to
//! completion before the fresher one even starts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::State;

use museion_binarize_core::progress::{ProgressEvent, ProgressReporter};

use crate::dto::{EstimateRequestDto, EstimateResultDto, UiErrorDto};
use crate::errors::{classify_core_error, request_error};
use crate::settings::to_processing_settings;
use crate::state::{AppState, CachedEstimate};
use crate::worker::WorkerCommand;

#[tauri::command]
pub async fn start_estimate(
    request: EstimateRequestDto,
    state: State<'_, AppState>,
) -> Result<EstimateResultDto, UiErrorDto> {
    {
        let document = state.document.lock().unwrap();
        match document.as_ref() {
            Some(doc) if doc.document_id == request.document_id => {}
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
    }

    // Estimation and a real conversion both need the serialized PDFium
    // worker thread; running an estimate while a conversion is in
    // progress would only queue behind it uselessly and confuse the UI
    // about which operation is "the" one running.
    if state.job.lock().unwrap().is_some() {
        return Err(request_error(
            "job_active",
            "a processing job is running; estimation is unavailable until it finishes",
        ));
    }

    let settings = to_processing_settings(&request.settings)
        .map_err(|message| request_error("invalid_settings", message))?;

    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut guard = state.estimate_job.lock().unwrap();
        if let Some(previous) = guard.take() {
            // Supersede, don't queue: the previous estimate's next
            // cancellation checkpoint will now abort it.
            previous.store(true, Ordering::SeqCst);
        }
        *guard = Some(cancelled.clone());
    }

    let reporter: Box<dyn ProgressReporter> = Box::new(CancellableReporter {
        cancelled: cancelled.clone(),
    });
    let samples = request.samples;
    let outcome = state
        .worker
        .call(move |reply| WorkerCommand::Estimate {
            settings,
            samples,
            progress: reporter,
            reply,
        })
        .await;

    // Only clear the slot if it is still *this* request's flag — a newer
    // request may already have replaced it while this one was running.
    {
        let mut guard = state.estimate_job.lock().unwrap();
        if guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &cancelled))
        {
            *guard = None;
        }
    }

    let report = outcome.map_err(|e| classify_core_error(&e))?;

    *state.estimate_cache.lock().unwrap() = Some(CachedEstimate {
        document_id: request.document_id,
        settings_fingerprint: report.settings_fingerprint.clone(),
        estimated_output_bytes: report.estimated_output_bytes,
    });

    Ok(EstimateResultDto::build(request.request_id, &report))
}

/// A minimal [`ProgressReporter`] for estimation: no Tauri events (the
/// command's own return value carries the result), just a cancellation
/// flag another, newer `start_estimate` call can flip.
struct CancellableReporter {
    cancelled: Arc<AtomicBool>,
}

impl ProgressReporter for CancellableReporter {
    fn report(&self, _event: ProgressEvent) {}

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
