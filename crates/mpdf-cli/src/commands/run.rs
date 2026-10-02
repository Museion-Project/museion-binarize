//! `mpdf run` — the single main-flow command.
//!
//! Everything ordering-related lives in [`mpdf_core::orchestrator`]. This file
//! only turns command-line arguments into a request, reports stages, and
//! prints the result. It must never re-implement the pipeline order, because a
//! second implementation is exactly how "OCR the binarized output" gets
//! reintroduced.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mpdf_core::error::CoreError;
use mpdf_core::ocr::{self, SidecarOcrConfig};
use mpdf_core::ocr_provider::credits;
use mpdf_core::ocr_provider::{OcrProviderMode, BYOK_DISABLED_MESSAGE};
use mpdf_core::orchestrator::{
    self, CloudProviderFactory, FinalPdfOutcome, FinalPdfRequest, Halt, OcrProviderChoice,
    OutputVerification, PipelineStage, ReviewPolicy,
};
use mpdf_core::pipeline::OutputWriteStrategy;

use crate::cli::{CloudFallbackArg, OcrProviderModeArg, ReviewPolicyArg, RunArgs, RunProviderArg};
use crate::errors::{self, ExitReason};
use crate::output;
use crate::progress::StderrProgress;

#[derive(serde::Serialize)]
struct RunReport {
    schema: &'static str,
    schema_version: &'static str,
    status: &'static str,
    stage_reached: String,
    /// The stages this run actually entered, in the order they were entered.
    /// Reported rather than assumed, so a reader can see that OCR really did
    /// precede binarization in this specific run.
    stages: Vec<String>,
    /// `written`, `none_confirmed`, or `safe_refusal`. Independent of
    /// `status`: a safe refusal still completes and still writes a PDF.
    bookmark_status: &'static str,
    output_path: Option<String>,
    output_sha256: Option<String>,
    source_sha256: String,
    page_count: u32,
    ocr_pages: usize,
    ocr_errors: usize,
    bookmarks_written: usize,
    bookmarks_auto_confirmed: usize,
    bookmarks_needing_review: usize,
    bookmarks_skipped: usize,
    text_layer_words: usize,
    output_verification: Option<OutputVerification>,
    ocr_provenance: Vec<String>,
    provider: String,
    /// `local`, `gemini-byok` or `mpdf-credits`: the mode this run was
    /// started under, not the mode every page ended up using.
    provider_mode: String,
    /// Pages whose text came from the local engine even though a cloud mode
    /// was selected. Reported per page, never summarized away.
    cloud_fallback_pages: Vec<u32>,
    cloud_input_tokens: u64,
    cloud_output_tokens: u64,
    cloud_usage_may_be_incomplete: bool,
    credits_reserved: u64,
    credits_charged: u64,
    credits_released: u64,
    credits_refunded: u64,
    language_profile: String,
    refusal_reason: Option<String>,
    workspace: String,
}

