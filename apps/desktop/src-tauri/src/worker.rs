//! The dedicated PDFium worker thread.
//!
//! `pdfium-render`'s bindings are guarded by a mutex (the `thread_safe`
//! feature — see `docs/adr/0001-pdfium-runtime-binding.md`), but this
//! crate makes no assumption beyond that: `PdfDocumentSession` and the
//! `PdfDocument` it holds never leave the single OS thread that owns
//! them. Every operation that touches PDFium — opening a document,
//! rendering a page, running a conversion — is a message sent to this
//! thread and executed there, one at a time, in the order received. This
//! is what "PDFium operations remain serialized" means concretely in this
//! codebase, and it holds without requiring any `Send`/`Sync` claim about
//! `pdfium-render` types themselves.
//!
//! A long-running `Process` command occupies the worker thread for the
//! whole conversion, which is intentional: cancellation is delivered out
//! of band (an `Arc<AtomicBool>` the caller flips directly — see
//! `commands::processing`), not as a message queued behind the job, so it
//! takes effect at the next cancellation check inside the core pipeline
//! without waiting for a queue slot.

use std::path::PathBuf;
use std::thread;
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};

use mpdf_core::document::PdfDocumentInfo;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::error::{CoreError, Result as CoreResult};
use mpdf_core::estimation::SizeEstimateReport;
use mpdf_core::image_pipeline::process_rendered_page;
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::pipeline::{self, EstimationOptions, PdfProcessingOptions, ProcessingReport};
use mpdf_core::progress::ProgressReporter;
use mpdf_core::settings::ProcessingSettings;
use mpdf_core::page_selection::PageSelection;

/// A single-use reply channel back to the command handler that issued a
/// [`WorkerCommand`]. An ordinary `std::sync::mpsc` sender rather than an
/// async channel: the worker thread is a plain OS thread with a blocking
/// receive loop, not an async task, and the command side bridges the
/// blocking receive with `tauri::async_runtime::spawn_blocking` (see
/// `commands::*`) instead of pulling in an async channel dependency for
/// this one use.
pub type Reply<T> = std::sync::mpsc::Sender<CoreResult<T>>;

/// What the worker rendered for one preview/thumbnail request.
pub struct RenderedPage {
    pub image: image::DynamicImage,
}

pub struct OpenedDocument {
    pub source_sha256: String,
    pub info: PdfDocumentInfo,
    pub pdfium_library: String,
}

pub enum WorkerCommand {
    Open {
        path: PathBuf,
        password: Option<String>,
        reply: Reply<OpenedDocument>,
    },
    /// Drops the open session, if any. No reply: closing is always
    /// immediate from the caller's point of view.
    Close,
    RenderPage {
        page_index: u32,
        dpi: u16,
        /// `Some` renders through the real processing pipeline at these
        /// settings; `None` renders the untouched rasterized page.
        processed: Option<ProcessingSettings>,
        reply: Reply<RenderedPage>,
    },
    PreviewPage {
        page_index: u32,
        dpi: u16,
        processed: Option<ProcessingSettings>,
        expected_source: PathBuf,
        cancelled: Arc<AtomicBool>,
        reply: Reply<RenderedPage>,
    },
    ProcessSelected {
        output: PathBuf,
        settings: ProcessingSettings,
        selection: PageSelection,
        progress: Box<dyn ProgressReporter>,
        reply: Reply<ProcessingReport>,
    },
    Process {
        output: PathBuf,
        settings: ProcessingSettings,
        overwrite: bool,
        prior_estimate: Option<pipeline::PriorEstimate>,
        progress: Box<dyn ProgressReporter>,
        reply: Reply<ProcessingReport>,
    },
    /// Compiles bookmarks from MDP/OCR evidence and, when anything reaches
    /// the deterministic gate, writes and verifies an outlined PDF. Runs on
    /// the worker thread because its output verification reopens the finished
    /// file with PDFium, which this crate keeps serialized.
    AutoBookmark {
        request: AutoBookmarkWork,
        reply: Reply<AutoBookmarkOutcome>,
    },
    /// The whole main flow, in `mpdf_core::orchestrator`'s fixed order:
    /// OCR the original pages, derive the text layer, compile bookmarks,
    /// binarize the visible pages, and assemble one final PDF.
    ///
    /// It runs on this thread because every PDFium step it contains -- the
    /// source session, the conversion, and the reopen verification -- must
    /// stay serialized with everything else in this crate.
    FinalPdf {
        /// Boxed: this variant carries the whole run configuration — source,
        /// output, settings, language, provider mode, cloud policy — and it
        /// would otherwise set the size of every message the worker channel
        /// ever moves.
        request: Box<FinalPdfWork>,
        reply: Reply<mpdf_core::orchestrator::FinalPdfOutcome>,
    },
    /// Renders, processes, and CCITT-encodes a deterministic sample of
    /// pages to estimate the converted output's size — never writes or
    /// validates an output PDF. See `docs/size-estimation.md`.
    Estimate {
        settings: ProcessingSettings,
        samples: u32,
        progress: Box<dyn ProgressReporter>,
        reply: Reply<SizeEstimateReport>,
    },
}

