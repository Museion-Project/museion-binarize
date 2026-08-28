//! Managed Tauri state.
//!
//! One active document per window, one active processing job at a time —
//! the M4 simplification documented in `docs/desktop.md`. Both are
//! `Mutex`-guarded so command handlers can check-and-set atomically
//! (never "check, then separately set" with a gap another command could
//! land in between).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::worker::WorkerHandle;

/// The one document this window currently has open, if any.
#[derive(Clone)]
pub struct OpenDocumentState {
    pub document_id: String,
    pub file_name: String,
    /// Canonicalized (best-effort) path, used for the default output
    /// filename and so `docs/desktop.md`'s "no source bytes cross the IPC
    /// boundary" rule has a concrete path to point at.
    pub input_path: PathBuf,
    pub page_count: u32,
    pub password_protected_session: bool,
}

/// The one processing job currently running, if any. `cancelled` is
/// shared with the `ProgressReporter` running inside the worker thread —
/// `cancel_processing` flips it directly, with no channel round-trip, so
/// cancellation is observed at the very next check inside the core
/// pipeline rather than waiting behind any queue.
pub struct JobState {
    pub job_id: String,
    pub cancelled: Arc<AtomicBool>,
}

/// The most recently completed size estimate for the open document, kept
/// only so a matching `process` call can report
/// `ProcessingReport::estimate_comparison` automatically — never shown to
/// the frontend directly from here (the frontend keeps its own estimate
/// display state independently; see `docs/desktop.md`). Matched by
/// document id and [`mpdf_core::estimation::settings_fingerprint`]
/// at the moment a conversion actually starts, so a stale estimate from a
/// different document or different settings is never compared against.
#[derive(Clone)]
pub struct CachedEstimate {
    pub document_id: String,
    pub settings_fingerprint: String,
    pub estimated_output_bytes: u64,
}

pub struct AppState {
    pub worker: WorkerHandle,
    pub document: Mutex<Option<OpenDocumentState>>,
    pub job: Mutex<Option<JobState>>,
    /// A cancellation flag for whichever size estimate is currently
    /// in-flight on the worker thread, if any. A new estimate request
    /// flips and replaces this immediately, so a superseded estimate
    /// (settings changed again before it finished) aborts at its next
    /// checkpoint instead of running to completion on stale settings.
    pub estimate_job: Mutex<Option<Arc<AtomicBool>>>,
    pub estimate_cache: Mutex<Option<CachedEstimate>>,
    /// Cancellation handle for the one consented remote OCR request. The
    /// handle contains no credential or document bytes.
    pub api_cancellation: Mutex<Option<mpdf_api_client::Cancellation>>,
    /// The trusted bundled PDFium library path for this packaged build,
    /// if one was found under Tauri's resolved resource directory at
    /// startup — `None` in a development run with no bundled resource.
    /// See `worker::pdfium_config` and `docs/pdfium-bundling.md`
    /// for how this is used (and why an explicit
    /// `MPDF_PDFIUM_LIBRARY` still takes precedence over it, matching
    /// the core resolver's own documented precedence).
    pub bundled_pdfium_path: Option<PathBuf>,
    next_id: AtomicU64,
}

impl AppState {
    pub fn new(bundled_pdfium_path: Option<PathBuf>) -> Self {
        Self {
            worker: WorkerHandle::spawn(bundled_pdfium_path.clone()),
            document: Mutex::new(None),
            job: Mutex::new(None),
            estimate_job: Mutex::new(None),
            estimate_cache: Mutex::new(None),
            api_cancellation: Mutex::new(None),
            bundled_pdfium_path,
            next_id: AtomicU64::new(1),
        }
    }

    /// A process-unique id for a new document or job. Not a security
    /// token — only used so the frontend and a stale async response can
    /// tell "the document/job I meant" from "whatever is current now".
    pub fn new_id(&self, prefix: &str) -> String {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst);
        format!("{prefix}-{n}")
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(None)
    }
}
