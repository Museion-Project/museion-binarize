//! The one high-level pipeline: original PDF in, final binarized + searchable
//! + outlined PDF out.
//!
//! # The data dependency this module exists to enforce
//!
//! ```text
//!   authoritative colour/grayscale source PDF
//!        -> render ORIGINAL pages
//!        -> OCR those original renders            (ocr_original)
//!        -> typed evidence + derived text layer   (deriving_text)
//!        -> automatic bookmarks                   (generating_bookmarks)
//!        -> optional human review                 (awaiting_review)
//!        -> freeze text layer + bookmark tree
//!        -> binarize the VISIBLE pages only       (binarizing_visuals)
//!        -> assemble one final PDF carrying the
//!           binarized pixels, the ORIGINAL OCR
//!           coordinates, and the confirmed tree   (assembling_final_pdf)
//!        -> reopen and verify independently       (validating)
//! ```
//!
//! Binarized pixels are never fed back into OCR, and the text layer is never
//! rebuilt from them. The binarized bytes exist only as a *visual carrier*:
//! [`crate::searchable_pdf::CarrierKind::NormalizedBilevel`] is constructed
//! here and nowhere else, and only from bytes this module just produced from
//! the same package.
//!
//! Both front ends call [`run`]. Neither the CLI nor the desktop app
//! re-implements any of the ordering, the source-binding checks, or the
//! output discipline.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::bookmarks::{self, AutoBookmarkConfig, BookmarkCandidate, BookmarkStatus};
use crate::derived::DerivedDocument;
use crate::document_package::DocumentPackage;
use crate::document_session::{PdfDocumentSession, PdfOpenOptions};
use crate::error::{CoreError, Result};
use crate::ocr::{self, OcrRun, PageOcrProvider};
use crate::ocr_provider::runner::ProviderPageAdapter;
use crate::ocr_provider::{
    CloudFallback, JobPreparation, OcrProvider, OcrProviderMode, ProviderUsage,
};
use crate::pdfium_backend::PdfiumConfig;
use crate::pipeline::{OutputWriteStrategy, PdfProcessingOptions, ProcessingReport};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::searchable_pdf::CarrierKind;
use crate::settings::ProcessingSettings;

/// Coarse stages, in the order they always occur.
///
/// A front end shows these; it does not decide them. `AwaitingReview` is the
/// only stage that can be skipped, and only when nothing needs review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipelineStage {
    AnalyzingSource,
    OcrOriginal,
    DerivingText,
    GeneratingBookmarks,
    AwaitingReview,
    BinarizingVisuals,
    AssemblingFinalPdf,
    Validating,
}

impl PipelineStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnalyzingSource => "analyzing_source",
            Self::OcrOriginal => "ocr_original",
            Self::DerivingText => "deriving_text",
            Self::GeneratingBookmarks => "generating_bookmarks",
            Self::AwaitingReview => "awaiting_review",
            Self::BinarizingVisuals => "binarizing_visuals",
            Self::AssemblingFinalPdf => "assembling_final_pdf",
            Self::Validating => "validating",
        }
    }

    pub const ORDER: [PipelineStage; 8] = [
        Self::AnalyzingSource,
        Self::OcrOriginal,
        Self::DerivingText,
        Self::GeneratingBookmarks,
        Self::AwaitingReview,
        Self::BinarizingVisuals,
        Self::AssemblingFinalPdf,
        Self::Validating,
    ];
}

/// What to do when the bookmark engine produces entries that need a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewPolicy {
    /// Stop at [`PipelineStage::AwaitingReview`] and write nothing. The
    /// workspace keeps the candidates so a reviewer can decide, and a later
    /// run with [`ReviewPolicy::UseExistingDecisions`] continues.
    #[default]
    PauseForReview,
    /// Continue with whatever is already confirmed. Entries still needing
    /// review are simply not written to the outline; they are never promoted.
    ContinueWithConfirmed,
    /// Reuse the review decisions already stored in the workspace and then
    /// continue. Used by the "I have reviewed, carry on" action.
    UseExistingDecisions,
}

/// Which local OCR provider to run.
#[derive(Debug, Clone)]
pub enum OcrProviderChoice {
    /// Base-package path when the optional offline OCR plugin is absent.
    /// Native text pages remain fully usable; the first page that actually
    /// needs recognition receives the explicit diagnostic carried here.
    NativeTextOnly { diagnostic: String },
    /// The deterministic offline reference provider. Development and tests
    /// only: it recognizes nothing and can never produce real bookmarks.
    Reference,
    /// The real local sidecar.
    Sidecar(ocr::SidecarOcrConfig),
}

impl OcrProviderChoice {
    fn label(&self) -> &'static str {
        match self {
            Self::NativeTextOnly { .. } => "native-text-only",
            Self::Reference => "reference",
            Self::Sidecar(_) => "sidecar",
        }
    }
}

/// How a front end supplies a cloud provider without dragging a network stack
/// into this crate.
///
/// The factory is handed the *already-built local provider*, because the local
/// detector is the cloud path's geometry source rather than a fallback bolted
/// on afterwards. A front end cannot accidentally construct a cloud provider
/// without one.
pub trait CloudProviderFactory {
    /// Which mode this factory builds. Used for the fingerprint and the
    /// report before anything is constructed.
    fn mode(&self) -> OcrProviderMode;
    /// The non-secret configuration digest bound into the checkpoint.
    fn config_digest(&self) -> String;
    /// The full non-secret fingerprint contribution.
    fn fingerprint_contribution(&self) -> String;
    /// Ceiling of credits the user authorized for this run.
    fn max_credits(&self) -> u64;
    fn fallback(&self) -> CloudFallback;
    fn build(&self, local: Box<dyn PageOcrProvider>) -> Result<Box<dyn OcrProvider>>;
}

