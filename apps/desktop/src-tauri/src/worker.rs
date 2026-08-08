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

use museion_binarize_core::document::PdfDocumentInfo;
use museion_binarize_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use museion_binarize_core::error::{CoreError, Result as CoreResult};
use museion_binarize_core::estimation::SizeEstimateReport;
use museion_binarize_core::image_pipeline::process_rendered_page;
use museion_binarize_core::pdfium_backend::PdfiumConfig;
use museion_binarize_core::pipeline::{
    self, EstimationOptions, PdfProcessingOptions, ProcessingReport,
};
use museion_binarize_core::progress::ProgressReporter;
use museion_binarize_core::settings::ProcessingSettings;

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
    Process {
        output: PathBuf,
        settings: ProcessingSettings,
        overwrite: bool,
        prior_estimate: Option<pipeline::PriorEstimate>,
        progress: Box<dyn ProgressReporter>,
        reply: Reply<ProcessingReport>,
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
            .name("museion-pdfium-worker".to_string())
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

/// Builds the [`PdfiumConfig`] this desktop backend actually uses,
/// applying one precedence rule on top of the core resolver's own
/// (`resolve_library`'s explicit-path-then-env-var-then-search order):
/// an explicit `MUSEION_PDFIUM_LIBRARY` still wins when set (unchanged
/// developer/support override behavior — see `docs/pdfium-bundling.md`),
/// but otherwise a packaged build's trusted bundled resource (resolved
/// once at startup via Tauri's own resource-directory API — see
/// `lib.rs`'s `.setup()`) is used explicitly rather than relying on the
/// core resolver's generic executable-adjacent search, which does not
/// know macOS's `Contents/Resources` bundle layout. A development run
/// with no bundled resource falls back to `PdfiumConfig::default()`
/// unchanged, so `MUSEION_ALLOW_CWD_PDFIUM`/executable-adjacent search
/// keep working exactly as before.
pub(crate) fn pdfium_config(bundled_pdfium_path: Option<&std::path::Path>) -> PdfiumConfig {
    if std::env::var_os(museion_binarize_core::pdfium_backend::PDFIUM_LIBRARY_ENV).is_some() {
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
        compute_source_hash: false,
    };
    let session = PdfDocumentSession::open(path, &options)?;
    let info = session.info().clone();
    let pdfium_library =
        museion_binarize_core::pdfium_backend::describe_resolved(session.resolved_library());
    Ok((
        session,
        OpenedDocument {
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
        validation: museion_binarize_core::validation::ValidationMode::default(),
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

fn no_open_document() -> CoreError {
    CoreError::InvalidParameter("no document is open".to_string())
}

#[cfg(test)]
mod output_write_strategy_tests {
    use super::output_write_strategy;
    use museion_binarize_core::pipeline::OutputWriteStrategy;

    // `output_write_strategy` is a compile-time `cfg` branch, not a
    // runtime toggle, so one `#[test]` cannot exercise both arms in the
    // same binary — each half below only compiles for the matching
    // feature state, and `cargo test -p museion-binarize-desktop` /
    // `cargo test -p museion-binarize-desktop --features mas-sandbox`
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
mod pdfium_config_tests {
    use super::pdfium_config;
    use museion_binarize_core::pdfium_backend::PDFIUM_LIBRARY_ENV;
    use std::path::PathBuf;

    /// Serializes tests that touch the shared process-global
    /// `MUSEION_PDFIUM_LIBRARY` environment variable, the same
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
