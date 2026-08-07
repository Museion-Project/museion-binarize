use std::process::ExitCode;

use museion_binarize_core::error::CoreError;
use museion_binarize_core::pipeline::{self, PdfProcessingOptions, ProcessingReport};
use museion_binarize_core::report::ReportEnvelope;

use crate::cli::ProcessArgs;
use crate::errors::{self, ExitReason};
use crate::output;
use crate::progress::StderrProgress;

pub fn run(args: ProcessArgs) -> ExitCode {
    let settings = match args.settings.to_settings() {
        Ok(s) => s,
        Err(message) => {
            return usage_fail(&message, args.output_mode.json, args.output_mode.pretty)
        }
    };

    // Every pair of paths this command touches must be distinct: the
    // core already refuses input == output, but a --report aliasing
    // either one is a CLI-only concern (the core has never heard of a
    // report path) and would otherwise silently overwrite the source or
    // the just-written PDF with a JSON report.
    let mut named = vec![
        ("input", args.input.as_path()),
        ("output", args.output.as_path()),
    ];
    if let Some(report_path) = &args.report {
        named.push(("report", report_path.as_path()));
    }
    if let Err(message) = output::reject_path_aliases(&named) {
        return usage_fail(&message, args.output_mode.json, args.output_mode.pretty);
    }

    let options = PdfProcessingOptions {
        password: super::password_from_env(),
        overwrite: args.overwrite,
        validation: args.validate.into(),
        pdfium: args.pdfium.to_config(),
    };
    let progress = StderrProgress::new(args.output_mode.quiet);

    let report =
        match pipeline::process_pdf(&args.input, &args.output, &settings, &options, &progress) {
            Ok(report) => report,
            Err(e) => {
                return core_fail(
                    &e,
                    &args.input,
                    &args.output,
                    args.output_mode.json,
                    args.output_mode.pretty,
                )
            }
        };

    let envelope = ReportEnvelope::new(
        pipeline::PROCESS_REPORT_SCHEMA,
        pipeline::PROCESS_REPORT_SCHEMA_VERSION,
        report,
    );

    if let Some(report_path) = &args.report {
        if let Err(message) = output::write_report_atomic(
            &envelope,
            report_path,
            args.output_mode.pretty,
            args.overwrite,
        ) {
            return usage_fail(&message, args.output_mode.json, args.output_mode.pretty);
        }
    }

    if args.output_mode.json {
        output::print_json(&envelope, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        print_human(&args.output, &envelope.result);
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

fn core_fail(
    error: &CoreError,
    input: &std::path::Path,
    output: &std::path::Path,
    json: bool,
    pretty: bool,
) -> ExitCode {
    let (_, reason) = errors::classify(error);
    if json {
        let envelope = errors::core_error_envelope(
            error,
            &[
                ("input", input.display().to_string()),
                ("output", output.display().to_string()),
            ],
        );
        output::print_json(&envelope, pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}

fn print_human(output_path: &std::path::Path, report: &ProcessingReport) {
    println!("Output:      {}", output_path.display());
    println!("Pages:       {}", report.pages_processed);
    println!("Original:    {} bytes", report.original_bytes);
    println!("Output size: {} bytes", report.output_bytes);
    if let Some(reduction) = report.reduction_percent() {
        println!("Reduction:   {reduction:.1}%");
    }
    if let Some(average) = report.average_bytes_per_page() {
        println!("Per page:    {average:.0} bytes");
    }
    println!(
        "Elapsed:     {:.2}s",
        report.elapsed_us as f64 / 1_000_000.0
    );
    println!("PDFium:      {}", report.pdfium_library);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{DespeckleArg, MethodArg, OutputArgs, PdfiumArgs, SettingsArgs, ValidateArg};

    fn base_settings_args() -> SettingsArgs {
        SettingsArgs {
            dpi: 300,
            method: MethodArg::Otsu,
            threshold: None,
            sauvola_k: None,
            sauvola_window: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: DespeckleArg::Off,
        }
    }

    fn no_pdfium() -> PdfiumArgs {
        PdfiumArgs {
            pdfium_library: None,
            allow_system_pdfium: false,
        }
    }

    /// Regression test for a real bug caught during manual Milestone 3
    /// verification: `--report` pointing at the *input* PDF was never
    /// checked, so a successful conversion would silently overwrite the
    /// source file with the JSON report afterwards. `run` must reject
    /// this before ever opening a PDFium session, so this test needs no
    /// provisioned library.
    #[test]
    fn report_path_aliasing_the_input_is_rejected_before_any_processing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("book.pdf");
        std::fs::write(&input, b"%PDF-1.7 not a real document").unwrap();

        let args = ProcessArgs {
            input: input.clone(),
            output: dir.path().join("out.pdf"),
            settings: base_settings_args(),
            overwrite: true,
            validate: ValidateArg::Structural,
            output_mode: OutputArgs::default(),
            report: Some(input.clone()),
            pdfium: no_pdfium(),
        };

        let exit = run(args);
        assert_eq!(exit, ExitReason::UsageError.exit_code());
        // The decisive assertion: the input file must be byte-for-byte
        // untouched, not overwritten with a JSON report.
        assert_eq!(
            std::fs::read(&input).unwrap(),
            b"%PDF-1.7 not a real document"
        );
    }

    #[test]
    fn report_path_aliasing_the_output_is_also_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("book.pdf");
        std::fs::write(&input, b"input").unwrap();
        let output = dir.path().join("out.pdf");

        let args = ProcessArgs {
            input,
            output: output.clone(),
            settings: base_settings_args(),
            overwrite: true,
            validate: ValidateArg::Structural,
            output_mode: OutputArgs::default(),
            report: Some(output),
            pdfium: no_pdfium(),
        };

        assert_eq!(run(args), ExitReason::UsageError.exit_code());
    }

    /// Regression test for a data-loss bug found during independent review
    /// of Milestone 3: `--output` names a file that does not exist yet, so
    /// the earlier alias check (`std::fs::canonicalize`-or-raw-string-
    /// compare) missed a `--report` given as a *different spelling* of
    /// that same path (here, a relative path vs its absolute form). This
    /// must be rejected before PDFium is ever touched — no real PDFium
    /// library is available in this ordinary test, so a rejection this
    /// early is exactly what proves the fix does not depend on processing
    /// having happened first.
    #[test]
    fn report_path_aliasing_a_relative_vs_absolute_spelling_of_the_output_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("book.pdf");
        std::fs::write(&input, b"input").unwrap();
        let output = dir.path().join("out.pdf");
        assert!(!output.exists(), "the destination must not exist yet");
        let absolute_output = std::fs::canonicalize(dir.path()).unwrap().join("out.pdf");

        let args = ProcessArgs {
            input,
            output,
            settings: base_settings_args(),
            overwrite: true,
            validate: ValidateArg::Structural,
            output_mode: OutputArgs::default(),
            report: Some(absolute_output),
            pdfium: no_pdfium(),
        };

        assert_eq!(run(args), ExitReason::UsageError.exit_code());
    }
}