/// What a cloud run actually did, per run rather than per page.
///
/// `fallback_pages` is the field that keeps a cloud run honest: it lists the
/// pages whose text is local, so "I paid for Gemini" and "these 12 pages are
/// Tesseract" are never confused.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloudRunSummary {
    pub mode: Option<OcrProviderMode>,
    pub fallback_pages: Vec<u32>,
    pub usage: ProviderUsage,
    /// True when at least one request ended without a provider response after
    /// it may have been accepted. Token and cost totals are then lower bounds.
    pub usage_may_be_incomplete: bool,
    pub reserved_credits: u64,
    pub estimated_credits: u64,
    pub charged_credits: u64,
    pub released_credits: u64,
    pub refunded_credits: u64,
    pub settled: bool,
    /// Verbatim from the provider's capabilities. Front ends print this
    /// instead of deciding for themselves whether a mode is usable.
    pub not_production_ready_reason: Option<String>,
}

pub struct FinalPdfRequest<'a> {
    /// The authoritative colour/grayscale PDF. Never modified, never written.
    pub source: &'a Path,
    /// Where the finished binarized + searchable + outlined PDF goes.
    pub output: &'a Path,
    /// Durable workspace for the MDP package, OCR records, derived bundle and
    /// bookmark candidates. Created if absent; reused if it already matches
    /// this exact source.
    pub workspace: &'a Path,
    pub settings: &'a ProcessingSettings,
    pub provider: OcrProviderChoice,
    pub language_profile: String,
    pub review: ReviewPolicy,
    pub overwrite: bool,
    pub password: Option<String>,
    pub pdfium: PdfiumConfig,
    pub output_write_strategy: OutputWriteStrategy,
    pub bookmark_config: AutoBookmarkConfig,
    /// OCR raster resolution. The canonical value keeps evidence comparable
    /// across runs; it is unrelated to the binarization DPI.
    pub ocr_dpi: u16,
    /// Job identity for the durable, resumable OCR store.
    pub job_id: String,
    pub jobs_db: PathBuf,
    pub owner: String,
    /// `None` — the default — is a purely local run: nothing in this module
    /// opens a socket, and `provider` above is the whole OCR story. `Some`
    /// selects a cloud mode explicitly, and the local provider becomes the
    /// geometry source and the per-page fallback.
    pub cloud: Option<&'a dyn CloudProviderFactory>,
}

/// Why a run stopped without producing a PDF. A normal result, not an error:
/// the caller reports it and writes nothing.
///
/// A bookmark safe refusal is deliberately *not* a halt. Refusing to invent an
/// outline says nothing about the OCR evidence or the binarized pixels, so the
/// combined main flow still produces the final PDF and reports the refusal
/// through [`FinalPdfOutcome::bookmark_status`]. The standalone bookmark
/// command keeps its own, stricter contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// Entries need a human decision before anything is written.
    AwaitingReview { needs_review: usize },
}

/// What happened to the outline, independently of whether the run produced a
/// PDF. `SafeRefusal` is a warning, never a failure: the document was still
/// converted, it simply carries no invented bookmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkOutcome {
    /// Confirmed or auto-confirmed entries were written to the outline.
    Written,
    /// The engine found structure but nothing reached a writable status —
    /// for example everything is still awaiting review.
    NoneConfirmed,
    /// No reliable structure was found, and nothing was invented.
    SafeRefusal,
}

impl BookmarkOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::NoneConfirmed => "none_confirmed",
            Self::SafeRefusal => "safe_refusal",
        }
    }
}

/// The independent checks that ran against the finished file before it was
/// committed to its final path. Every field is `true` because the run reached
/// this point: any of them failing aborts with `OutputValidationFailed` and no
/// output is left behind. They are reported so a caller can show *what* was
/// verified rather than asserting an unqualified "verified".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct OutputVerification {
    /// Page count, order, size and rotation/CropBox semantics were re-read
    /// from the written file and matched the MDP.
    pub geometry_matches_source: bool,
    /// The outline was walked again in the written file and matched exactly
    /// the confirmed and auto-confirmed tree.
    pub outline_matches_confirmed: bool,
    /// The source PDF was re-hashed at the final pre-commit boundary and was
    /// byte-identical to the source from which this derivative was built.
    pub source_unchanged: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalPdfOutcome {
    pub stage_reached: PipelineStage,
    pub halt: Option<Halt>,
    pub output_path: Option<PathBuf>,
    pub output_sha256: Option<String>,
    pub source_sha256: String,
    pub page_count: u32,
    pub ocr_pages: usize,
    pub ocr_errors: usize,
    pub bookmarks_written: usize,
    pub bookmarks_auto_confirmed: usize,
    pub bookmarks_needing_review: usize,
    /// Entries the engine retained for audit but judged too weak to propose.
    pub bookmarks_skipped: usize,
    pub bookmark_status: BookmarkOutcome,
    /// Present exactly when `bookmark_status` is
    /// [`BookmarkOutcome::SafeRefusal`]; kept in the report so the user is
    /// told why no outline was added.
    pub safe_refusal_reason: Option<String>,
    pub text_layer_words: usize,
    pub binarization: Option<ProcessingReport>,
    /// `Some` only for a run that produced and committed a PDF.
    pub verification: Option<OutputVerification>,
    /// Engine/model/version/digest/license/profile actually used, as recorded
    /// on the OCR evidence. Empty for a native-text-only document.
    pub ocr_provenance: Vec<String>,
    pub provider: String,
    /// The provider mode this run was started under, as an id. `local` unless
    /// a cloud mode was explicitly selected.
    pub provider_mode: String,
    /// Empty for a local run.
    pub cloud: CloudRunSummary,
}

