//! Progress reporting and cancellation, shared by the CLI and desktop UI.
//!
//! The Milestone 2 PDF pipeline (`pipeline.rs`) will check
//! [`ProgressReporter::is_cancelled`] between major processing stages, per
//! `docs/architecture.md`. Defined now so [`crate::settings`] and future
//! pipeline code can depend on a stable trait.

/// An event describing pipeline progress, emitted at page granularity.
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    Started { total_pages: u32 },
    PageStarted { page: u32 },
    PageFinished { page: u32 },
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
        reporter.report(ProgressEvent::PageFinished { page: 1 });
        reporter.report(ProgressEvent::Finished);
        assert!(!reporter.is_cancelled());
    }
}