/// Everything the worker needs for one automatic bookmark run. The source
/// path comes from the open document's own state, never from the frontend.
pub struct AutoBookmarkWork {
    pub package_root: PathBuf,
    pub source: PathBuf,
    pub output: PathBuf,
    pub overwrite: bool,
    pub regenerate: bool,
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub stage: Box<dyn Fn(&str) + Send>,
}

/// Everything the worker needs for one full main-flow run.
///
/// The source path comes from the open document's own state. It is the
/// *original* PDF: the desktop app has no way to hand this a binarized
/// derivative, which is the point.
pub struct FinalPdfWork {
    pub source: PathBuf,
    pub output: PathBuf,
    pub workspace: PathBuf,
    pub settings: ProcessingSettings,
    pub language_profile: String,
    /// Which execution mode the user explicitly chose. `Local` unless the
    /// provider picker was moved *and* the cloud consent box was ticked; the
    /// frontend cannot set this on its own, because `cloud_consent` below is
    /// re-checked here rather than trusted.
    pub provider_mode: mpdf_core::ocr_provider::OcrProviderMode,
    pub cloud_consent: bool,
    pub max_credits: u64,
    /// Native process configuration only, never a renderer-supplied runtime path.
    pub broker_test_config: Option<PathBuf>,
    pub engine: mpdf_core::ocr::OcrEngine,
    pub engine_binary: Option<PathBuf>,
    pub sidecar: Option<PathBuf>,
    pub model_dir: Option<PathBuf>,
    pub review: mpdf_core::orchestrator::ReviewPolicy,
    pub overwrite: bool,
    pub ocr_dpi: u16,
    pub job_id: String,
    pub progress: Box<dyn ProgressReporter>,
    pub stage: Box<dyn Fn(&str) + Send>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoBookmarkOutcome {
    pub mode: &'static str,
    pub status: &'static str,
    pub toc_page_count: u32,
    pub parsed_entries: u32,
    pub auto_confirmed: u32,
    pub needs_review: u32,
    pub skipped: u32,
    pub written_bookmarks: u32,
    pub safe_refusal_reason: Option<String>,
    pub report_path: PathBuf,
    pub output_path: Option<PathBuf>,
}

/// A handle the rest of the backend uses to talk to the worker thread.
/// Cheap to clone; every clone shares the same underlying channel and
/// therefore the same serialization guarantee.
#[derive(Clone)]
pub struct WorkerHandle {
    sender: std::sync::mpsc::Sender<WorkerCommand>,
}

impl WorkerHandle {
    /// Spawns the worker thread. Call once per application run — this
    /// crate stores the single resulting handle in managed Tauri state.
    pub fn spawn(bundled_pdfium_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<WorkerCommand>();
        thread::Builder::new()
            .name("mpdf-pdfium-worker".to_string())
            .spawn(move || run(receiver, bundled_pdfium_path))
            .expect("failed to spawn the PDFium worker thread");
        Self { sender }
    }

    /// Sends a command to the worker thread. The worker thread only stops
    /// running when the process is shutting down, at which point a send
    /// failure is not actionable by the caller.
    pub fn send(&self, command: WorkerCommand) {
        let _ = self.sender.send(command);
    }