pub fn run(args: RunArgs) -> ExitCode {
    let settings = match args.settings.to_settings() {
        Ok(settings) => settings,
        Err(message) => return usage(&message),
    };
    if ocr::required_model_files(&args.language).is_none() {
        return usage(&format!(
            "unknown --language {}; expected one of {:?}",
            args.language,
            ocr::OCR_LANGUAGE_PROFILES
        ));
    }

    let workspace = args
        .workspace
        .clone()
        .unwrap_or_else(|| default_workspace(&args.output));
    // Cloud policy first: consent, endpoint and ceiling are refused before the
    // local provider is even resolved, so "you have no sidecar" never masks
    // "you did not authorize an upload".
    let cloud = match build_cloud_factory(&args) {
        Ok(cloud) => cloud,
        Err(message) => return usage(&message),
    };
    if args.dry_run {
        // Nothing has been opened, uploaded, reserved or charged, and nothing
        // will be: a dry run exists precisely so a user can read the plan
        // before authorizing any of that.
        print_dry_run(&args, &workspace);
        return ExitReason::Success.exit_code();
    }
    let provider = match build_provider(&args, &workspace) {
        Ok(provider) => provider,
        Err(message) => return usage(&message),
    };

    let jobs_db = workspace.join("jobs.sqlite3");
    // The provider mode is part of the job identity so a resumed run can
    // never mix pages recognized under different modes into one job.
    let job_id = format!(
        "run-{}-{}",
        provider_mode(args.ocr_provider).id(),
        args.language
    );
    let progress = StderrProgress::new(args.output_mode.quiet || args.output_mode.json);
    let quiet = args.output_mode.quiet || args.output_mode.json;

    let request = FinalPdfRequest {
        source: &args.input,
        output: &args.output,
        workspace: &workspace,
        settings: &settings,
        provider,
        language_profile: args.language.clone(),
        review: match args.on_review {
            ReviewPolicyArg::Pause => ReviewPolicy::PauseForReview,
            ReviewPolicyArg::Confirmed => ReviewPolicy::ContinueWithConfirmed,
            ReviewPolicyArg::Reviewed => ReviewPolicy::UseExistingDecisions,
        },
        overwrite: args.overwrite,
        password: None,
        pdfium: args.pdfium.to_config(),
        output_write_strategy: OutputWriteStrategy::AtomicSameDirectoryRename,
        bookmark_config: Default::default(),
        ocr_dpi: args.ocr_dpi,
        job_id,
        jobs_db,
        owner: "mpdf-cli".into(),
        cloud: cloud.as_ref().map(|factory| factory.as_ref()),
    };

    let stages = std::cell::RefCell::new(Vec::new());
    let outcome = orchestrator::run(&request, &progress, &|stage| {
        stages.borrow_mut().push(stage.as_str().to_owned());
        if !quiet {
            eprintln!("[{}/{}] {}", stage_number(stage), 8, describe(stage));
        }
    });
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return fail(&error, args.output_mode.json, args.output_mode.pretty),
    };

    let report = build_report(&outcome, &args.language, &workspace, stages.into_inner());
    if args.output_mode.json {
        output::print_json(&report, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        print_human(&outcome, &workspace);
    }
    ExitReason::Success.exit_code()
}

fn build_report(
    outcome: &FinalPdfOutcome,
    language: &str,
    workspace: &Path,
    stages: Vec<String>,
) -> RunReport {
    // A bookmark refusal is not a run status. The run either completed or is
    // waiting for a reviewer; the outline result is reported separately.
    let status = match &outcome.halt {
        None => "completed",
        Some(Halt::AwaitingReview { .. }) => "awaiting_review",
    };
    RunReport {
        schema: "mpdf-run",
        schema_version: "1.2",
        status,
        stage_reached: outcome.stage_reached.as_str().to_owned(),
        stages,
        bookmark_status: outcome.bookmark_status.as_str(),
        output_path: outcome
            .output_path
            .as_ref()
            .map(|path| path.display().to_string()),
        output_sha256: outcome.output_sha256.clone(),
        source_sha256: outcome.source_sha256.clone(),
        page_count: outcome.page_count,
        ocr_pages: outcome.ocr_pages,
        ocr_errors: outcome.ocr_errors,
        bookmarks_written: outcome.bookmarks_written,
        bookmarks_auto_confirmed: outcome.bookmarks_auto_confirmed,
        bookmarks_needing_review: outcome.bookmarks_needing_review,
        bookmarks_skipped: outcome.bookmarks_skipped,
        text_layer_words: outcome.text_layer_words,
        output_verification: outcome.verification,
        ocr_provenance: outcome.ocr_provenance.clone(),
        provider: outcome.provider.clone(),
        provider_mode: outcome.provider_mode.clone(),
        cloud_fallback_pages: outcome.cloud.fallback_pages.clone(),
        cloud_input_tokens: outcome.cloud.usage.input_tokens,
        cloud_output_tokens: outcome.cloud.usage.output_tokens,
        cloud_usage_may_be_incomplete: outcome.cloud.usage_may_be_incomplete,
        credits_reserved: outcome.cloud.reserved_credits,
        credits_charged: outcome.cloud.charged_credits,
        credits_released: outcome.cloud.released_credits,
        credits_refunded: outcome.cloud.refunded_credits,
        language_profile: language.to_owned(),
        refusal_reason: outcome.safe_refusal_reason.clone(),
        workspace: workspace.display().to_string(),
    }
}

