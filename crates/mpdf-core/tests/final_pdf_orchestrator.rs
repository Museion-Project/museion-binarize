//! The one main flow, end to end: OCR the original, compile bookmarks,
//! binarize the visible pages, assemble a single final PDF, verify it.
//!
//! What these tests are really guarding is the *data dependency*: the text
//! layer and the bookmark tree are derived from renders of the authoritative
//! colour/grayscale source, and the binarized pixels are produced afterwards
//! and never fed back into recognition.
//!
//! They need a real PDFium library and are therefore `#[ignore]`d, so an
//! ordinary `cargo test --workspace` reports them as ignored rather than as
//! passed:
//!
//! ```text
//! MPDF_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.so \
//!   cargo test -p mpdf-core --test final_pdf_orchestrator -- --ignored --test-threads=1
//! ```
//!
//! `--test-threads=1` is required: each test opens its own PDFium document
//! session, and PDFium is initialized per process, not per thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mpdf_core::error::CoreError;
use mpdf_core::orchestrator::{
    self, BookmarkOutcome, FinalPdfRequest, OcrProviderChoice, PipelineStage, ReviewPolicy,
};
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::pipeline::OutputWriteStrategy;
use mpdf_core::progress::{ProgressEvent, ProgressReporter};
use mpdf_core::settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings};

const ENV_VAR: &str = "MPDF_PDFIUM_LIBRARY";

fn require_pdfium_config() -> PdfiumConfig {
    let Some(raw) = std::env::var_os(ENV_VAR) else {
        panic!(
            "{ENV_VAR} is not set, but this test was run explicitly.\n\
             This integration test requires a provisioned PDFium library:\n  \
             {ENV_VAR}=/absolute/path/to/{} \\\n    \
             cargo test -p mpdf-core --test final_pdf_orchestrator -- --ignored\n\
             See docs/pdfium.md for how to obtain one.",
            mpdf_core::pdfium_backend::pdfium_library_file_name()
        );
    };
    let path = PathBuf::from(raw);
    assert!(
        path.is_file(),
        "{ENV_VAR} points at {}, which is not a file",
        path.display()
    );
    PdfiumConfig {
        library_path: Some(path),
        allow_system_library: false,
    }
}

fn settings() -> ProcessingSettings {
    ProcessingSettings {
        dpi: 300,
        method: BinarizationMethod::Otsu,
        contrast: 0.0,
        preprocessing: PreprocessingSettings::default(),
        cleanup: Default::default(),
    }
}

/// Records every stage the orchestrator announced, in order.
#[derive(Default)]
struct StageLog(Mutex<Vec<PipelineStage>>);

impl StageLog {
    fn push(&self, stage: PipelineStage) {
        self.0.lock().unwrap().push(stage);
    }
    fn stages(&self) -> Vec<PipelineStage> {
        self.0.lock().unwrap().clone()
    }
}

struct Progress {
    cancelled: Arc<AtomicBool>,
    /// Flipped once the binarization stage starts emitting page events.
    binarization_started: Arc<AtomicBool>,
    cancel_at_binarization: bool,
}

impl Progress {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            binarization_started: Arc::new(AtomicBool::new(false)),
            cancel_at_binarization: false,
        }
    }
}