/// Runs the whole pipeline. The only entry point a front end needs.
pub fn run(
    request: &FinalPdfRequest<'_>,
    progress: &dyn ProgressReporter,
    stage: &dyn Fn(PipelineStage),
) -> Result<FinalPdfOutcome> {
    let cancelled = || progress.is_cancelled();

    // ---------------------------------------------------------------- stage 1
    stage(PipelineStage::AnalyzingSource);
    guard_source_and_output(request)?;
    let open = PdfOpenOptions {
        password: request.password.clone(),
        pdfium: request.pdfium.clone(),
        compute_source_hash: true,
    };
    // One session for the whole run: the source is opened once, read-only.
    let session = PdfDocumentSession::open(request.source, &open)?;
    let source_sha256 = session
        .source_identity()
        .content_sha256
        .clone()
        .ok_or_else(|| CoreError::InvalidDocument("source digest was not captured".into()))?;
    let page_count = session.info().page_count;

    let package = load_or_create_package(request.workspace, &session, request.source)?;
    if package.source.content_sha256 != source_sha256 {
        return Err(CoreError::InvalidDocument(
            "workspace belongs to a different source PDF".into(),
        ));
    }
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 2
    // OCR reads renders of the ORIGINAL page. Nothing binarized exists yet,
    // which is the structural guarantee, not a comment.
    stage(PipelineStage::OcrOriginal);
    let (ocr_run, cloud) = run_ocr(request, &session, progress)?;
    let ocr_provenance = provenance_lines(&ocr_run);
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 3
    stage(PipelineStage::DerivingText);
    let mut derived = DerivedDocument::from_package(&package, Some(&ocr_run))?;
    // Human revisions stored in the workspace are part of the effective text
    // layer, and the persisted bookmark snapshot is bound to the *revised*
    // derived document. Applying them here is what lets a reviewed run reuse
    // the snapshot it produced instead of declaring it stale. AI-suggested
    // revisions are not applied by `apply_revisions`; only human ones are.
    let revisions = crate::derived::load_revisions(request.workspace)?;
    derived.apply_revisions(&revisions)?;
    let derived = derived;
    let text_layer_words: usize = derived
        .pages
        .iter()
        .flat_map(|page| page.blocks.iter())
        .flat_map(|block| block.lines.iter())
        .map(|line| line.words.len())
        .sum();
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 4
    stage(PipelineStage::GeneratingBookmarks);
    let generated = bookmarks::generate_auto_with_cancel(
        &bookmarks::AutoBookmarkInput {
            package: &package,
            ocr: Some(&ocr_run),
            derived: Some(&derived),
        },
        &request.bookmark_config,
        &cancelled,
    )?;
    let auto_confirmed = generated.auto_confirmed();
    let skipped = generated
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.status == BookmarkStatus::Skipped)
        .count();
    // A refusal is a normal, explained result about the *outline only*. It is
    // computed here and carried through; it never short-circuits the run.
    let safe_refusal_reason = (generated.report.mode == bookmarks::GenerationMode::SafeRefusal)
        .then(|| {
            generated
                .report
                .safe_refusal_reason
                .clone()
                .unwrap_or_else(|| "no reliable document structure was found".to_owned())
        });
    persist_generation(request, &generated)?;

    let reviews = bookmarks::load_reviews(request.workspace, &generated.snapshot)?;
    let effective = bookmarks::effective(&generated.snapshot, &reviews)?;
    // Report and pause on what is unresolved *after* applying the durable
    // human decisions. Counting the generation snapshot here would keep
    // already-confirmed/rejected entries labelled "needs review" forever.
    let needs_review = effective
        .iter()
        .filter(|candidate| candidate.status == BookmarkStatus::NeedsReview)
        .count();

    // ---------------------------------------------------------------- stage 5
    let base = FinalPdfOutcome {
        stage_reached: PipelineStage::GeneratingBookmarks,
        halt: None,
        output_path: None,
        output_sha256: None,
        source_sha256: source_sha256.clone(),
        page_count,
        ocr_pages: ocr_run.pages.len(),
        ocr_errors: ocr_run.errors.len(),
        bookmarks_written: 0,
        bookmarks_auto_confirmed: auto_confirmed,
        bookmarks_needing_review: needs_review,
        bookmarks_skipped: skipped,
        bookmark_status: if safe_refusal_reason.is_some() {
            BookmarkOutcome::SafeRefusal
        } else {
            BookmarkOutcome::NoneConfirmed
        },
        safe_refusal_reason: safe_refusal_reason.clone(),
        text_layer_words,
        binarization: None,
        verification: None,
        ocr_provenance,
        provider: request.provider.label().to_owned(),
        provider_mode: cloud.mode.unwrap_or(OcrProviderMode::Local).id().to_owned(),
        cloud,
    };

    if needs_review > 0 && request.review == ReviewPolicy::PauseForReview {
        stage(PipelineStage::AwaitingReview);
        return Ok(FinalPdfOutcome {
            stage_reached: PipelineStage::AwaitingReview,
            halt: Some(Halt::AwaitingReview { needs_review }),
            ..base
        });
    }

    let writable: Vec<BookmarkCandidate> = effective
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .cloned()
        .collect();
    // A bookmark refusal stops the *outline*, not the document. The OCR
    // evidence and the binarized pixels are unaffected by it, so the run
    // continues and produces a searchable bilevel PDF with no invented
    // bookmarks. Refusing to convert the document as well would throw away
    // work that is both valid and independently verifiable.
    let bookmark_status = if !writable.is_empty() {
        BookmarkOutcome::Written
    } else if safe_refusal_reason.is_some() {
        BookmarkOutcome::SafeRefusal
    } else {
        BookmarkOutcome::NoneConfirmed
    };
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 6
    // The text layer and the bookmark tree are now frozen. Only the *visible
    // pixels* are binarized, from the same open session, still reading the
    // original page renders.
    stage(PipelineStage::BinarizingVisuals);
    let parent = request.output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
    let bilevel = TempPdf::create(parent, "bilevel")?;
    let report = crate::pipeline::process_with_open_session(
        &session,
        bilevel.path(),
        request.settings,
        &PdfProcessingOptions {
            password: request.password.clone(),
            // The temporary carrier is ours and was just created empty.
            overwrite: true,
            validation: crate::validation::ValidationMode::Structural,
            pdfium: request.pdfium.clone(),
            prior_estimate: None,
            output_write_strategy: OutputWriteStrategy::AtomicSameDirectoryRename,
        },
        progress,
    )?;
    let bilevel_bytes = fs::read(bilevel.path()).map_err(|e| CoreError::io(bilevel.path(), e))?;
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 7
    // One assembly pass: binarized visible pages + the invisible text layer at
    // the ORIGINAL OCR coordinates + the confirmed outline.
    stage(PipelineStage::AssemblingFinalPdf);
    let built = crate::searchable_pdf::build_on_carrier(
        &bilevel_bytes,
        CarrierKind::NormalizedBilevel,
        &package,
        &effective,
        Some(&derived),
        &cancelled,
    )?;
    check_cancelled(&cancelled)?;

    // ---------------------------------------------------------------- stage 8
    stage(PipelineStage::Validating);
    let summary = crate::searchable_output::install_and_verify(
        &built,
        request.output,
        request.source,
        &package,
        &writable,
        &request.pdfium,
        request.overwrite,
        request.output_write_strategy,
        CarrierKind::NormalizedBilevel,
        &cancelled,
        // The source must be byte-identical to what it was when the run
        // started, and that has to be established while the destination is
        // still untouched — otherwise a mutated source can only be reported
        // after its stale derivative has already replaced the user's file.
        &|| {
            let after = fs::read(request.source).map_err(|e| CoreError::io(request.source, e))?;
            if hex(&Sha256::digest(&after)) != source_sha256 {
                return Err(CoreError::OutputValidationFailed(
                    "source PDF changed while its derivative was written".into(),
                ));
            }
            Ok(())
        },
    )?;
    drop(bilevel);
    progress.report(ProgressEvent::Finished);

    Ok(FinalPdfOutcome {
        stage_reached: PipelineStage::Validating,
        output_path: Some(summary.output_path),
        output_sha256: Some(summary.output_sha256),
        bookmarks_written: summary.written_bookmarks,
        bookmark_status,
        binarization: Some(report),
        verification: Some(OutputVerification {
            // `install_and_verify` re-read the geometry and walked the outline
            // of the written file; reaching here means both matched.
            geometry_matches_source: true,
            outline_matches_confirmed: true,
            source_unchanged: true,
        }),
        ..base
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    Ok(())
}

fn guard_source_and_output(request: &FinalPdfRequest<'_>) -> Result<()> {
    let metadata = fs::symlink_metadata(request.source)
        .map_err(|error| CoreError::io(request.source, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidDocument(
            "source PDF must be a regular file".into(),
        ));
    }
    if crate::searchable_output::same_path(request.output, request.source) {
        return Err(CoreError::DestinationConflict(
            "source and output must be distinct".into(),
        ));
    }
    if let Ok(existing) = fs::symlink_metadata(request.output) {
        if existing.file_type().is_symlink() || existing.is_dir() || !request.overwrite {
            return Err(CoreError::DestinationConflict(
                "output exists or is unsafe; pass overwrite for a regular file".into(),
            ));
        }
    }
    if ocr::required_model_files(&request.language_profile).is_none() {
        return Err(CoreError::InvalidParameter(format!(
            "unknown OCR language profile: {}",
            request.language_profile
        )));
    }
    Ok(())
}

/// Reuses an existing workspace package when it is already bound to this
/// source, and creates one otherwise. A workspace that belongs to a different
/// document is rejected rather than overwritten.
fn load_or_create_package(
    workspace: &Path,
    session: &PdfDocumentSession,
    source: &Path,
) -> Result<DocumentPackage> {
    if workspace.join("manifest.json").is_file() {
        return DocumentPackage::read_from(workspace);
    }
    // `write_to` creates the package directory itself and refuses to write
    // into an existing one, so only the parent is prepared here.
    if let Some(parent) = workspace.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| CoreError::io(parent, error))?;
        }
    }
    let package = DocumentPackage::create_from_session(
        session,
        source
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned),
    )?;
    package.write_to(workspace)?;
    DocumentPackage::read_from(workspace)
}

