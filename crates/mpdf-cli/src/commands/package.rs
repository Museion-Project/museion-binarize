//! MDP package command wiring.

use std::process::ExitCode;

use mpdf_core::document_package::{self, DocumentPackage};
use mpdf_core::document_session::PdfOpenOptions;

use crate::cli::{PackageCreateArgs, PackageValidateArgs};
use crate::errors::{self, ExitReason};
use crate::output;

pub fn create(args: PackageCreateArgs) -> ExitCode {
    let options = PdfOpenOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        compute_source_hash: true,
    };
    let package = match DocumentPackage::create_from_pdf(&args.input, &options) {
        Ok(package) => package,
        Err(error) => {
            return fail_with_reason(
                &error,
                &args.input,
                args.output_mode.json,
                args.output_mode.pretty,
                ExitReason::InputError,
            )
        }
    };
    if let Err(error) = package.write_to(&args.output) {
        return fail(
            &error,
            &args.output,
            args.output_mode.json,
            args.output_mode.pretty,
        );
    }
    let summary = match package.validation_report() {
        Ok(summary) => summary,
        Err(error) => {
            return fail_with_reason(
                &error,
                &args.output,
                args.output_mode.json,
                args.output_mode.pretty,
                ExitReason::OutputError,
            )
        }
    };
    if args.output_mode.json {
        output::print_json(&summary, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        println!(
            "Created MDP package: {} ({} pages)",
            args.output.display(),
            summary.checked_pages
        );
    }
    ExitReason::Success.exit_code()
}

pub fn validate(args: PackageValidateArgs) -> ExitCode {
    let summary = match document_package::validate_directory(&args.input) {
        Ok(summary) => summary,
        Err(error) => {
            return fail_with_reason(
                &error,
                &args.input,
                args.output_mode.json,
                args.output_mode.pretty,
                ExitReason::InputError,
            )
        }
    };
    if args.output_mode.json {
        output::print_json(&summary, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        println!(
            "Valid MDP package: {} ({} pages, {} assets)",
            args.input.display(),
            summary.checked_pages,
            summary.checked_assets
        );
    }
    ExitReason::Success.exit_code()
}

fn fail(
    error: &mpdf_core::error::CoreError,
    path: &std::path::Path,
    json: bool,
    pretty: bool,
) -> ExitCode {
    let (_, reason) = errors::classify(error);
    fail_with_reason(error, path, json, pretty, reason)
}

fn fail_with_reason(
    error: &mpdf_core::error::CoreError,
    path: &std::path::Path,
    json: bool,
    pretty: bool,
    reason: ExitReason,
) -> ExitCode {
    if json {
        let envelope = errors::core_error_envelope(error, &[("path", path.display().to_string())]);
        output::print_json(&envelope, pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}
