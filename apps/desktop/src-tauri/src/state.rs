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

pub struct AppState {
    pub worker: WorkerHandle,
    pub document: Mutex<Option<OpenDocumentState>>,
    pub job: Mutex<Option<JobState>>,
    next_id: AtomicU64,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            worker: WorkerHandle::spawn(),
            document: Mutex::new(None),
            job: Mutex::new(None),
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
        Self::new()
    }
}