fn run_ocr(
    request: &FinalPdfRequest<'_>,
    session: &PdfDocumentSession,
    progress: &dyn ProgressReporter,
) -> Result<(OcrRun, CloudRunSummary)> {
    if progress.is_cancelled() {
        return Err(CoreError::Cancelled);
    }
    let local: Box<dyn PageOcrProvider> = match &request.provider {
        OcrProviderChoice::NativeTextOnly { diagnostic } => {
            Box::new(ocr::OptionalOcrPluginUnavailable {
                diagnostic: diagnostic.clone(),
            })
        }
        OcrProviderChoice::Reference => Box::new(ocr::ReferenceOcrProvider),
        OcrProviderChoice::Sidecar(config) => {
            Box::new(ocr::SidecarOcrProvider::from_sidecar(config.clone()))
        }
    };
    let store = crate::jobs::JobStore::open(&request.jobs_db)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    let fingerprint = job_fingerprint(request, &session.source_identity().content_sha256)?;
    let document_sha256 = session
        .source_identity()
        .content_sha256
        .clone()
        .unwrap_or_default();

    let Some(factory) = request.cloud else {
        // The default path. No provider abstraction is interposed and no
        // network stack is reachable from here, so a local run behaves and
        // checkpoints exactly as it did before cloud modes existed.
        let mut local = local;
        let run = ocr::run_session_durable_with_cancel(
            session,
            local.as_mut(),
            &store,
            &request.job_id,
            &fingerprint,
            request.workspace,
            &request.owner,
            request.ocr_dpi,
            &|| progress.is_cancelled(),
        )?;
        if matches!(&request.provider, OcrProviderChoice::NativeTextOnly { .. }) {
            if let Some(error) = run
                .errors
                .iter()
                .find(|error| error.code == "provider_unavailable")
            {
                return Err(CoreError::InvalidParameter(error.message.clone()));
            }
        }
        return Ok((run, CloudRunSummary::default()));
    };

    let mut cloud = factory.build(local)?;
    // Everything checkable without spending anything happens before the first
    // page image is encoded, let alone uploaded.
    let capabilities = cloud.capabilities();
    capabilities
        .validate_complete_ocr()
        .map_err(|error| CoreError::InvalidParameter(error.to_string()))?;
    cloud
        .validate_configuration()
        .map_err(|error| CoreError::InvalidParameter(error.to_string()))?;
    let ticket = cloud
        .prepare_job(&JobPreparation {
            job_id: request.job_id.clone(),
            document_sha256: document_sha256.clone(),
            page_indices: session.info().pages.iter().map(|page| page.index).collect(),
            language_profile: request.language_profile.clone(),
            dpi: request.ocr_dpi,
            max_credits: factory.max_credits(),
        })
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;

    let (result, fallback_pages, usage) = {
        let mut adapter = ProviderPageAdapter::new(
            cloud.as_mut(),
            document_sha256,
            request.language_profile.clone(),
            request.ocr_dpi,
            local_layout_version(request),
            factory.config_digest(),
        );
        let result = ocr::run_session_durable_with_cancel(
            session,
            &mut adapter,
            &store,
            &request.job_id,
            &fingerprint,
            request.workspace,
            &request.owner,
            request.ocr_dpi,
            &|| progress.is_cancelled(),
        );
        (result, adapter.fallback_pages(), adapter.usage())
    };

    let mut summary = CloudRunSummary {
        mode: Some(factory.mode()),
        fallback_pages,
        usage,
        reserved_credits: ticket.reserved_credits,
        estimated_credits: ticket.estimated_credits,
        not_production_ready_reason: capabilities.not_production_ready_reason.clone(),
        ..Default::default()
    };
    match result {
        Ok(run) => {
            let (job_fallback_pages, job_usage, usage_may_be_incomplete) =
                cloud_summary_from_run(&run)?;
            summary.fallback_pages = job_fallback_pages;
            summary.usage = job_usage;
            summary.usage_may_be_incomplete = usage_may_be_incomplete;
            // Settle on the way out, always: an unsettled reservation is a
            // hold the user cannot spend elsewhere.
            let settlement = cloud
                .finalize_job(&ticket)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
            summary.charged_credits = settlement.charged_credits;
            summary.released_credits = settlement.released_credits;
            summary.refunded_credits = settlement.refunded_credits;
            summary.settled = settlement.settled;
            Ok((run, summary))
        }
        Err(error) => {
            // A cancelled or failed run releases whatever it did not use. The
            // release is best-effort: reporting the original failure matters
            // more than reporting a failure to tidy up after it.
            let _ = cloud.cancel_job(&ticket);
            Err(error)
        }
    }
}

