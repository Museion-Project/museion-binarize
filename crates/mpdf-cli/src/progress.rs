//! Human progress reporting, always to stderr.
//!
//! stdout is reserved for exactly one thing per invocation: either the
//! human summary or, in `--json` mode, exactly one JSON document. Since
//! progress lines only ever go to stderr, they can never corrupt a
//! redirected `--json` stdout stream regardless of `--quiet`.

use std::io::Write;

use mpdf_core::progress::{ProgressEvent, ProgressReporter};

pub struct StderrProgress {
    quiet: bool,
}

impl StderrProgress {
    pub fn new(quiet: bool) -> Self {
        Self { quiet }
    }
}

impl ProgressReporter for StderrProgress {
    fn report(&self, event: ProgressEvent) {
        if self.quiet {
            return;
        }
        let mut err = std::io::stderr();
        let _ = match event {
            ProgressEvent::Started { total_pages } => {
                writeln!(err, "Processing {total_pages} page(s)...")
            }
            ProgressEvent::PageStarted { page } => write!(err, "  page {page}: "),
            ProgressEvent::StageChanged { stage, .. } => write!(err, "{stage:?} "),
            ProgressEvent::PageFinished {
                compressed_bytes, ..
            } => writeln!(err, "-> {compressed_bytes} bytes"),
            ProgressEvent::Validating => writeln!(err, "Validating output..."),
            ProgressEvent::Finished => writeln!(err, "Done."),
            ProgressEvent::Cancelled => writeln!(err, "Cancelled."),
        };
        let _ = err.flush();
    }

    fn is_cancelled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_reports_cancellation_on_its_own() {
        assert!(!StderrProgress::new(false).is_cancelled());
        assert!(!StderrProgress::new(true).is_cancelled());
    }

    #[test]
    fn quiet_and_verbose_both_accept_every_event_without_panicking() {
        for quiet in [false, true] {
            let progress = StderrProgress::new(quiet);
            progress.report(ProgressEvent::Started { total_pages: 3 });
            progress.report(ProgressEvent::PageStarted { page: 1 });
            progress.report(ProgressEvent::StageChanged {
                page: 1,
                stage: mpdf_core::progress::ProcessingStage::Rendering,
            });
            progress.report(ProgressEvent::PageFinished {
                page: 1,
                compressed_bytes: 10,
            });
            progress.report(ProgressEvent::Validating);
            progress.report(ProgressEvent::Finished);
        }
    }
}
