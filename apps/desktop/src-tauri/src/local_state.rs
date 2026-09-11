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
    /// Canonicalized (best-effort) path, used for the default output
    /// filename and so `docs/desktop.md`'s "no source bytes cross the IPC
    /// boundary" rule has a concrete path to point at.
    pub input_path: PathBuf,
    pub page_count: u32,
    pub password_protected_session: bool,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Processing,
    AutoBookmark,
}

/// A single atomic gate for all operations that use the serialized worker or
/// mutate the active document.
pub struct OperationLease {
    gate: Arc<Mutex<Option<OperationKind>>>,
    kind: OperationKind,
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        if let Ok(mut gate) = self.gate.lock() {
            if gate.as_ref() == Some(&self.kind) {
                *gate = None;
            }
        }
    }
}

pub struct AppState {
    pub worker: WorkerHandle,
    /// Only the newest main preview may enter expensive work.
    pub main_preview_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub document: Mutex<Option<OpenDocumentState>>,
    operation_gate: Arc<Mutex<Option<OperationKind>>>,
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
            main_preview_cancel: Mutex::new(None),
            document: Mutex::new(None),
            operation_gate: Arc::new(Mutex::new(None)),
            bundled_pdfium_path,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn try_claim_operation(&self, kind: OperationKind) -> Option<OperationLease> {
        let mut gate = self.operation_gate.lock().ok()?;
        if gate.is_some() {
            return None;
        }
        *gate = Some(kind);
        Some(OperationLease {
            gate: self.operation_gate.clone(),
            kind,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_gate_is_mutually_exclusive_across_all_long_running_kinds() {
        let state = AppState::default();
        for kind in [OperationKind::Processing, OperationKind::AutoBookmark] {
            let lease = state.try_claim_operation(kind).expect("first claim wins");
            assert!(state.try_claim_operation(kind).is_none());
            assert!(state
                .try_claim_operation(OperationKind::Processing)
                .is_none());
            drop(lease);
            assert!(state.try_claim_operation(kind).is_some());
            // The temporary lease above is dropped at the end of this loop
            // iteration, proving both success and failure release paths.
            state.operation_gate.lock().unwrap().take();
        }
    }
}