    /// Sends a command (built by `build`, which receives the reply
    /// channel to attach) and awaits its single reply, bridging the
    /// worker thread's blocking `recv()` through
    /// `tauri::async_runtime::spawn_blocking` so the calling async Tauri
    /// command never blocks the event loop while it waits.
    pub async fn call<T, F>(&self, build: F) -> CoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(Reply<T>) -> WorkerCommand,
    {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<CoreResult<T>>();
        self.send(build(reply_tx));
        let join = tauri::async_runtime::spawn_blocking(move || {
            reply_rx.recv().unwrap_or_else(|_| Err(worker_gone()))
        });
        join.await.unwrap_or_else(|_| Err(worker_gone()))
    }
}

fn worker_gone() -> CoreError {
    CoreError::InvalidParameter("the PDFium worker thread is not responding".to_string())
}

fn run(receiver: std::sync::mpsc::Receiver<WorkerCommand>, bundled_pdfium_path: Option<PathBuf>) {
    let mut session: Option<PdfDocumentSession> = None;
    for command in receiver {
        match command {
            WorkerCommand::Open {
                path,
                password,
                reply,
            } => {
                let outcome = open(&path, password, bundled_pdfium_path.as_deref());
                match outcome {
                    Ok((new_session, opened)) => {
                        session = Some(new_session);
                        let _ = reply.send(Ok(opened));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            WorkerCommand::Close => {
                session = None;
            }
            WorkerCommand::RenderPage {
                page_index,
                dpi,
                processed,
                reply,
            } => {
                let result = render_page(session.as_ref(), page_index, dpi, processed);
                let _ = reply.send(result);
            }
            WorkerCommand::PreviewPage {page_index,dpi,processed,expected_source,cancelled,reply} => {
                let result = (|| {
                    if cancelled.load(Ordering::SeqCst) { return Err(CoreError::Cancelled); }
                    let current=session.as_ref().ok_or_else(no_open_document)?;
                    if current.source_identity().canonical_path!=expected_source { return Err(CoreError::InvalidParameter("document_stale".into())); }
                    let raster=current.render_page(page_index,dpi)?;
                    if cancelled.load(Ordering::SeqCst) { return Err(CoreError::Cancelled); }
                    let image=if let Some(settings)=processed {
                        image::DynamicImage::ImageLuma8(pipeline::bilevel_to_gray(&process_rendered_page(&raster,&settings)?.bilevel))
                    } else {raster};
                    Ok(RenderedPage{image})
                })();
                let _=reply.send(result);
            }
            WorkerCommand::ProcessSelected {output,settings,selection,progress,reply} => {
                let result=(|| {
                    let session=session.as_ref().ok_or_else(no_open_document)?;
                    let options=PdfProcessingOptions{pdfium:pdfium_config(bundled_pdfium_path.as_deref()),..Default::default()};
                    mpdf_core::selective_pdf::process_selected_with_open_session(session,&selection,&output,&settings,&options,progress.as_ref())
                })();
                let _=reply.send(result);
            }
            WorkerCommand::Process {
                output,
                settings,
                overwrite,
                prior_estimate,
                progress,
                reply,
            } => {
                let result = process(
                    session.as_ref(),
                    &output,
                    &settings,
                    overwrite,
                    prior_estimate,
                    progress.as_ref(),
                    bundled_pdfium_path.as_deref(),
                );
                let _ = reply.send(result);
            }
            WorkerCommand::AutoBookmark { request, reply } => {
                let result = auto_bookmark(&request, bundled_pdfium_path.as_deref());
                let _ = reply.send(result);
            }
            WorkerCommand::FinalPdf { request, reply } => {
                let result = final_pdf(&request, bundled_pdfium_path.as_deref());
                let _ = reply.send(result);
            }
            WorkerCommand::Estimate {
                settings,
                samples,
                progress,
                reply,
            } => {
                let result = estimate(
                    session.as_ref(),
                    &settings,
                    samples,
                    progress.as_ref(),
                    bundled_pdfium_path.as_deref(),
                );
                let _ = reply.send(result);
            }
        }
    }
}

/// Runs the one core orchestrator. Nothing about the stage order, the
/// source binding, or the output discipline is decided here.
fn final_pdf(
    work: &FinalPdfWork,
    bundled_pdfium_path: Option<&std::path::Path>,
) -> CoreResult<mpdf_core::orchestrator::FinalPdfOutcome> {
    use mpdf_core::orchestrator::FinalPdfRequest;

    let provider = final_pdf_provider(
        work.engine,
        work.engine_binary.as_deref(),
        &work.language_profile,
        work.sidecar.as_deref(),
        work.model_dir.as_deref(),
    )?;
    // The backend decides for itself whether a cloud mode is permitted; the
    // frontend's word is never enough, because a disabled button is a UI
    // affordance and this is the enforcement point.
    let cloud = final_pdf_cloud(work)?;
    let request = FinalPdfRequest {
        source: &work.source,
        output: &work.output,
        workspace: &work.workspace,
        settings: &work.settings,
        provider,
        language_profile: work.language_profile.clone(),
        review: work.review,
        overwrite: work.overwrite,
        password: None,
        pdfium: pdfium_config(bundled_pdfium_path),
        output_write_strategy: pipeline::OutputWriteStrategy::AtomicSameDirectoryRename,
        bookmark_config: Default::default(),
        ocr_dpi: work.ocr_dpi,
        job_id: work.job_id.clone(),
        jobs_db: work.workspace.join("jobs.sqlite3"),
        owner: "mpdf-desktop".into(),
        cloud: cloud.as_ref().map(|factory| factory.as_ref()),
    };
    mpdf_core::orchestrator::run(&request, work.progress.as_ref(), &|stage| {
        (work.stage)(stage.as_str())
    })
}

/// Builds the cloud factory for a desktop run, or `None` for a local one.
///
/// This is the backend's own check, deliberately duplicating what the
/// provider picker already enforces. A Tauri command is a public API of the
/// process: anything reachable from the webview has to be re-validated here,
/// or a compromised renderer could start an upload the user never authorized.
fn final_pdf_cloud(
    work: &FinalPdfWork,
) -> CoreResult<Option<Box<dyn mpdf_core::orchestrator::CloudProviderFactory>>> {
    use mpdf_core::ocr_provider::{credits, OcrProviderMode, BYOK_DISABLED_MESSAGE};

    if work.provider_mode == OcrProviderMode::Local {
        return Ok(None);
    }
    if work.provider_mode == OcrProviderMode::GeminiByok {
        return Err(CoreError::InvalidParameter(
            BYOK_DISABLED_MESSAGE.to_owned(),
        ));
    }
    if !work.cloud_consent {
        return Err(CoreError::InvalidParameter(
            "cloud OCR uploads a rendered image of every OCR'd page; it cannot start without \
             explicit consent"
                .to_owned(),
        ));
    }
    match work.provider_mode {
        OcrProviderMode::Local => unreachable!("handled above"),
        OcrProviderMode::GeminiByok => unreachable!("disabled above"),
        OcrProviderMode::BrokerTest => {
            if !cfg!(debug_assertions) || work.max_credits != 0 {
                return Err(CoreError::InvalidParameter(
                    "broker-test is debug-only and does not use Credits".into(),
                ));
            }
            let path = work.broker_test_config.as_deref().ok_or_else(|| {
                CoreError::InvalidParameter(
                    "broker-test requires native MPDF_BROKER_TEST_CONFIG".into(),
                )
            })?;
            let factory = mpdf_api_client::spatial_factory::SpatialFactory::from_config_file(path)
                .map_err(|e| CoreError::InvalidParameter(e.to_string()))?;
            Ok(Some(Box::new(factory)))
        }
        OcrProviderMode::MpdfCredits => {
            if work.max_credits == 0 {
                return Err(CoreError::InvalidParameter(
                    "a hard cost limit must be authorized before a paid brokered OCR run can start"
                        .to_owned(),
                ));
            }
            Err(CoreError::InvalidParameter(format!(
                "M PDF Cloud OCR is unavailable: no production backend currently returns the complete coordinate OCR contract; {}",
                credits::release_blockers().join("; ")
            )))
        }
    }
}

/// Builds the real OCR provider for a user-visible desktop conversion.
///
/// A base installation without the optional plugin uses `NativeTextOnly`:
/// reliable native-text pages still work, while a scanned page fails with the
/// installation diagnostic rather than producing a successful-looking but
/// unsearchable PDF.
fn final_pdf_provider(
    engine: mpdf_core::ocr::OcrEngine,
    engine_binary: Option<&std::path::Path>,
    language_profile: &str,
    sidecar: Option<&std::path::Path>,
    model_dir: Option<&std::path::Path>,
) -> CoreResult<mpdf_core::orchestrator::OcrProviderChoice> {
    use mpdf_core::ocr::SidecarOcrConfig;
    use mpdf_core::orchestrator::OcrProviderChoice;

    let (executable, model_dir) = match (sidecar, model_dir) {
        (Some(executable), Some(model_dir)) => (executable, model_dir),
        (None, None) => return Ok(OcrProviderChoice::NativeTextOnly {
            diagnostic: "this scanned or image-only page requires the optional local OCR plugin; install and verify the offline OCR runtime, or configure both the OCR sidecar and model directory. PDFs with a reliable native text layer do not require the plugin".to_owned(),
        }),
        _ => {
            return Err(CoreError::InvalidParameter(
                "an explicit optional local OCR plugin configuration requires both the OCR sidecar and model directory"
                    .to_owned(),
            ))
        }
    };
    let mut config = SidecarOcrConfig::tesseract(
        executable.to_path_buf(),
        model_dir.to_path_buf(),
        language_profile,
    )
    .map_err(|error| CoreError::InvalidParameter(error.to_string()))?;
    config.engine = engine;
    config.engine_binary = engine_binary.map(std::path::Path::to_path_buf);
    if !engine.cleared_for_production() {
        config.required_files.clear();
    }
    Ok(OcrProviderChoice::Sidecar(config))
}

/// Builds the [`PdfiumConfig`] this desktop backend actually uses,
/// applying one precedence rule on top of the core resolver's own
/// (`resolve_library`'s explicit-path-then-env-var-then-search order):
/// an explicit `MPDF_PDFIUM_LIBRARY` still wins when set (unchanged
/// developer/support override behavior — see `docs/pdfium-bundling.md`),
/// but otherwise a packaged build's trusted bundled resource (resolved
/// once at startup via Tauri's own resource-directory API — see
/// `lib.rs`'s `.setup()`) is used explicitly rather than relying on the
/// core resolver's generic executable-adjacent search, which does not
/// know macOS's `Contents/Resources` bundle layout. A development run
/// with no bundled resource falls back to `PdfiumConfig::default()`
/// unchanged, so `MPDF_ALLOW_CWD_PDFIUM`/executable-adjacent search
/// keep working exactly as before.
pub(crate) fn pdfium_config(bundled_pdfium_path: Option<&std::path::Path>) -> PdfiumConfig {
    if std::env::var_os(mpdf_core::pdfium_backend::PDFIUM_LIBRARY_ENV).is_some() {
        return PdfiumConfig::default();
    }
    match bundled_pdfium_path {
        Some(path) => PdfiumConfig {
            library_path: Some(path.to_path_buf()),
            allow_system_library: false,
        },
        None => PdfiumConfig::default(),
    }
}

/// A compile-time choice, not a runtime one: whether *this build* is the
/// Mac App Store variant is fixed at build time by
/// `scripts/distribution/package_mas.py` (`tauri build --features
/// mas-sandbox`), never toggled at runtime. See
/// `docs/mac-app-store-readiness.md`, "Sandboxed output-save
/// architecture," for why the GitHub build (never sandboxed) keeps the
/// atomic same-directory rename unconditionally.
fn output_write_strategy() -> pipeline::OutputWriteStrategy {
    #[cfg(feature = "mas-sandbox")]
    {
        pipeline::OutputWriteStrategy::DirectWriteToDestination
    }
    #[cfg(not(feature = "mas-sandbox"))]
    {
        pipeline::OutputWriteStrategy::default()
    }
}

fn open(
    path: &std::path::Path,
    password: Option<String>,
    bundled_pdfium_path: Option<&std::path::Path>,
) -> CoreResult<(PdfDocumentSession, OpenedDocument)> {
    let options = PdfOpenOptions {
        password,
        pdfium: pdfium_config(bundled_pdfium_path),
        compute_source_hash: true,
    };
    let session = PdfDocumentSession::open(path, &options)?;
    let info = session.info().clone();
    let pdfium_library = mpdf_core::pdfium_backend::describe_resolved(session.resolved_library());
    let source_sha256=session.source_identity().content_sha256.clone().ok_or_else(||CoreError::InvalidDocument("missing open source hash".into()))?;
    Ok((
        session,
        OpenedDocument {
            source_sha256,
            info,
            pdfium_library,
        },
    ))
}

fn render_page(
    session: Option<&PdfDocumentSession>,
    page_index: u32,
    dpi: u16,
    processed: Option<ProcessingSettings>,
) -> CoreResult<RenderedPage> {
    let session = session.ok_or_else(no_open_document)?;
    let rendered = session.render_page(page_index, dpi)?;
    let image = match processed {
        Some(settings) => {
            let result = process_rendered_page(&rendered, &settings)?;
            image::DynamicImage::ImageLuma8(pipeline::bilevel_to_gray(&result.bilevel))
        }
        None => rendered,
    };
    Ok(RenderedPage { image })
}

fn process(
    session: Option<&PdfDocumentSession>,
    output: &std::path::Path,
    settings: &ProcessingSettings,
    overwrite: bool,
    prior_estimate: Option<pipeline::PriorEstimate>,
    progress: &dyn ProgressReporter,
    bundled_pdfium_path: Option<&std::path::Path>,
) -> CoreResult<ProcessingReport> {
    let session = session.ok_or_else(no_open_document)?;
    let options = PdfProcessingOptions {
        password: None,
        overwrite,
        validation: mpdf_core::validation::ValidationMode::default(),
        pdfium: pdfium_config(bundled_pdfium_path),
        prior_estimate,
        output_write_strategy: output_write_strategy(),
    };
    pipeline::process_with_open_session(session, output, settings, &options, progress)
}

fn estimate(
    session: Option<&PdfDocumentSession>,
    settings: &ProcessingSettings,
    samples: u32,
    progress: &dyn ProgressReporter,
    bundled_pdfium_path: Option<&std::path::Path>,
) -> CoreResult<SizeEstimateReport> {
    let session = session.ok_or_else(no_open_document)?;
    let options = EstimationOptions {
        password: None,
        pdfium: pdfium_config(bundled_pdfium_path),
        samples,
    };
    pipeline::estimate_with_open_session(session, settings, &options, progress)
}

/// One automatic bookmark run: compile, persist, and — only when something
/// reached the gate — write and verify the outlined PDF. Every core rule
/// (source binding, no-clobber, atomic install, reopen verification) lives
/// in `mpdf_core`; this function adds cancellation and stage reporting only.
pub(crate) fn auto_bookmark(
    work: &AutoBookmarkWork,
    bundled_pdfium_path: Option<&std::path::Path>,
) -> CoreResult<AutoBookmarkOutcome> {
    use mpdf_core::bookmarks::{self, AutoBookmarkConfig};
    use mpdf_core::searchable_output::{build_searchable_output_observed, SearchableOutputRequest};
    use std::sync::atomic::Ordering;

    let cancelled = work.cancelled.clone();
    let is_cancelled = move || cancelled.load(Ordering::SeqCst);
    (work.stage)("analyzing_toc");
    let existing = bookmarks::candidates_path(&work.package_root).exists();
    if existing && !work.regenerate {
        return Err(CoreError::DestinationConflict(
            "bookmark candidates already exist for this package; regeneration must be \
             authorized explicitly"
                .to_string(),
        ));
    }
    if existing {
        let snapshot = bookmarks::load_snapshot(&work.package_root)?;
        let reviews = bookmarks::load_reviews(&work.package_root, &snapshot)?;
        if !reviews.operations.is_empty() {
            return Err(CoreError::DestinationConflict(format!(
                "{} human review decision(s) exist for the current bookmarks; export or \
                 remove them before regenerating",
                reviews.operations.len()
            )));
        }
    }
    let inputs = bookmarks::load_auto_bookmark_inputs(&work.package_root)?;
    (work.stage)("aligning");
    let result = bookmarks::generate_auto_with_cancel(
        &inputs.as_input(),
        &AutoBookmarkConfig::default(),
        &is_cancelled,
    )?;
    let generation = bookmarks::begin_generation(&work.package_root, &result, existing)?;
    let reviews = match bookmarks::load_reviews(&work.package_root, &result.snapshot) {
        Ok(reviews) => reviews,
        Err(error) => return rollback_generation(generation, error),
    };
    let effective = match bookmarks::effective(&result.snapshot, &reviews) {
        Ok(effective) => effective,
        Err(error) => return rollback_generation(generation, error),
    };
    let writable = effective
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .count() as u32;
    let output_path = if writable == 0 {
        None
    } else {
        let summary = match build_searchable_output_observed(
            &SearchableOutputRequest {
                package: &inputs.package,
                source: &work.source,
                output: &work.output,
                overwrite: work.overwrite,
                candidates: &effective,
                derived: inputs.derived.as_ref(),
                pdfium: pdfium_config(bundled_pdfium_path),
                output_write_strategy: output_write_strategy(),
            },
            &is_cancelled,
            &|stage| (work.stage)(stage.as_str()),
        ) {
            Ok(summary) => summary,
            Err(error) => {
                return rollback_generation(generation, error);
            }
        };
        Some(summary.output_path)
    };
    generation.commit()?;
    Ok(AutoBookmarkOutcome {
        mode: result.report.mode.as_str(),
        status: result.report.status.as_str(),
        toc_page_count: result.report.toc_pages.len() as u32,
        parsed_entries: result.report.parsed_entries,
        auto_confirmed: result.report.auto_confirmed,
        needs_review: result.report.needs_review,
        skipped: result.report.skipped,
        written_bookmarks: if output_path.is_some() { writable } else { 0 },
        safe_refusal_reason: result.report.safe_refusal_reason.clone(),
        report_path: bookmarks::generation_report_path(&work.package_root),
        output_path,
    })
}

fn no_open_document() -> CoreError {
    CoreError::InvalidParameter("no document is open".to_string())
}

fn rollback_generation<T>(
    generation: mpdf_core::bookmarks::GenerationTransaction,
    error: CoreError,
) -> CoreResult<T> {
    match generation.rollback() {
        Ok(()) => Err(error),
        Err(rollback_error) => Err(CoreError::InvalidDocument(format!(
            "automatic bookmark output failed: {error}; rollback failed: {rollback_error}"
        ))),
    }
}

#[cfg(test)]
mod output_write_strategy_tests {
    use super::output_write_strategy;
    use mpdf_core::pipeline::OutputWriteStrategy;

    // `output_write_strategy` is a compile-time `cfg` branch, not a
    // runtime toggle, so one `#[test]` cannot exercise both arms in the
    // same binary — each half below only compiles for the matching
    // feature state, and `cargo test -p mpdf-desktop` /
    // `cargo test -p mpdf-desktop --features mas-sandbox`
    // each give the other its real, non-`#[cfg]`-skipped coverage.

    #[cfg(not(feature = "mas-sandbox"))]
    #[test]
    fn ordinary_build_keeps_the_atomic_same_directory_rename_default() {
        // Ordinary `cargo test`/the GitHub distribution build compiles
        // without `mas-sandbox`, so this asserts exactly the M0–M7A
        // behavior stays the default.
        assert_eq!(
            output_write_strategy(),
            OutputWriteStrategy::AtomicSameDirectoryRename
        );
    }

    #[cfg(feature = "mas-sandbox")]
    #[test]
    fn mas_sandbox_build_selects_direct_write_to_destination() {
        assert_eq!(
            output_write_strategy(),
            OutputWriteStrategy::DirectWriteToDestination
        );
    }
}

#[cfg(test)]
mod final_pdf_provider_tests {
    use super::final_pdf_provider;
    use mpdf_core::error::CoreError;
    use mpdf_core::ocr::OcrEngine;

    #[test]
    fn a_base_run_without_the_plugin_uses_native_text_only() {
        let provider = final_pdf_provider(OcrEngine::Tesseract, None, "auto", None, None).unwrap();
        let mpdf_core::orchestrator::OcrProviderChoice::NativeTextOnly { diagnostic } = provider
        else {
            panic!("the base package must not substitute an empty OCR provider");
        };
        assert!(diagnostic.contains("optional local OCR plugin"));
        assert!(diagnostic.contains("reliable native text"));
    }

    #[test]
    fn a_partial_explicit_plugin_configuration_fails_closed() {
        for (sidecar, models) in [
            (Some(std::path::Path::new("sidecar")), None),
            (None, Some(std::path::Path::new("models"))),
        ] {
            let error = final_pdf_provider(OcrEngine::Tesseract, None, "auto", sidecar, models)
                .expect_err("an incomplete production provider must fail closed");
            let CoreError::InvalidParameter(message) = error else {
                panic!("expected an invalid-parameter error");
            };
            assert!(message.contains("sidecar") && message.contains("model directory"));
        }
    }

    #[test]
    fn a_complete_configuration_builds_the_real_sidecar_provider() {
        let provider = final_pdf_provider(
            OcrEngine::Tesseract,
            None,
            "auto",
            Some(std::path::Path::new("sidecar")),
            Some(std::path::Path::new("models")),
        )
        .unwrap();
        assert!(matches!(
            provider,
            mpdf_core::orchestrator::OcrProviderChoice::Sidecar(_)
        ));
    }
}

#[cfg(test)]
mod pdfium_config_tests {
    use super::pdfium_config;
    use mpdf_core::pdfium_backend::PDFIUM_LIBRARY_ENV;
    use std::path::PathBuf;

    /// Serializes tests that touch the shared process-global
    /// `MPDF_PDFIUM_LIBRARY` environment variable, the same
    /// discipline `pdfium_backend`'s own tests use.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn uses_the_bundled_path_explicitly_when_no_env_override_is_set() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(PDFIUM_LIBRARY_ENV);

        let bundled = PathBuf::from("/app/Resources/libpdfium.dylib");
        let config = pdfium_config(Some(&bundled));
        assert_eq!(config.library_path, Some(bundled));
        assert!(!config.allow_system_library);
    }

    #[test]
    fn falls_back_to_the_default_resolver_when_no_bundled_path_exists() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(PDFIUM_LIBRARY_ENV);

        let config = pdfium_config(None);
        assert_eq!(config.library_path, None);
    }

    #[test]
    fn an_explicit_env_override_still_wins_over_a_bundled_path() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(PDFIUM_LIBRARY_ENV, "/dev/override/libpdfium.dylib");

        let bundled = PathBuf::from("/app/Resources/libpdfium.dylib");
        let config = pdfium_config(Some(&bundled));
        std::env::remove_var(PDFIUM_LIBRARY_ENV);

        // pdfium_config defers to PdfiumConfig::default() (no explicit
        // library_path of its own) so the core resolver's own env-var
        // handling takes over — it must not shadow that with the
        // bundled path.
        assert_eq!(config.library_path, None);
    }
}

#[cfg(test)]
mod auto_bookmark_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use mpdf_core::bookmark_fixtures::{self as fixtures, FixtureLine, FixturePage};