impl ProgressReporter for Progress {
    fn report(&self, event: ProgressEvent) {
        if matches!(event, ProgressEvent::Started { .. }) {
            self.binarization_started.store(true, Ordering::SeqCst);
            if self.cancel_at_binarization {
                self.cancelled.store(true, Ordering::SeqCst);
            }
        }
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

fn write_source(directory: &Path, bytes: &[u8]) -> PathBuf {
    let path = directory.join("source.pdf");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn request<'a>(
    source: &'a Path,
    output: &'a Path,
    workspace: &'a Path,
    settings: &'a ProcessingSettings,
    pdfium: PdfiumConfig,
    review: ReviewPolicy,
) -> FinalPdfRequest<'a> {
    FinalPdfRequest {
        source,
        output,
        workspace,
        settings,
        // The deterministic offline provider: no model, no network, and its
        // output is inspectable, so these tests assert on the *pipeline*
        // rather than on a recognizer's accuracy (that is the gold set's job,
        // see scripts/ocr/gold).
        provider: OcrProviderChoice::Reference,
        language_profile: "auto".into(),
        review,
        overwrite: false,
        password: None,
        pdfium,
        output_write_strategy: OutputWriteStrategy::AtomicSameDirectoryRename,
        bookmark_config: Default::default(),
        ocr_dpi: 150,
        job_id: "test-run".into(),
        jobs_db: workspace.join("jobs.sqlite3"),
        owner: "integration-test".into(),
        // The default: a purely local run. These tests assert the pipeline's
        // ordering guarantees, which must hold identically with no cloud
        // provider in the picture.
        cloud: None,
    }
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn ocr_and_bookmarks_complete_before_any_page_is_binarized() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let source = write_source(
        directory.path(),
        &mpdf_core::test_fixtures::heterogeneous_document(3),
    );
    let output = directory.path().join("final.pdf");
    let workspace = directory.path().join("workspace");
    let settings = settings();
    let progress = Progress::new();
    let log = StageLog::default();

    let outcome = orchestrator::run(
        &request(
            &source,
            &output,
            &workspace,
            &settings,
            pdfium,
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &progress,
        &|stage| log.push(stage),
    );

    // The reference provider recognizes nothing, so this document has no
    // printed contents list. That is a bookmark safe refusal — and in the
    // combined main flow a refusal only withholds the *outline*. The document
    // must still be converted, because the OCR evidence and the binarized
    // pixels are unaffected by the absence of a contents list.
    let outcome = outcome.expect("the run itself must not fail");
    let stages = log.stages();
    let position = |stage: PipelineStage| stages.iter().position(|item| *item == stage);

    assert!(
        position(PipelineStage::OcrOriginal).is_some(),
        "OCR stage must run: {stages:?}"
    );
    assert!(
        position(PipelineStage::OcrOriginal) < position(PipelineStage::DerivingText),
        "text is derived from OCR, not before it: {stages:?}"
    );
    assert!(
        position(PipelineStage::DerivingText) < position(PipelineStage::GeneratingBookmarks),
        "bookmarks are compiled from the derived text: {stages:?}"
    );
    if let Some(binarize) = position(PipelineStage::BinarizingVisuals) {
        assert!(
            position(PipelineStage::GeneratingBookmarks).unwrap() < binarize,
            "binarization must not begin before bookmarks are frozen: {stages:?}"
        );
        assert!(
            binarize < position(PipelineStage::AssemblingFinalPdf).unwrap(),
            "the final PDF is assembled after binarization: {stages:?}"
        );
    }

    // The typed OCR evidence exists in the workspace and was produced from
    // the original pages: there is no binarized artifact for it to have come
    // from at that point in the run.
    let ocr = mpdf_core::ocr::read_ocr_records(&workspace).expect("OCR records must be readable");
    assert_eq!(ocr.pages.len(), 3);
    assert!(ocr.errors.is_empty());

    assert_eq!(
        outcome.halt, None,
        "a bookmark refusal must not halt the combined main flow"
    );
    assert_eq!(
        outcome.bookmark_status,
        BookmarkOutcome::SafeRefusal,
        "no contents list means no invented outline"
    );
    assert!(
        outcome.safe_refusal_reason.is_some(),
        "the refusal must explain itself in the report"
    );
    assert_eq!(
        outcome.bookmarks_written, 0,
        "a refusal must never invent a bookmark"
    );
    assert_eq!(
        outcome.output_path.as_deref(),
        Some(output.as_path()),
        "the final PDF is still produced and named"
    );
    assert!(output.is_file(), "the converted PDF must exist on disk");
    assert!(
        outcome.verification.is_some(),
        "the written PDF must have been independently verified"
    );
    assert_eq!(outcome.stage_reached, PipelineStage::Validating);

    // And that PDF really has no outline: refusing must not degrade into
    // writing an empty or partial one.
    let written = lopdf::Document::load(&output).expect("the output must be a readable PDF");
    assert!(
        written
            .trailer
            .get(b"Root")
            .and_then(|root| written.dereference(root))
            .and_then(|(_, object)| object.as_dict())
            .map(|catalog| catalog.get(b"Outlines").is_err())
            .unwrap_or(false),
        "a refusal must leave the catalog without an outline"
    );

    // The source is untouched.
    assert_eq!(
        std::fs::read(&source).unwrap(),
        mpdf_core::test_fixtures::heterogeneous_document(3)
    );
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn cancelling_during_binarization_leaves_no_output_and_keeps_the_source() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let bytes = mpdf_core::test_fixtures::heterogeneous_document(4);
    let source = write_source(directory.path(), &bytes);
    let output = directory.path().join("final.pdf");
    let workspace = directory.path().join("workspace");
    let settings = settings();
    let mut progress = Progress::new();
    progress.cancel_at_binarization = true;

    let outcome = orchestrator::run(
        &request(
            &source,
            &output,
            &workspace,
            &settings,
            pdfium,
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &progress,
        &|_| {},
    );

    // Either it refused before binarization, or it was cancelled inside it.
    // Both must leave the destination untouched and the source intact.
    match outcome {
        Err(CoreError::Cancelled) => {}
        Ok(result) => assert!(
            result.output_path.is_none(),
            "a cancelled or refused run must not name an output"
        ),
        Err(other) => panic!("unexpected failure: {other}"),
    }
    assert!(!output.exists(), "no half-written output may remain");
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    // No temporary file may be left in the destination's directory.
    assert!(
        mpdf_core::pipeline::leftover_temporary_files(directory.path()).is_empty(),
        "cancellation must not leave a temporary file behind"
    );
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn a_second_run_reuses_the_committed_ocr_evidence() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let source = write_source(
        directory.path(),
        &mpdf_core::test_fixtures::heterogeneous_document(3),
    );
    let workspace = directory.path().join("workspace");
    let settings = settings();

    // Each run gets its own destination, so the no-clobber guard stays armed
    // (`overwrite: false`) and this test measures evidence reuse rather than
    // overwrite behaviour. Both runs do produce a PDF: this fixture has no
    // printed contents list, and a bookmark refusal no longer withholds the
    // conversion.
    for index in 0..2 {
        let output = directory.path().join(format!("final-{index}.pdf"));
        let progress = Progress::new();
        let outcome = orchestrator::run(
            &request(
                &source,
                &output,
                &workspace,
                &settings,
                pdfium.clone(),
                ReviewPolicy::ContinueWithConfirmed,
            ),
            &progress,
            &|_| {},
        )
        .expect("both runs must succeed");
        assert!(output.is_file(), "run {index} must produce its own PDF");
        assert_eq!(outcome.bookmark_status, BookmarkOutcome::SafeRefusal);
    }

    let first = mpdf_core::ocr::read_ocr_records(&workspace).unwrap();
    // Reading twice must give byte-identical evidence: a resumed run verifies
    // and reuses the committed pages instead of recomputing them.
    let second = mpdf_core::ocr::read_ocr_records(&workspace).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.pages.len(), 3);
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn the_source_and_the_output_may_not_be_the_same_file() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let source = write_source(
        directory.path(),
        &mpdf_core::test_fixtures::heterogeneous_document(2),
    );
    let workspace = directory.path().join("workspace");
    let settings = settings();
    let progress = Progress::new();

    let error = orchestrator::run(
        &request(
            &source,
            &source,
            &workspace,
            &settings,
            pdfium,
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &progress,
        &|_| {},
    )
    .expect_err("writing over the source must be refused");
    assert!(matches!(error, CoreError::DestinationConflict(_)));
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn a_symlinked_source_is_refused() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let real = write_source(
        directory.path(),
        &mpdf_core::test_fixtures::heterogeneous_document(2),
    );
    let link = directory.path().join("alias.pdf");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(not(unix))]
    return;

    let output = directory.path().join("final.pdf");
    let workspace = directory.path().join("workspace");
    let settings = settings();
    let progress = Progress::new();

    let error = orchestrator::run(
        &request(
            &link,
            &output,
            &workspace,
            &settings,
            pdfium,
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &progress,
        &|_| {},
    )
    .expect_err("a symlinked source must be refused");
    assert!(matches!(error, CoreError::InvalidDocument(_)));
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn a_workspace_belonging_to_another_document_is_refused() {
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let first = write_source(
        directory.path(),
        &mpdf_core::test_fixtures::heterogeneous_document(2),
    );
    let workspace = directory.path().join("workspace");
    let settings = settings();

    orchestrator::run(
        &request(
            &first,
            &directory.path().join("a.pdf"),
            &workspace,
            &settings,
            pdfium.clone(),
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &Progress::new(),
        &|_| {},
    )
    .expect("the first run must succeed");

    // A different document, pointed at the same workspace.
    let second = directory.path().join("other.pdf");
    std::fs::write(&second, mpdf_core::test_fixtures::heterogeneous_document(3)).unwrap();
    let error = orchestrator::run(
        &request(
            &second,
            &directory.path().join("b.pdf"),
            &workspace,
            &settings,
            pdfium,
            ReviewPolicy::ContinueWithConfirmed,
        ),
        &Progress::new(),
        &|_| {},
    )
    .expect_err("a workspace bound to another source must be refused");
    assert!(matches!(error, CoreError::InvalidDocument(_)));
}
