//! Bookmark generation, one-command automatic compilation, and review.
//!
//! Every command here goes through the same core engine and the same safe
//! output boundary; the CLI adds argument handling and reporting only.

use crate::cli::{
    BookmarkAutoArgs, BookmarkEditArgs, BookmarkGenerateArgs, BookmarkListArgs,
    BookmarkMutationArgs, BookmarkReparentArgs,
};
use crate::errors::{self, ExitReason};
use crate::output;
use mpdf_core::bookmarks::{
    self, AutoBookmarkConfig, BookmarkStatus, GenerationStatus, ReviewAction,
};
use mpdf_core::error::CoreError;
use mpdf_core::searchable_output::{
    build_searchable_output, SearchableOutputRequest, SearchableOutputSummary,
};
use std::path::Path;
use std::process::ExitCode;

/// Refuses to replace candidates that already carry human review decisions.
/// The reviews are never deleted or migrated by guesswork.
fn guard_existing_reviews(root: &Path, regenerate: bool) -> Result<bool, CoreError> {
    let exists = bookmarks::candidates_path(root).exists();
    if !exists {
        return Ok(false);
    }
    if !regenerate {
        return Err(CoreError::DestinationConflict(
            "bookmark candidates already exist; pass --regenerate to replace them".into(),
        ));
    }
    let snapshot = bookmarks::load_snapshot(root)?;
    let reviews = bookmarks::load_reviews(root, &snapshot)?;
    if !reviews.operations.is_empty() {
        return Err(CoreError::DestinationConflict(format!(
            "{} human review operation(s) exist for the current generation; \
             copy or remove {} before regenerating",
            reviews.operations.len(),
            bookmarks::reviews_path(root).display()
        )));
    }
    Ok(true)
}

pub fn generate(a: BookmarkGenerateArgs) -> ExitCode {
    let overwrite = match guard_existing_reviews(&a.input, a.overwrite) {
        Ok(value) => value,
        Err(e) => return fail(&e),
    };
    let result = match bookmarks::generate_auto_from_package(
        &a.input,
        &AutoBookmarkConfig::default(),
        &|| false,
    ) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = bookmarks::save_generation(&a.input, &result, overwrite) {
        return fail(&e);
    }
    if a.output_mode.json {
        output::print_json(&result.snapshot, a.output_mode.pretty)
    } else if !a.output_mode.quiet {
        println!(
            "generated {} bookmark candidate(s): {} automatic, {} to review, {} skipped ({})",
            result.snapshot.candidates.len(),
            result.report.auto_confirmed,
            result.report.needs_review,
            result.report.skipped,
            result.report.mode.as_str()
        );
    }
    ExitReason::Success.exit_code()
}

