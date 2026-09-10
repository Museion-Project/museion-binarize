//! Development entry: native_bookmarks SOURCE.pdf NEW_OUTPUT_DIRECTORY
//! Uses MPDF_PDFIUM_LIBRARY. No recognition or binarization is performed.
use mpdf_core::searchable_output::{build_searchable_output, SearchableOutputRequest};
use mpdf_core::{bookmarks, document_session::PdfOpenOptions};
use serde_json::json;
#[path = "native_bookmarks/intake.rs"]
mod intake;

use std::{fs, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: native_bookmarks SOURCE.pdf NEW_OUTPUT_DIRECTORY".into());
    }
    let source = PathBuf::from(&args[0]);
    let destination = PathBuf::from(&args[1]);
    // Refuse even an empty existing directory, preserving previous runs/reviews.
    fs::create_dir(&destination)?;
    let options = PdfOpenOptions::default();
    let start = Instant::now();
    let (inputs, diagnostics) = intake::load_native_bookmark_inputs(&source, &options, &|| false)?;
    let extraction_seconds = start.elapsed().as_secs_f64();
    let compilation_start = Instant::now();
    let result = bookmarks::generate_auto(
        &inputs.as_input(),
        &bookmarks::AutoBookmarkConfig::default(),
    )?;
    let compilation_seconds = compilation_start.elapsed().as_secs_f64();
    let root = destination.join("book.mdp");
    inputs.package.write_to(&root)?;
    if let Some(ocr) = &inputs.ocr {
        mpdf_core::ocr::write_ocr_records(&root, ocr)?;
    }
    bookmarks::save_generation(&root, &result, false)?;
    let output = destination.join("bookmarked.pdf");
    let write_start = Instant::now();
    let mut write_error = None;
    let written = if result.auto_confirmed() > 0 {
        match build_searchable_output(&SearchableOutputRequest {
            package: &inputs.package,
            source: &source,
            output: &output,
            overwrite: false,
            candidates: &result.snapshot.candidates,
            // Native source text already exists. Never add an approximate overlay.
            derived: None,
            pdfium: options.pdfium,
            output_write_strategy: Default::default(),
        }) {
            Ok(summary) => Some(summary),
            Err(error) => {
                write_error = Some(error.to_string());
                None
            }
        }
    } else {
        None
    };
    let summary = json!({
        "schema": "mpdf-native-bookmark-probe/1",
        "status": if write_error.is_some() { "output_blocked" } else if written.is_some() { "written" } else { "needs_review_or_no_toc" },
        "write_error": write_error,
        "mode": result.report.mode.as_str(), "diagnostics": diagnostics,
        "auto_confirmed": result.report.auto_confirmed, "needs_review": result.report.needs_review,
        "skipped": result.report.skipped, "toc_pages": result.report.toc_pages,
        "source_sha256": inputs.package.source.content_sha256,
        "output": written.as_ref().map(|s| &s.output_path),
        "written_bookmarks": written.as_ref().map(|s| s.written_bookmarks).unwrap_or(0),
        "extraction_seconds": extraction_seconds, "compilation_seconds": compilation_seconds,
        "write_verify_seconds": write_start.elapsed().as_secs_f64(),
        "total_seconds": start.elapsed().as_secs_f64(),
        "network_calls": 0, "recognition_calls": 0,
        "limitations": ["approximate native geometry", "no scan recognition", "no desktop integration", "not a release build"]
    });
    let text = serde_json::to_string_pretty(&summary)?;
    fs::write(destination.join("summary.json"), &text)?;
    println!("{text}");
    if let Some(error) = write_error {
        return Err(error.into());
    }
    Ok(())
}
