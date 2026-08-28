//! Searchable-PDF derivative output.
//!
//! The source-boundary checks, temporary write, reopen verification, and
//! outline verification all live in `mpdf_core::searchable_output`, shared
//! with `bookmark auto` and the desktop application. This command only maps
//! arguments and prints the result.
use crate::cli::PdfBuildSearchableArgs;
use crate::errors::{self, ExitReason};
use crate::output;
use mpdf_core::bookmarks;
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::error::CoreError;
use mpdf_core::searchable_output::{build_searchable_output, SearchableOutputRequest};
use std::process::ExitCode;

pub fn build_searchable(a: PdfBuildSearchableArgs) -> ExitCode {
    let package = match DocumentPackage::read_from(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let snapshot = match bookmarks::load_snapshot(&a.input) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let reviews = match bookmarks::load_reviews(&a.input, &snapshot) {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let effective = match bookmarks::effective(&snapshot, &reviews) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let derived = match bookmarks::load_auto_bookmark_inputs(&a.input) {
        Ok(inputs) => inputs.derived,
        Err(e) => return fail(&e),
    };
    let summary = match build_searchable_output(&SearchableOutputRequest {
        package: &package,
        source: &a.source,
        output: &a.output,
        overwrite: a.overwrite,
        candidates: &effective,
        derived: derived.as_ref(),
        pdfium: a.pdfium.to_config(),
    }) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if a.output_mode.json {
        output::print_json(
            &serde_json::json!({
                "schema": "mpdf-searchable-pdf",
                "schema_version": "0.1",
                "source_sha256": summary.source_sha256,
                "output_path": summary.output_path,
                "confirmed_bookmarks": summary.human_confirmed_bookmarks,
                "auto_confirmed_bookmarks": summary.auto_confirmed_bookmarks,
                "written_bookmarks": summary.written_bookmarks,
            }),
            a.output_mode.pretty,
        )
    } else if !a.output_mode.quiet {
        println!("built searchable PDF: {}", summary.output_path.display());
    }
    ExitReason::Success.exit_code()
}

fn fail(e: &CoreError) -> ExitCode {
    eprintln!("error: {}", e);
    errors::classify(e).1.exit_code()
}
