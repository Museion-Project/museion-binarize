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
use museion_binarize_core::image_pipeline::process_rendered_page;
use museion_binarize_core::pdfium_backend::PdfiumConfig;
use museion_binarize_core::pipeline::{self, PdfProcessingOptions, ProcessingReport};
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
        progress: Box<dyn ProgressReporter>,
        reply: Reply<ProcessingReport>,
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
    pub fn spawn() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<WorkerCommand>();
        thread::Builder::new()
            .name("museion-pdfium-worker".to_string())
            .spawn(move || run(receiver))
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

fn run(receiver: std::sync::mpsc::Receiver<WorkerCommand>) {
    let mut session: Option<PdfDocumentSession> = None;
    for command in receiver {
        match command {
            WorkerCommand::Open {
                path,
                password,
                reply,
            } => {
                let outcome = open(&path, password);
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
                progress,
                reply,
            } => {
                let result = process(
                    session.as_ref(),
                    &output,
                    &settings,
                    overwrite,
                    progress.as_ref(),
                );
                let _ = reply.send(result);
            }
        }
    }
}

fn pdfium_config() -> PdfiumConfig {
    // Development-only resolution: an explicit MUSEION_PDFIUM_LIBRARY, or
    // an application-relative/executable-relative bundled copy (handled
    // by `pdfium_backend::resolve_library`'s existing search order). No
    // automatic download — see docs/adr/0001-pdfium-runtime-binding.md.
    PdfiumConfig::default()
}

fn open(
    path: &std::path::Path,
    password: Option<String>,
) -> CoreResult<(PdfDocumentSession, OpenedDocument)> {
    let options = PdfOpenOptions {
        password,
        pdfium: pdfium_config(),
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
    progress: &dyn ProgressReporter,
) -> CoreResult<ProcessingReport> {
    let session = session.ok_or_else(no_open_document)?;
    let options = PdfProcessingOptions {
        password: None,
        overwrite,
        validation: museion_binarize_core::validation::ValidationMode::default(),
        pdfium: pdfium_config(),
    };
    pipeline::process_with_open_session(session, output, settings, &options, progress)
}

fn no_open_document() -> CoreError {
    CoreError::InvalidParameter("no document is open".to_string())
}