fn cloud_summary_from_run(run: &OcrRun) -> Result<(Vec<u32>, ProviderUsage, bool)> {
    let mut fallback_pages = Vec::new();
    let mut usage = ProviderUsage::default();
    let mut usage_may_be_incomplete = false;
    for page in &run.pages {
        let Some(provenance) = page.provider_provenance.as_ref() else {
            continue;
        };
        let parameters = &provenance.parameters;
        if let Some(reason) = parameters.get("fallback_reason") {
            fallback_pages.push(page.page_index);
            usage_may_be_incomplete |= reason == "transport_outcome_unknown";
        }
        let input_tokens = parse_usage_parameter(parameters, "usage_input_tokens")?;
        let output_tokens = parse_usage_parameter(parameters, "usage_output_tokens")?;
        let credits_charged = parse_usage_parameter(parameters, "usage_credits_charged")?;
        let explicit_requests = parse_usage_parameter(parameters, "usage_requests")?;
        let has_usage = [
            "usage_input_tokens",
            "usage_output_tokens",
            "usage_credits_charged",
            "usage_requests",
        ]
        .iter()
        .any(|key| parameters.contains_key(*key));
        let requests = if parameters.contains_key("usage_requests") {
            u32::try_from(explicit_requests).map_err(|_| {
                CoreError::InvalidDocument("provider usage request count is out of range".into())
            })?
        } else if has_usage {
            // Compatibility with provider pages written before requests were
            // persisted explicitly: one page response represented one call.
            1
        } else {
            0
        };
        usage.add(ProviderUsage {
            input_tokens,
            output_tokens,
            credits_charged,
            requests,
        });
    }
    fallback_pages.sort_unstable();
    fallback_pages.dedup();
    Ok((fallback_pages, usage, usage_may_be_incomplete))
}

fn parse_usage_parameter(
    parameters: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> Result<u64> {
    parameters
        .get(key)
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                CoreError::InvalidDocument(format!(
                    "provider provenance contains an invalid {key} value"
                ))
            })
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Identity of the local detector whose rectangles a cloud transcription is
/// aligned to. Different weights measure different lines, so this belongs in
/// the fingerprint of any aligned page.
fn local_layout_version(request: &FinalPdfRequest<'_>) -> String {
    match &request.provider {
        OcrProviderChoice::NativeTextOnly { .. } => "native-text-only".to_owned(),
        OcrProviderChoice::Reference => "reference".to_owned(),
        OcrProviderChoice::Sidecar(config) => {
            let mut identity = format!(
                "{}/{}/{}",
                config.engine.as_str(),
                config.language_profile,
                config.passes.fingerprint()
            );
            // PSM 6 is the historical default and keeps old checkpoint
            // identities stable. Geometry-only PSM 3 is a distinct layout
            // producer and therefore must not reuse those checkpoints.
            if config.psm != 6 {
                identity.push_str(&format!("/psm={}", config.psm));
            }
            identity
        }
    }
}

/// The fingerprint that decides whether a durable job may be resumed.
///
/// It includes the source, engine, sidecar bytes, every required model file,
/// language profile and OCR DPI. Replacing a model *at the same path* must
/// therefore invalidate the durable job rather than silently reusing evidence
/// produced by different weights.
fn job_fingerprint(
    request: &FinalPdfRequest<'_>,
    source_sha256: &Option<String>,
) -> Result<String> {
    let source_sha256 = source_sha256.as_deref().ok_or_else(|| {
        CoreError::InvalidDocument("source digest is required for OCR checkpointing".into())
    })?;
    let mut identity = format!(
        "mpdf-final-pdf/2|source={source_sha256}|dpi={}|profile={}",
        request.ocr_dpi, request.language_profile
    );
    match &request.provider {
        OcrProviderChoice::NativeTextOnly { .. } => {
            identity.push_str("|engine=native-text-only|optional_local_ocr_plugin=absent")
        }
        OcrProviderChoice::Reference => identity.push_str("|engine=reference"),
        OcrProviderChoice::Sidecar(config) => {
            identity.push_str(&format!(
                "|engine={}|provider_profile={}|passes={}|sidecar={}",
                config.engine.as_str(),
                config.language_profile,
                config.passes.fingerprint(),
                hash_file(&config.executable)?
            ));
            // Preserve the historical fingerprint for developer-configured
            // PATH Tesseract runs, while binding bundled runs to the exact
            // engine path and bytes they actually execute.
            if let Some(path) = &config.engine_binary {
                identity.push_str(&format!(
                    "|engine_binary={}:{}",
                    path.display(),
                    hash_file(path)?
                ));
            }
            if config.psm != 6 {
                identity.push_str(&format!("|psm={}", config.psm));
            }
            let mut names = config.required_files.clone();
            names.sort();
            names.dedup();
            for name in names {
                let path = config.model_dir.join(&name);
                identity.push_str(&format!("|model={name}:{}", hash_file(&path)?));
            }
        }
    }
    // Cloud identity is *appended*, never interleaved. A local run therefore
    // produces byte-identical input to the digest that it did before cloud
    // modes existed, so existing local checkpoints stay resumable; and the
    // three modes can never collide, because the appended segment names the
    // mode, the model and its pinned version, the prompt digest, the local
    // layout identity, the alignment version, the structured-bbox policy and
    // the fallback policy. Nothing derived from a credential appears here.
    if let Some(factory) = request.cloud {
        identity.push_str(&format!(
            "|cloud={}|{}",
            factory.mode().id(),
            factory.fingerprint_contribution()
        ));
    }
    Ok(hex(&Sha256::digest(identity.as_bytes())))
}

fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(|error| CoreError::io(path, error))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| CoreError::io(path, error))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex(&digest.finalize()))
}

fn persist_generation(
    request: &FinalPdfRequest<'_>,
    generated: &bookmarks::AutoBookmarkResult,
) -> Result<()> {
    let exists = bookmarks::candidates_path(request.workspace).exists();
    if exists {
        let previous = bookmarks::load_snapshot(request.workspace)?;
        if previous == generated.snapshot {
            // Identical regeneration: keep the existing file (and any reviews
            // attached to it) rather than churning it.
            return Ok(());
        }
        let reviews = bookmarks::load_reviews(request.workspace, &previous)?;
        if !reviews.operations.is_empty() {
            return Err(CoreError::DestinationConflict(
                "the workspace holds human review decisions for a different generation; \
                 the evidence changed, so those decisions are stale and are not migrated"
                    .into(),
            ));
        }
    }
    bookmarks::save_generation(request.workspace, generated, exists)
}

fn provenance_lines(run: &OcrRun) -> Vec<String> {
    let mut lines: Vec<String> = run
        .pages
        .iter()
        .filter_map(|page| page.provider_provenance.as_ref())
        .map(|provenance| {
            format!(
                "engine={} model={} version={} profile={} model_set={} license={} digest={}",
                provenance.engine,
                provenance.model,
                provenance.version,
                provenance.language_profile.as_deref().unwrap_or("-"),
                provenance.model_set.as_deref().unwrap_or("-"),
                provenance.model_license.as_deref().unwrap_or("-"),
                provenance.model_digest.as_deref().unwrap_or("-"),
            )
        })
        .collect();
    lines.sort();
    lines.dedup();
    lines
}

/// A temporary PDF in the destination's own directory, removed on drop.
struct TempPdf {
    path: PathBuf,
}

impl TempPdf {
    fn create(directory: &Path, label: &str) -> Result<Self> {
        let path = directory.join(format!(
            "{}{label}-{}-{}.pdf",
            crate::pipeline::TEMP_FILE_PREFIX,
            std::process::id(),
            unique()
        ));
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn unique() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_order_is_the_documented_pipeline() {
        let names: Vec<&str> = PipelineStage::ORDER
            .iter()
            .map(|stage| stage.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "analyzing_source",
                "ocr_original",
                "deriving_text",
                "generating_bookmarks",
                "awaiting_review",
                "binarizing_visuals",
                "assembling_final_pdf",
                "validating",
            ]
        );
    }

    #[test]
    fn ocr_always_precedes_binarization_in_the_stage_order() {
        // A structural guard: if someone reorders the enum so binarization
        // could run before OCR, this fails.
        let position = |stage: PipelineStage| {
            PipelineStage::ORDER
                .iter()
                .position(|item| *item == stage)
                .unwrap()
        };
        assert!(position(PipelineStage::OcrOriginal) < position(PipelineStage::BinarizingVisuals));
        assert!(position(PipelineStage::DerivingText) < position(PipelineStage::BinarizingVisuals));
        assert!(
            position(PipelineStage::GeneratingBookmarks)
                < position(PipelineStage::BinarizingVisuals)
        );
        assert!(
            position(PipelineStage::BinarizingVisuals)
                < position(PipelineStage::AssemblingFinalPdf)
        );
    }

    #[test]
    fn a_bookmark_refusal_is_not_a_halt() {
        // The combined main flow may stop for exactly one reason: a human has
        // to decide something. If a second variant is ever added here, this
        // match stops compiling — which is the point. Refusing to invent an
        // outline must never again become a reason to withhold the converted,
        // searchable PDF.
        let halt = Halt::AwaitingReview { needs_review: 1 };
        match halt {
            Halt::AwaitingReview { needs_review } => assert_eq!(needs_review, 1),
        }
    }

    #[test]
    fn the_outline_result_is_reported_separately_from_the_run_result() {
        assert_eq!(BookmarkOutcome::Written.as_str(), "written");
        assert_eq!(BookmarkOutcome::NoneConfirmed.as_str(), "none_confirmed");
        assert_eq!(BookmarkOutcome::SafeRefusal.as_str(), "safe_refusal");
    }

