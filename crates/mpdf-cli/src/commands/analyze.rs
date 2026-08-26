use std::process::ExitCode;

use mpdf_core::analysis::{self, DocumentAnalysisReport};
use mpdf_core::error::CoreError;
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::pipeline::{self, AnalysisOptions};
use mpdf_core::report::ReportEnvelope;

use crate::cli::AnalyzeArgs;
use crate::errors::{self, ExitReason};
use crate::output;
use crate::progress::StderrProgress;

pub fn run(args: AnalyzeArgs) -> ExitCode {
    let settings = match args.settings.to_settings() {
        Ok(s) => s,
        Err(message) => return usage_fail(&message, args.output.json, args.output.pretty),
    };

    if let Some(report_path) = &args.report {
        if let Err(message) =
            output::reject_path_aliases(&[("input", &args.input), ("report", report_path)])
        {
            return usage_fail(&message, args.output.json, args.output.pretty);
        }
    }

    let options = AnalysisOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        pages: Some(args.pages.clone()),
        encode: args.encode,
        path_mode: args.path_mode.into(),
    };
    let progress = StderrProgress::new(args.output.quiet);

    let report = match pipeline::analyze_pdf(&args.input, &settings, &options, &progress) {
        Ok(report) => report,
        Err(e) => {
            return core_fail(
                &e,
                &args.input,
                &options.pdfium,
                args.output.json,
                args.output.pretty,
            )
        }
    };

    let envelope = ReportEnvelope::new(
        analysis::ANALYSIS_REPORT_SCHEMA,
        analysis::ANALYSIS_REPORT_SCHEMA_VERSION,
        report,
    );

    if let Some(report_path) = &args.report {
        if let Err(message) =
            output::write_report_atomic(&envelope, report_path, args.output.pretty, args.overwrite)
        {
            return usage_fail(&message, args.output.json, args.output.pretty);
        }
    }

    if args.output.json {
        output::print_json(&envelope, args.output.pretty);
    } else if !args.output.quiet {
        print_human(&envelope.result);
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
    _pdfium: &PdfiumConfig,
    json: bool,
    pretty: bool,
) -> ExitCode {
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

fn print_human(report: &DocumentAnalysisReport) {
    if let Some(path) = &report.source_path {
        println!("File:            {path}");
    }
    println!(
        "Pages:           {} (analyzed {})",
        report.page_count, report.analyzed_page_count
    );
    if report.failed_page_count > 0 {
        println!("Failed pages:    {}", report.failed_page_count);
    }
    println!("DPI:             {}", report.dpi);
    println!("Method:          {}", report.method);
    println!("PDFium:          {}", report.pdfium_library);
    println!(
        "Total duration:  {:.2}s",
        report.total_duration_us as f64 / 1_000_000.0
    );
    if let Some(stats) = &report.page_duration {
        println!(
            "Page duration:   min {:.1}ms, mean {:.1}ms, median {:.1}ms, max {:.1}ms",
            stats.min_us as f64 / 1_000.0,
            stats.mean_us / 1_000.0,
            stats.median_us / 1_000.0,
            stats.max_us as f64 / 1_000.0
        );
    }
    println!();
    println!(
        "{:>5} {:>10} {:>10} {:>7} {:>8} {:>10}",
        "page", "threshold", "black %", "min", "max", "ccitt B"
    );
    for page in &report.pages {
        let threshold_display = match &page.threshold {
            mpdf_core::image_pipeline::ThresholdInfo::Otsu { threshold } => threshold.to_string(),
            mpdf_core::image_pipeline::ThresholdInfo::Manual { threshold } => threshold.to_string(),
            mpdf_core::image_pipeline::ThresholdInfo::Sauvola { .. } => "adaptive".to_string(),
        };
        println!(
            "{:>5} {:>10} {:>9.1}% {:>7} {:>8} {:>10}",
            page.page_number,
            threshold_display,
            page.ink.black_pixel_ratio * 100.0,
            page.grayscale.min,
            page.grayscale.max,
            page.ccitt_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "-".to_string()),
        );
    }
}