fn print_human(outcome: &FinalPdfOutcome, workspace: &Path) {
    match &outcome.halt {
        Some(Halt::AwaitingReview { needs_review }) => {
            println!(
                "paused for review: {needs_review} bookmark entr{} need a decision.\n\
                 Nothing was written. Review with `mpdf bookmark list {}`, then rerun \
                 with --on-review reviewed.",
                if *needs_review == 1 { "y" } else { "ies" },
                workspace.display()
            );
        }
        None => {
            let path = outcome
                .output_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            println!(
                "done: {path}\n  {} page(s), {} OCR page(s), {} word(s) in the text layer\n  \
                 {} bookmark(s) written ({} automatic), {} still needing review",
                outcome.page_count,
                outcome.ocr_pages,
                outcome.text_layer_words,
                outcome.bookmarks_written,
                outcome.bookmarks_auto_confirmed,
                outcome.bookmarks_needing_review,
            );
            for line in &outcome.ocr_provenance {
                println!("  ocr: {line}");
            }
            print_cloud(outcome);
            // A warning, not a failure: the PDF above exists and is verified.
            if let Some(reason) = &outcome.safe_refusal_reason {
                println!(
                    "  warning: no automatic bookmarks were added: {reason}\n                       The document was still converted and is searchable; only the \n                       outline was left empty rather than invented."
                );
            }
        }
    }
}