    #[test]
    fn cloud_summary_is_rebuilt_from_all_persisted_page_provenance() {
        use std::collections::BTreeMap;

        use crate::bookmark_fixtures::{ocr_run, FixtureLine, FixturePage};
        use crate::jobs::ExecutionLocation;
        use crate::ocr::OcrProviderProvenance;

        let parameters = BTreeMap::from([
            ("provider_mode".into(), "gemini_byok".into()),
            ("fallback_reason".into(), "transport_outcome_unknown".into()),
            ("usage_input_tokens".into(), "120".into()),
            ("usage_output_tokens".into(), "30".into()),
            ("usage_credits_charged".into(), "0".into()),
            ("usage_requests".into(), "2".into()),
        ]);
        let provenance = OcrProviderProvenance {
            engine: "gemini".into(),
            model: "gemini-test".into(),
            version: "test".into(),
            parameters,
            input_asset_sha256: "a".repeat(64),
            execution_location: ExecutionLocation::LegacyUserKey,
            language_profile: Some("greek-ancient".into()),
            model_digest: None,
            model_license: None,
            model_set: None,
        };
        let pages = vec![FixturePage::new(vec![FixtureLine::new(
            "λόγος",
            10.0,
            10.0,
        )])];
        let run = ocr_run(&pages, Some(provenance));

        let (fallback_pages, usage, usage_may_be_incomplete) =
            cloud_summary_from_run(&run).unwrap();
        assert_eq!(fallback_pages, vec![0]);
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.requests, 2);
        assert!(usage_may_be_incomplete);
    }

    #[test]
    fn job_fingerprint_binds_source_sidecar_models_and_profile() {
        let directory = tempfile::tempdir().unwrap();
        let sidecar = directory.path().join("sidecar");
        fs::write(&sidecar, b"sidecar-v1").unwrap();
        let models_a = directory.path().join("models-a");
        let models_b = directory.path().join("models-b");
        for model_dir in [&models_a, &models_b] {
            fs::create_dir(model_dir).unwrap();
            for name in [
                "grc.traineddata",
                "deu.traineddata",
                "eng.traineddata",
                "osd.traineddata",
                "manifest.json",
            ] {
                fs::write(model_dir.join(name), format!("model:{name}")).unwrap();
            }
        }
        let settings = ProcessingSettings {
            dpi: 300,
            method: crate::settings::BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: crate::settings::PreprocessingSettings::default(),
            cleanup: crate::cleanup::CleanupSettings::default(),
        };
        let make = |profile: &str, dir: &Path, source_digest: &str| {
            let source = PathBuf::from("/tmp/a.pdf");
            let output = PathBuf::from("/tmp/b.pdf");
            let workspace = PathBuf::from("/tmp/ws");
            let request = FinalPdfRequest {
                source: Box::leak(source.into_boxed_path()),
                output: Box::leak(output.into_boxed_path()),
                workspace: Box::leak(workspace.into_boxed_path()),
                settings: &settings,
                provider: OcrProviderChoice::Sidecar(
                    ocr::SidecarOcrConfig::tesseract(sidecar.clone(), dir.to_path_buf(), profile)
                        .unwrap(),
                ),
                language_profile: profile.to_owned(),
                review: ReviewPolicy::PauseForReview,
                overwrite: false,
                password: None,
                pdfium: PdfiumConfig::default(),
                output_write_strategy: OutputWriteStrategy::default(),
                bookmark_config: AutoBookmarkConfig::default(),
                ocr_dpi: 300,
                job_id: "job".into(),
                jobs_db: PathBuf::from("/tmp/jobs.db"),
                owner: "test".into(),
                cloud: None,
            };
            job_fingerprint(&request, &Some(source_digest.to_owned())).unwrap()
        };
        let source_a = "a".repeat(64);
        let source_b = "b".repeat(64);
        let baseline = make("auto", &models_a, &source_a);
        assert_ne!(baseline, make("auto", &models_a, &source_b));
        assert_ne!(baseline, make("german", &models_a, &source_a));
        assert_eq!(baseline, make("auto", &models_b, &source_a));

        fs::write(models_a.join("grc.traineddata"), b"changed model bytes").unwrap();
        assert_ne!(baseline, make("auto", &models_a, &source_a));
        fs::write(models_a.join("grc.traineddata"), b"model:grc.traineddata").unwrap();
        fs::write(&sidecar, b"sidecar-v2").unwrap();
        assert_ne!(baseline, make("auto", &models_a, &source_a));
    }

    #[test]
    fn toggling_an_optional_sidecar_pass_invalidates_the_checkpoint() {
        // Turning a pass on or off changes what the evidence *is*, so a
        // resumed job must not be allowed to mix pages recognized under both
        // settings. The passes are therefore part of the fingerprint, not
        // left to the sidecar's own defaults.
        let directory = tempfile::tempdir().unwrap();
        let sidecar = directory.path().join("sidecar");
        fs::write(&sidecar, b"sidecar-v1").unwrap();
        let models = directory.path().join("models");
        fs::create_dir(&models).unwrap();
        for name in ocr::required_model_files("auto").unwrap() {
            fs::write(models.join(&name), format!("model:{name}")).unwrap();
        }
        let settings = ProcessingSettings {
            dpi: 300,
            method: crate::settings::BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: crate::settings::PreprocessingSettings::default(),
            cleanup: crate::cleanup::CleanupSettings::default(),
        };
        let make = |passes: ocr::SidecarPasses| {
            let mut config =
                ocr::SidecarOcrConfig::tesseract(sidecar.clone(), models.clone(), "auto").unwrap();
            config.passes = passes;
            let request = FinalPdfRequest {
                source: Box::leak(PathBuf::from("/tmp/a.pdf").into_boxed_path()),
                output: Box::leak(PathBuf::from("/tmp/b.pdf").into_boxed_path()),
                workspace: Box::leak(PathBuf::from("/tmp/ws").into_boxed_path()),
                settings: &settings,
                provider: OcrProviderChoice::Sidecar(config),
                language_profile: "auto".into(),
                review: ReviewPolicy::PauseForReview,
                overwrite: false,
                password: None,
                pdfium: PdfiumConfig::default(),
                output_write_strategy: OutputWriteStrategy::default(),
                bookmark_config: AutoBookmarkConfig::default(),
                ocr_dpi: 300,
                job_id: "job".into(),
                jobs_db: PathBuf::from("/tmp/jobs.db"),
                owner: "test".into(),
                cloud: None,
            };
            job_fingerprint(&request, &Some("a".repeat(64))).unwrap()
        };
        // All eight combinations must be distinguishable: a resumed job may
        // never mix pages recognized under different passes.
        let mut seen = std::collections::BTreeSet::new();
        for greek in [false, true] {
            for small in [false, true] {
                for detached in [false, true] {
                    let fingerprint = make(ocr::SidecarPasses {
                        greek_routing: greek,
                        small_type_latin: small,
                        detached_greek_accents: detached,
                    });
                    assert!(
                        seen.insert(fingerprint),
                        "passes {greek}/{small}/{detached} collided with another combination"
                    );
                }
            }
        }
        assert_eq!(seen.len(), 8);
        assert_eq!(
            make(ocr::SidecarPasses::default()),
            make(ocr::SidecarPasses {
                greek_routing: false,
                small_type_latin: false,
                detached_greek_accents: false,
            }),
            "the default must be exactly all-off"
        );
    }

    /// A stand-in for `mpdf-api-client`'s real factories. It supplies only
    /// the non-secret identity the fingerprint is allowed to see, which is
    /// also the point of the test: nothing else is available to it.
    struct TestCloudFactory {
        mode: OcrProviderMode,
        contribution: String,
    }

    impl CloudProviderFactory for TestCloudFactory {
        fn mode(&self) -> OcrProviderMode {
            self.mode
        }
        fn config_digest(&self) -> String {
            self.contribution.clone()
        }
        fn fingerprint_contribution(&self) -> String {
            self.contribution.clone()
        }
        fn max_credits(&self) -> u64 {
            0
        }
        fn fallback(&self) -> CloudFallback {
            CloudFallback::Local
        }
        fn build(&self, _local: Box<dyn PageOcrProvider>) -> Result<Box<dyn OcrProvider>> {
            Err(CoreError::InvalidParameter("test factory".into()))
        }
    }

    fn fingerprint_with(
        cloud: Option<&dyn CloudProviderFactory>,
        models: &Path,
        sidecar: &Path,
    ) -> String {
        let settings = ProcessingSettings {
            dpi: 300,
            method: crate::settings::BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: crate::settings::PreprocessingSettings::default(),
            cleanup: crate::cleanup::CleanupSettings::default(),
        };
        let request = FinalPdfRequest {
            source: Box::leak(PathBuf::from("/tmp/a.pdf").into_boxed_path()),
            output: Box::leak(PathBuf::from("/tmp/b.pdf").into_boxed_path()),
            workspace: Box::leak(PathBuf::from("/tmp/ws").into_boxed_path()),
            settings: &settings,
            provider: OcrProviderChoice::Sidecar(
                ocr::SidecarOcrConfig::tesseract(
                    sidecar.to_path_buf(),
                    models.to_path_buf(),
                    "auto",
                )
                .unwrap(),
            ),
            language_profile: "auto".into(),
            review: ReviewPolicy::PauseForReview,
            overwrite: false,
            password: None,
            pdfium: PdfiumConfig::default(),
            output_write_strategy: OutputWriteStrategy::default(),
            bookmark_config: AutoBookmarkConfig::default(),
            ocr_dpi: 300,
            job_id: "job".into(),
            jobs_db: PathBuf::from("/tmp/jobs.db"),
            owner: "test".into(),
            cloud,
        };
        job_fingerprint(&request, &Some("a".repeat(64))).unwrap()
    }

    fn frozen_fixture(directory: &Path) -> (PathBuf, PathBuf) {
        let sidecar = directory.join("sidecar");
        fs::write(&sidecar, b"sidecar-v1").unwrap();
        let models = directory.join("models");
        fs::create_dir(&models).unwrap();
        for name in ocr::required_model_files("auto").unwrap() {
            fs::write(models.join(&name), format!("model:{name}")).unwrap();
        }
        (sidecar, models)
    }

    #[test]
    fn a_local_run_fingerprints_exactly_as_it_did_before_cloud_modes_existed() {
        // The regression this locks: adding the provider architecture must
        // not invalidate a single existing local checkpoint. The value below
        // is the digest this fixture produced *before* `cloud` was appended
        // to the identity string — reconstructed independently from the
        // documented format — so any future change that reorders or
        // interleaves the local segments will fail here rather than silently
        // discarding a user's half-finished 400-page run.
        let directory = tempfile::tempdir().unwrap();
        let (sidecar, models) = frozen_fixture(directory.path());
        assert_eq!(
            fingerprint_with(None, &models, &sidecar),
            "e558d678fafa7e8963e749a497d3562942b9ef73e057cb646d774eeb1c773c11"
        );
    }

    #[test]
    fn the_three_provider_modes_can_never_share_a_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let (sidecar, models) = frozen_fixture(directory.path());
        let byok = TestCloudFactory {
            mode: OcrProviderMode::GeminiByok,
            contribution: "model=gemini-3.7-flash|prompt=abc".into(),
        };
        let credits = TestCloudFactory {
            mode: OcrProviderMode::MpdfCredits,
            // Same model, same prompt: only the execution changes, and that
            // still has to be a different checkpoint.
            contribution: "model=gemini-3.7-flash|prompt=abc".into(),
        };
        let fingerprints = [
            fingerprint_with(None, &models, &sidecar),
            fingerprint_with(Some(&byok), &models, &sidecar),
            fingerprint_with(Some(&credits), &models, &sidecar),
        ];
        let unique: std::collections::BTreeSet<_> = fingerprints.iter().collect();
        assert_eq!(unique.len(), 3, "{fingerprints:?}");
    }

    #[test]
    fn changing_a_cloud_prompt_or_alignment_invalidates_the_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let (sidecar, models) = frozen_fixture(directory.path());
        let before = TestCloudFactory {
            mode: OcrProviderMode::GeminiByok,
            contribution: "prompt=abc|alignment=v1".into(),
        };
        let after = TestCloudFactory {
            mode: OcrProviderMode::GeminiByok,
            contribution: "prompt=abc|alignment=v2".into(),
        };
        assert_ne!(
            fingerprint_with(Some(&before), &models, &sidecar),
            fingerprint_with(Some(&after), &models, &sidecar)
        );
    }

    #[test]
    fn optional_passes_are_off_by_default_and_spelled_out_on_the_command_line() {
        let passes = ocr::SidecarPasses::default();
        assert!(
            !passes.greek_routing && !passes.small_type_latin && !passes.detached_greek_accents
        );
        // Explicit "off" rather than an omitted flag: the run must reproduce
        // the combined baseline whatever the sidecar's own default becomes.
        assert_eq!(passes.routing_arg(), "off");
        assert_eq!(passes.small_type_arg(), "off");
        assert_eq!(passes.detached_accents_arg(), "off");
    }
}