/// `mpdf bookmark auto`: the one-command path from an MDP package to a
/// verified, outlined PDF — or to an explained refusal that writes nothing.
pub fn auto(a: BookmarkAutoArgs) -> ExitCode {
    let overwrite_candidates = match guard_existing_reviews(&a.input, a.regenerate) {
        Ok(value) => value,
        Err(e) => return fail(&e),
    };
    let result = match bookmarks::generate_auto_from_package(
        &a.input,
        &AutoBookmarkConfig::default(),
        &|| false,
    ) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = bookmarks::save_generation(&a.input, &result, overwrite_candidates) {
        return fail(&e);
    }
    let package = match mpdf_core::document_package::DocumentPackage::read_from(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let reviews = match bookmarks::load_reviews(&a.input, &result.snapshot) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let effective = match bookmarks::effective(&result.snapshot, &reviews) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let writable = effective
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .count();
    let derived = if result.snapshot.derived_digest.is_some() {
        match bookmarks::load_auto_bookmark_inputs(&a.input) {
            Ok(inputs) => inputs.derived,
            Err(e) => return fail(&e),
        }
    } else {
        None
    };
    let summary: Option<SearchableOutputSummary> = if writable == 0 {
        None
    } else {
        match build_searchable_output(&SearchableOutputRequest {
            package: &package,
            source: &a.source,
            output: &a.output,
            overwrite: a.overwrite,
            candidates: &effective,
            derived: derived.as_ref(),
            pdfium: a.pdfium.to_config(),
        }) {
            Ok(x) => Some(x),
            Err(e) => return fail(&e),
        }
    };
    let status = if summary.is_some() {
        "written"
    } else {
        match result.report.status {
            GenerationStatus::SafeRefusal => "safe_refusal",
            _ => "needs_review",
        }
    };
    if a.output_mode.json {
        output::print_json(
            &serde_json::json!({
                "schema": "mpdf-bookmark-auto",
                "schema_version": "0.1",
                "status": status,
                "mode": result.report.mode.as_str(),
                "generation_status": result.report.status.as_str(),
                "safe_refusal_reason": result.report.safe_refusal_reason,
                "auto_confirmed": result.report.auto_confirmed,
                "needs_review": result.report.needs_review,
                "skipped": result.report.skipped,
                "written_bookmarks": summary.as_ref().map(|s| s.written_bookmarks).unwrap_or(0),
                "toc_pages": result.report.toc_pages.len(),
                "report_path": bookmarks::generation_report_path(&a.input),
                "candidates_path": bookmarks::candidates_path(&a.input),
                "output_path": summary.as_ref().map(|s| s.output_path.clone()),
                "source_sha256": result.snapshot.source_digest,
                "generation_digest": result.snapshot.generation_digest,
            }),
            a.output_mode.pretty,
        )
    } else if !a.output_mode.quiet {
        match &summary {
            Some(summary) => println!(
                "Added {} reliable bookmark(s) automatically; {} need review and {} were skipped for \
                 insufficient evidence. Output verified: {}",
                summary.written_bookmarks,
                result.report.needs_review,
                result.report.skipped,
                summary.output_path.display()
            ),
            None => println!(
                "No sufficiently reliable table of contents was found; no guessed bookmarks were \
                 written to a PDF. Reason: {}",
                result
                    .report
                    .safe_refusal_reason
                    .clone()
                    .unwrap_or_else(|| "no entry reached the confidence gate".to_owned())
            ),
        }
    }
    ExitReason::Success.exit_code()
}

pub fn list(a: BookmarkListArgs) -> ExitCode {
    let s = match bookmarks::load_snapshot(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let r = match bookmarks::load_reviews(&a.input, &s) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let e = match bookmarks::effective(&s, &r) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if a.output_mode.json {
        output::print_json(&e, a.output_mode.pretty)
    } else if !a.output_mode.quiet {
        for c in e {
            println!(
                "{}\t{}\t{}\tpage {}",
                c.candidate_id,
                status_label(c.status),
                c.effective_title,
                c.physical_page_index + 1
            )
        }
    }
    ExitReason::Success.exit_code()
}

/// User-facing status names. `auto_confirmed` and `confirmed` are always
/// distinguishable: one is a rule decision, the other a person's.
fn status_label(status: BookmarkStatus) -> &'static str {
    match status {
        BookmarkStatus::Proposed => "proposed",
        BookmarkStatus::NeedsReview => "needs_review",
        BookmarkStatus::Confirmed => "confirmed",
        BookmarkStatus::Rejected => "rejected",
        BookmarkStatus::AutoConfirmed => "auto_confirmed",
        BookmarkStatus::Skipped => "skipped",
    }
}

pub fn confirm(a: BookmarkMutationArgs) -> ExitCode {
    mutate(a.input, a.candidate, ReviewAction::Confirm, a.output_mode)
}
pub fn reject(a: BookmarkMutationArgs) -> ExitCode {
    mutate(a.input, a.candidate, ReviewAction::Reject, a.output_mode)
}
pub fn edit(a: BookmarkEditArgs) -> ExitCode {
    mutate(
        a.input,
        a.candidate,
        ReviewAction::Edit { title: a.title },
        a.output_mode,
    )
}
pub fn reparent(a: BookmarkReparentArgs) -> ExitCode {
    mutate(
        a.input,
        a.candidate,
        ReviewAction::Reparent {
            parent_id: a.parent,
            level: a.level,
        },
        a.output_mode,
    )
}
fn mutate(
    root: std::path::PathBuf,
    id: String,
    action: ReviewAction,
    o: crate::cli::OutputArgs,
) -> ExitCode {
    let s = match bookmarks::load_snapshot(&root) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let mut r = match bookmarks::load_reviews(&root, &s) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = bookmarks::append(&s, &mut r, id, action) {
        return fail(&e);
    };
    if let Err(e) = bookmarks::save_reviews(&root, &r) {
        return fail(&e);
    };
    if o.json {
        output::print_json(&r, o.pretty)
    } else if !o.quiet {
        println!("bookmark review appended")
    }
    ExitReason::Success.exit_code()
}

fn fail(e: &CoreError) -> ExitCode {
    eprintln!("error: {}", e);
    errors::classify(e).1.exit_code()
}
