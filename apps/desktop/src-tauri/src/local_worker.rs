//! Serialized PDFium worker for the macOS local tools.
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

use mpdf_core::document::PdfDocumentInfo;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::error::{CoreError, Result as CoreResult};
use mpdf_core::image_pipeline::process_rendered_page;
use mpdf_core::page_selection::PageSelection;
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::pipeline::{self, PdfProcessingOptions, ProcessingReport};
use mpdf_core::progress::ProgressReporter;
use mpdf_core::settings::ProcessingSettings;

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
}

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
            WorkerCommand::PreviewPage {
                page_index,
                dpi,
                processed,
                expected_source,
                cancelled,
                reply,
            } => {
                let result = (|| {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err(CoreError::Cancelled);
                    }
                    let current = session.as_ref().ok_or_else(no_open_document)?;
                    if current.source_identity().canonical_path != expected_source {
                        return Err(CoreError::InvalidParameter("document_stale".into()));
                    }
                    let raster = current.render_page(page_index, dpi)?;
                    if cancelled.load(Ordering::SeqCst) {
                        return Err(CoreError::Cancelled);
                    }
                    let image = if let Some(settings) = processed {
                        image::DynamicImage::ImageLuma8(pipeline::bilevel_to_gray(
                            &process_rendered_page(&raster, &settings)?.bilevel,
                        ))
                    } else {
                        raster
                    };
                    Ok(RenderedPage { image })
                })();
                let _ = reply.send(result);
            }
            WorkerCommand::ProcessSelected {
                output,
                settings,
                selection,
                progress,
                reply,
            } => {
                let result = (|| {
                    let session = session.as_ref().ok_or_else(no_open_document)?;
                    let options = PdfProcessingOptions {
                        pdfium: pdfium_config(bundled_pdfium_path.as_deref()),
                        ..Default::default()
                    };
                    mpdf_core::selective_pdf::process_selected_with_open_session(
                        session,
                        &selection,
                        &output,
                        &settings,
                        &options,
                        progress.as_ref(),
                    )
                })();
                let _ = reply.send(result);
            }
        }
    }
}

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
    let source_sha256 = session
        .source_identity()
        .content_sha256
        .clone()
        .ok_or_else(|| CoreError::InvalidDocument("missing open source hash".into()))?;
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

fn no_open_document() -> CoreError {
    CoreError::InvalidParameter("no document is open".to_string())
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
