//! Progress reporting and cancellation, shared by the CLI and desktop UI.
//!
//! The Milestone 2 PDF pipeline (`pipeline.rs`) will check
//! [`ProgressReporter::is_cancelled`] between major processing stages, per
//! `docs/architecture.md`. Defined now so [`crate::settings`] and future
//! pipeline code can depend on a stable trait.

/// A stage within the processing of a single page. Emitted so front ends
/// can show what the pipeline is currently doing, not merely how many
/// pages remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingStage {
    Opening,
    Rendering,
    Grayscale,
    Preprocessing,
    Binarization,
    Cleanup,
    Encoding,
    Writing,
    Validating,
}

/// An event describing pipeline progress.
///
/// Page numbers carried by these events are **one-based**, matching what
/// the user sees, not the zero-based indexes used internally.
///
/// Documented ordering: `Started` is emitted once; then for each page a
/// `PageStarted`, zero or more `StageChanged`, and one `PageFinished`;
/// then `Validating` before output validation; and finally exactly one
/// terminal event, either `Finished` or `Cancelled`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    Started { total_pages: u32 },
    PageStarted { page: u32 },
    StageChanged { page: u32, stage: ProcessingStage },
    PageFinished { page: u32, compressed_bytes: u64 },
    Validating,
    Finished,
    Cancelled,
}

/// Implemented by front ends (CLI, Tauri command handler) to receive
/// progress updates and signal cancellation requests back into the core.
///
/// Must be `Send + Sync` because processing may run on a worker thread
/// separate from the thread that owns the UI/CLI event loop.
pub trait ProgressReporter: Send + Sync {
    fn report(&self, event: ProgressEvent);
    fn is_cancelled(&self) -> bool;
}

/// A [`ProgressReporter`] that discards all events and never cancels.
/// Useful for tests and for CLI invocations that don't need progress
/// output (e.g. `analyze`).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgressReporter;

impl ProgressReporter for NullProgressReporter {
    fn report(&self, _event: ProgressEvent) {}
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_reporter_never_cancels_and_accepts_any_event() {
        let reporter = NullProgressReporter;
        assert!(!reporter.is_cancelled());
        reporter.report(ProgressEvent::Started { total_pages: 10 });
        reporter.report(ProgressEvent::PageStarted { page: 1 });
        reporter.report(ProgressEvent::StageChanged {
            page: 1,
            stage: ProcessingStage::Rendering,
        });
        reporter.report(ProgressEvent::PageFinished {
            page: 1,
            compressed_bytes: 42,
        });
        reporter.report(ProgressEvent::Validating);
        reporter.report(ProgressEvent::Finished);
        assert!(!reporter.is_cancelled());
    }
}
