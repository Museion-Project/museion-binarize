use std::process::ExitCode;

use mpdf_core::document_session::PdfOpenOptions;
use mpdf_core::error::CoreError;
use mpdf_core::pipeline;
use mpdf_core::report::ReportEnvelope;
use serde::Serialize;

use crate::cli::PreviewArgs;
use crate::errors::{self, ExitReason};
use crate::output;

pub const PREVIEW_REPORT_SCHEMA: &str = "mpdf-preview";
pub const PREVIEW_REPORT_SCHEMA_VERSION: &str = "1.0";

#[derive(Serialize)]
struct PreviewReport {
    output_path: String,
    page_number: u32,
    pixel_width: u32,
    pixel_height: u32,
}

pub fn run(args: PreviewArgs) -> ExitCode {
    let settings = match args.settings.to_settings() {
        Ok(s) => s,
        Err(message) => {
            return usage_fail(&message, args.output_mode.json, args.output_mode.pretty)
        }
    };
    if args.page == 0 {
        return usage_fail(
            "page numbers are one-based; --page 0 does not exist",
            args.output_mode.json,
            args.output_mode.pretty,
        );
    }
    if let Err(message) = output::reject_path_aliases(&[
        ("input", args.input.as_path()),
        ("output", args.output.as_path()),
    ]) {
        return usage_fail(&message, args.output_mode.json, args.output_mode.pretty);
    }
    if args.output.exists() && !args.overwrite {
        return usage_fail(
            &format!(
                "{} already exists; pass --overwrite to replace it",
                args.output.display()
            ),
            args.output_mode.json,
            args.output_mode.pretty,
        );
    }

    let options = PdfOpenOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        compute_source_hash: false,
    };
    let preview = match pipeline::preview_page(&args.input, args.page, &settings, &options) {
        Ok(preview) => preview,
        Err(e) => {
            return core_fail(
                &e,
                &args.input,
                args.output_mode.json,
                args.output_mode.pretty,
            )
        }
    };

    if let Err(e) = preview.save(&args.output) {
        let message = format!(
            "could not write the preview to {}: {e}",
            args.output.display()
        );
        return usage_fail(&message, args.output_mode.json, args.output_mode.pretty);
    }

    let envelope = ReportEnvelope::new(
        PREVIEW_REPORT_SCHEMA,
        PREVIEW_REPORT_SCHEMA_VERSION,
        PreviewReport {
            output_path: args.output.display().to_string(),
            page_number: args.page,
            pixel_width: preview.width(),
            pixel_height: preview.height(),
        },
    );

    if args.output_mode.json {
        output::print_json(&envelope, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        println!("Preview:     {}", envelope.result.output_path);
        println!("Page:        {}", envelope.result.page_number);
        println!(
            "Size:        {} x {} px",
            envelope.result.pixel_width, envelope.result.pixel_height
        );
    }
    ExitReason::Success.exit_code()
}

fn usage_fail(message: &str, json: bool, pretty: bool) -> ExitCode {
    if json {
        output::print_json(&errors::usage_error_envelope(message, &[]), pretty);
    } else {
        eprintln!("error: {message}");
    }
    ExitReason::UsageError.exit_code()
}

fn core_fail(error: &CoreError, input: &std::path::Path, json: bool, pretty: bool) -> ExitCode {
    let (_, reason) = errors::classify(error);
    if json {
        let envelope =
            errors::core_error_envelope(error, &[("input", input.display().to_string())]);
        output::print_json(&envelope, pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}
