use std::process::ExitCode;

use museion_binarize_core::error::CoreError;
use museion_binarize_core::estimation::{
    self, EstimationRangeMethod, SIZE_ESTIMATE_REPORT_SCHEMA, SIZE_ESTIMATE_REPORT_SCHEMA_VERSION,
};
use museion_binarize_core::pipeline::{self, EstimationOptions};
use museion_binarize_core::report::ReportEnvelope;

use crate::cli::EstimateArgs;
use crate::errors::{self, ExitReason};
use crate::output;
use crate::progress::StderrProgress;

pub fn run(args: EstimateArgs) -> ExitCode {
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

    let options = EstimationOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        samples: args.samples,
    };
    let progress = StderrProgress::new(args.output.quiet);

    let report = match pipeline::estimate_output_size(&args.input, &settings, &options, &progress) {
        Ok(report) => report,
        Err(e) => {
            return core_fail(&e, &args.input, args.output.json, args.output.pretty);
        }
    };

    let envelope = ReportEnvelope::new(
        SIZE_ESTIMATE_REPORT_SCHEMA,
        SIZE_ESTIMATE_REPORT_SCHEMA_VERSION,
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
        print_human(&args.input, &envelope.result);
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

fn print_human(input: &std::path::Path, report: &estimation::SizeEstimateReport) {
    println!("Input:           {}", input.display());
    println!("Pages:           {}", report.document_page_count);
    let sampled: Vec<String> = report
        .sampled_pages
        .iter()
        .map(|p| p.page_number.to_string())
        .collect();
    println!("Sampled pages:   {}", sampled.join(", "));
    println!();
    println!(
        "Estimated output:\n  central: {}\n  range:   {}-{}",
        format_bytes(report.estimated_output_bytes),
        format_bytes(report.estimated_lower_bytes),
        format_bytes(report.estimated_upper_bytes),
    );
    println!();
    let range_label = match report.range_method {
        EstimationRangeMethod::Quartiles => "25th/75th percentile of sampled bytes-per-pixel",
        EstimationRangeMethod::MinMax => {
            "observed min/max of sampled bytes-per-pixel (too few samples for a percentile)"
        }
    };
    println!(
        "Experimental estimate based on {} sampled page(s); range from {range_label}. Not a formal confidence interval — see docs/size-estimation.md.",
        report.sampled_pages.len()
    );
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_uses_the_expected_units() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(6_710_886), "6.4 MB");
        assert_eq!(format_bytes(7_730_941_133), "7.2 GB");
    }
}