    fn work(root: PathBuf, stages: Arc<std::sync::Mutex<Vec<String>>>) -> AutoBookmarkWork {
        AutoBookmarkWork {
            source: root.join("never-read.pdf"),
            output: root.join("out.pdf"),
            package_root: root,
            overwrite: false,
            regenerate: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            stage: Box::new(move |stage| stages.lock().unwrap().push(stage.to_owned())),
        }
    }

    /// A document with no printed contents produces a normal, explained
    /// result — not an error — and never touches the output path. This needs
    /// no PDFium because nothing reaches the writer.
    #[test]
    fn a_safe_refusal_is_a_result_not_a_failure_and_writes_no_pdf() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("book.mdp");
        let mut pages = vec![FixturePage::new(vec![FixtureLine::new(
            "A Book Without Contents",
            100.0,
            100.0,
        )])];
        for index in 0..4 {
            pages.push(FixturePage::new(vec![FixtureLine::new(
                &format!("page {index} body"),
                60.0,
                300.0,
            )]));
        }
        let package = fixtures::package("desktop-refusal", pages.len() as u32);
        package.write_to(&root).unwrap();
        mpdf_core::ocr::write_ocr_records(&root, &fixtures::ocr_run(&pages, None)).unwrap();

        let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let outcome = auto_bookmark(&work(root.clone(), stages.clone()), None)
            .expect("a refusal is a successful command result");
        assert_eq!(outcome.mode, "safe_refusal");
        assert_eq!(outcome.status, "safe_refusal");
        assert_eq!(outcome.written_bookmarks, 0);
        assert!(outcome.output_path.is_none());
        assert!(outcome.safe_refusal_reason.is_some());
        assert!(!root.join("out.pdf").exists());
        assert!(std::path::Path::new(&outcome.report_path).is_file());
        assert_eq!(
            *stages.lock().unwrap(),
            vec!["analyzing_toc".to_owned(), "aligning".to_owned()],
            "the write and validate stages are not reported for a refusal"
        );
    }

    #[test]
    fn regeneration_over_existing_candidates_must_be_authorized() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("book.mdp");
        let (package, pages) = fixtures::aligned_book();
        package.write_to(&root).unwrap();
        mpdf_core::ocr::write_ocr_records(&root, &fixtures::ocr_run(&pages, None)).unwrap();
        let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
        // The first run stops at the writer, which needs a real source PDF.
        let first = auto_bookmark(&work(root.clone(), stages.clone()), None);
        assert!(
            first.is_err(),
            "the fake source path cannot be written from"
        );
        assert!(
            !mpdf_core::bookmarks::candidates_path(&root).exists(),
            "a failed output write must roll back the new generation"
        );
        let seed = mpdf_core::bookmarks::generate_auto_from_package(
            &root,
            &mpdf_core::bookmarks::AutoBookmarkConfig::default(),
            &|| false,
        )
        .unwrap();
        mpdf_core::bookmarks::save_generation(&root, &seed, false).unwrap();

        let refused = auto_bookmark(&work(root.clone(), stages.clone()), None)
            .expect_err("existing candidates are protected");
        assert!(matches!(refused, CoreError::DestinationConflict(_)));
        let mut authorized = work(root.clone(), stages);
        authorized.regenerate = true;
        // Regeneration is allowed once authorized; it still fails later at
        // the writer for the same missing-source reason, not at the guard.
        let outcome = auto_bookmark(&authorized, None).unwrap_err();
        assert!(!matches!(outcome, CoreError::DestinationConflict(_)));
    }

    #[test]
    fn cancellation_produces_a_cancelled_error_and_no_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("book.mdp");
        let (package, pages) = fixtures::aligned_book();
        package.write_to(&root).unwrap();
        mpdf_core::ocr::write_ocr_records(&root, &fixtures::ocr_run(&pages, None)).unwrap();
        let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let request = work(root.clone(), stages);
        request
            .cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            auto_bookmark(&request, None),
            Err(CoreError::Cancelled)
        ));
        assert!(!mpdf_core::bookmarks::candidates_path(&root).exists());
        assert!(!root.join("out.pdf").exists());
    }
}