/// Reports what a cloud run actually did.
///
/// The fallback list is printed page by page rather than as a count. "12
/// pages fell back" invites the reader to shrug; "pages 3, 17, 41 … are local
/// text" is something they can act on.
fn print_cloud(outcome: &FinalPdfOutcome) {
    if outcome.cloud.mode.is_none() {
        return;
    }
    println!("  provider mode: {}", outcome.provider_mode);
    if outcome.cloud.fallback_pages.is_empty() {
        println!("  every OCR'd page used the selected cloud provider");
    } else {
        println!(
            "  {} page(s) fell back to local OCR: {}",
            outcome.cloud.fallback_pages.len(),
            outcome
                .cloud
                .fallback_pages
                .iter()
                .map(|page| (page + 1).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if outcome.cloud.usage.requests > 0 {
        println!(
            "  provider usage: {} request(s), {} input token(s), {} output token(s)",
            outcome.cloud.usage.requests,
            outcome.cloud.usage.input_tokens,
            outcome.cloud.usage.output_tokens
        );
    }
    if outcome.cloud.usage_may_be_incomplete {
        println!(
            "  warning: at least one provider request had an unknown outcome; token and cost totals are lower bounds"
        );
    }
    if outcome.cloud.reserved_credits > 0 || outcome.cloud.charged_credits > 0 {
        println!(
            "  credits: {} reserved, {} charged, {} released, {} refunded{}",
            outcome.cloud.reserved_credits,
            outcome.cloud.charged_credits,
            outcome.cloud.released_credits,
            outcome.cloud.refunded_credits,
            if outcome.cloud.settled {
                ""
            } else {
                " (NOT settled)"
            }
        );
    }
    if let Some(reason) = &outcome.cloud.not_production_ready_reason {
        println!("  warning: this provider mode is not production ready: {reason}");
    }
}

/// The workspace sits beside the output, named after it, so two conversions in
/// one directory never share evidence.
fn default_workspace(output: &Path) -> PathBuf {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    parent.join(format!(".{stem}.mpdf-workspace"))
}

fn build_provider(args: &RunArgs, workspace: &Path) -> Result<OcrProviderChoice, String> {
    if args.ocr_provider == OcrProviderModeArg::BrokerTest {
        return Ok(OcrProviderChoice::NativeTextOnly {
            diagnostic: "broker-test constructs its frozen Surya detector through the shared factory; no fallback".into(),
        });
    }
    let engine = match args.provider {
        RunProviderArg::Reference => return Ok(OcrProviderChoice::Reference),
        RunProviderArg::Tesseract => ocr::OcrEngine::Tesseract,
        RunProviderArg::Paddleocr => ocr::OcrEngine::PaddleOcr,
    };
    let _ = workspace;
    let explicit_executable = args
        .ocr_sidecar
        .clone()
        .or_else(|| std::env::var_os("MPDF_OCR_SIDECAR").map(PathBuf::from));
    let explicit_models = args
        .models
        .clone()
        .or_else(|| std::env::var_os("MPDF_OCR_MODELS").map(PathBuf::from));
    let (executable, model_dir, engine_binary) = match (explicit_executable, explicit_models) {
        (Some(executable), Some(model_dir)) => (executable, model_dir, None),
        (None, None) => match mpdf_core::ocr_runtime::resolve_executable_adjacent(&args.language) {
            Ok(Some(runtime)) => (runtime.sidecar, runtime.model_dir, Some(runtime.engine)),
            Ok(None) => return Ok(OcrProviderChoice::NativeTextOnly {
                diagnostic: "this scanned or image-only page requires the optional local OCR plugin; install and verify the offline OCR runtime, or pass --ocr-sidecar and --models. PDFs with a reliable native text layer do not require the plugin".to_owned(),
            }),
            Err(error) => return Err(format!("bundled OCR runtime refused: {error}")),
        },
        (None, Some(_)) => return Err("OCR models were configured but no sidecar was configured".into()),
        (Some(_), None) => return Err("OCR sidecar was configured but no models were configured".into()),
    };
    let mut config = SidecarOcrConfig::tesseract(executable, model_dir, &args.language)
        .map_err(|error| error.to_string())?;
    config.engine_binary = engine_binary;
    config.engine = engine;
    if !engine.cleared_for_production() {
        eprintln!(
            "warning: OCR engine `{}` is an evaluation candidate, not a production default; \
             see docs/ocr-engines.md",
            engine.as_str()
        );
        // An evaluation engine has no pinned model set to verify.
        config.required_files.clear();
    }
    Ok(OcrProviderChoice::Sidecar(config))
}

fn provider_mode(arg: OcrProviderModeArg) -> OcrProviderMode {
    match arg {
        OcrProviderModeArg::Local => OcrProviderMode::Local,
        OcrProviderModeArg::GeminiByok => OcrProviderMode::GeminiByok,
        OcrProviderModeArg::MpdfCredits => OcrProviderMode::MpdfCredits,
        OcrProviderModeArg::BrokerTest => OcrProviderMode::BrokerTest,
    }
}

/// Builds the cloud factory, or `None` for the default local run.
///
/// Every refusal here happens before the source PDF is opened, so a
/// misconfigured cloud run cannot get as far as touching a file, uploading a
/// page, or holding credits.
fn build_cloud_factory(args: &RunArgs) -> Result<Option<Box<dyn CloudProviderFactory>>, String> {
    let mode = provider_mode(args.ocr_provider);
    if mode == OcrProviderMode::GeminiByok {
        // Compatibility parse only. This happens before consent, endpoint,
        // credential labels, files, transports, or credential-store access.
        return Err(BYOK_DISABLED_MESSAGE.to_owned());
    }
    if mode != OcrProviderMode::BrokerTest && args.broker_test_config.is_some() {
        return Err("--broker-test-config requires --ocr-provider broker-test".into());
    }
    if mode == OcrProviderMode::Local {
        // Belt and braces: cloud-only flags on a local run are a usage error
        // rather than something silently ignored, because "I thought I was
        // using Gemini" is exactly the confusion that costs money elsewhere.
        if args.cloud_consent {
            return Err(
                "--cloud-consent was given but --ocr-provider is local; local OCR uploads nothing"
                    .to_owned(),
            );
        }
        return Ok(None);
    }
    if !args.cloud_consent {
        return Err(format!(
            "--ocr-provider {} uploads a rendered image of every OCR'd page to {}. \
             Re-run with --cloud-consent to authorize that.",
            mode.id(),
            mode.display_name()
        ));
    }
    match mode {
        OcrProviderMode::Local => unreachable!("handled above"),
        OcrProviderMode::GeminiByok => unreachable!("disabled above"),
        OcrProviderMode::BrokerTest => {
            if args.cloud_fallback != CloudFallbackArg::Fail
                || args.max_credits != 0
                || args.cloud_endpoint.is_some()
                || args.cloud_model.is_some()
                || args.cloud_model_version.is_some()
                || args.ocr_sidecar.is_some()
                || args.models.is_some()
                || args.provider != RunProviderArg::Tesseract
            {
                return Err("broker-test requires --cloud-fallback fail, no Credits ceiling, model/endpoint override or local OCR override; configure the frozen stack in --broker-test-config".into());
            }
            let path = args
                .broker_test_config
                .as_deref()
                .ok_or("broker-test requires --broker-test-config")?;
            let factory = mpdf_api_client::spatial_factory::SpatialFactory::from_config_file(path)
                .map_err(|e| e.to_string())?;
            Ok(Some(Box::new(factory)))
        }
        OcrProviderMode::MpdfCredits => {
            if args.max_credits == 0 {
                return Err(
                    "--max-credits must be set for --ocr-provider mpdf-credits; it is the hard cost ceiling the paid brokered run may reserve"
                        .to_owned(),
                );
            }
            Err(format!(
                "M PDF Cloud OCR is unavailable: no production backend currently returns the complete coordinate OCR contract. Outstanding blockers:\n  - {}",
                credits::release_blockers().join("\n  - ")
            ))
        }
    }
}

fn print_dry_run(args: &RunArgs, workspace: &Path) {
    let mode = provider_mode(args.ocr_provider);
    println!(
        "dry run — argument/policy checks only; the input and optional OCR plugin/runtime were not inspected."
    );
    println!("  side effects: nothing was opened, uploaded, reserved or charged");
    println!("  input:      {}", args.input.display());
    println!("  output:     {}", args.output.display());
    println!("  workspace:  {}", workspace.display());
    println!("  ocr mode:   {} ({})", mode.id(), mode.display_name());
    println!("  language:   {}", args.language);
    if mode.uses_network() {
        println!(
            "  uploads:    a {} dpi rendered image of every page routed to OCR",
            args.ocr_dpi
        );
        println!(
            "  model:      {}",
            args.cloud_model.as_deref().unwrap_or("gemini-3.7-flash")
        );
        println!(
            "  fallback:   {}",
            match args.cloud_fallback {
                CloudFallbackArg::Local => "local (per-page, recorded in the evidence)",
                CloudFallbackArg::Fail => "fail (the run stops and writes nothing)",
            }
        );
        if mode == OcrProviderMode::MpdfCredits {
            println!(
                "  credits:    up to {} per page, ceiling {}",
                args.credits_per_page, args.max_credits
            );
            println!("  NOT production ready:");
            for blocker in credits::release_blockers() {
                println!("    - {blocker}");
            }
        }
    } else {
        println!("  uploads:    nothing; local OCR makes no network request");
        println!(
            "  OCR plugin: optional and not inspected by dry-run; reliable native-text PDFs work without it, while scanned pages require it"
        );
    }
}

fn stage_number(stage: PipelineStage) -> usize {
    PipelineStage::ORDER
        .iter()
        .position(|item| *item == stage)
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn describe(stage: PipelineStage) -> &'static str {
    match stage {
        PipelineStage::AnalyzingSource => "reading the original PDF",
        PipelineStage::OcrOriginal => "recognizing text on the original pages",
        PipelineStage::DerivingText => "building the text layer",
        PipelineStage::GeneratingBookmarks => "compiling bookmarks from the evidence",
        PipelineStage::AwaitingReview => "waiting for review decisions",
        PipelineStage::BinarizingVisuals => "binarizing the visible pages",
        PipelineStage::AssemblingFinalPdf => "assembling the final PDF",
        PipelineStage::Validating => "verifying the finished file",
    }
}

fn usage(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitReason::UsageError.exit_code()
}

fn fail(error: &CoreError, json: bool, pretty: bool) -> ExitCode {
    let (_, reason) = errors::classify(error);
    if json {
        output::print_json(&errors::core_error_envelope(error, &[]), pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpdf_core::orchestrator::BookmarkOutcome;

    #[test]
    fn workspace_defaults_beside_the_output_and_is_named_after_it() {
        let workspace = default_workspace(Path::new("/books/out/final.pdf"));
        assert_eq!(workspace, PathBuf::from("/books/out/.final.mpdf-workspace"));
    }

    #[test]
    fn two_outputs_in_one_directory_do_not_share_a_workspace() {
        assert_ne!(
            default_workspace(Path::new("a.pdf")),
            default_workspace(Path::new("b.pdf"))
        );
    }

    #[test]
    fn every_stage_has_a_number_and_a_description() {
        for stage in PipelineStage::ORDER {
            assert!(stage_number(stage) >= 1);
            assert!(!describe(stage).is_empty());
        }
    }

    fn outcome(bookmark_status: BookmarkOutcome, halt: Option<Halt>) -> FinalPdfOutcome {
        let completed = halt.is_none();
        FinalPdfOutcome {
            stage_reached: if completed {
                PipelineStage::Validating
            } else {
                PipelineStage::AwaitingReview
            },
            halt,
            output_path: completed.then(|| PathBuf::from("/out/final.pdf")),
            output_sha256: completed.then(|| "b".repeat(64)),
            source_sha256: "a".repeat(64),
            page_count: 64,
            ocr_pages: 64,
            ocr_errors: 0,
            bookmarks_written: 0,
            bookmarks_auto_confirmed: 0,
            bookmarks_needing_review: 0,
            bookmarks_skipped: 3,
            bookmark_status,
            safe_refusal_reason: (bookmark_status == BookmarkOutcome::SafeRefusal)
                .then(|| "no printed contents list was found".to_owned()),
            text_layer_words: 12_000,
            binarization: None,
            verification: completed.then_some(OutputVerification {
                geometry_matches_source: true,
                outline_matches_confirmed: true,
                source_unchanged: true,
                transcription_spans_exact: true,
                pdfium_layout_text_exact: true,
            }),
            ocr_provenance: vec!["engine=tesseract".into()],
            provider: "sidecar".into(),
            provider_mode: "local".into(),
            cloud: Default::default(),
        }
    }

    #[test]
    fn a_bookmark_refusal_still_reports_a_completed_run_with_its_output() {
        // The product rule this locks: refusing to invent an outline is a
        // warning about the outline, not a failure of the conversion. The PDF
        // exists, `status` says completed, and the reason is still reported.
        let report = build_report(
            &outcome(BookmarkOutcome::SafeRefusal, None),
            "greek_german",
            Path::new("/out/.final.mpdf-workspace"),
            vec!["analyzing_source".into(), "validating".into()],
        );
        assert_eq!(report.status, "completed");
        assert_eq!(report.bookmark_status, "safe_refusal");
        assert_eq!(report.output_path.as_deref(), Some("/out/final.pdf"));
        assert_eq!(
            report.refusal_reason.as_deref(),
            Some("no printed contents list was found")
        );
        assert_eq!(report.bookmarks_written, 0);
        assert_eq!(report.bookmarks_skipped, 3);
        assert!(report.output_verification.is_some());
        assert_eq!(report.stages, vec!["analyzing_source", "validating"]);
    }

    #[test]
    fn a_refusal_exits_successfully_while_a_review_pause_is_still_reported() {
        assert_eq!(
            ExitReason::Success.exit_code(),
            std::process::ExitCode::SUCCESS
        );
        let paused = build_report(
            &outcome(
                BookmarkOutcome::NoneConfirmed,
                Some(Halt::AwaitingReview { needs_review: 4 }),
            ),
            "auto",
            Path::new("/out/.final.mpdf-workspace"),
            Vec::new(),
        );
        assert_eq!(paused.status, "awaiting_review");
        assert_eq!(paused.output_path, None);
        assert!(paused.output_verification.is_none());
    }
}
