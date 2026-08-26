use std::process::ExitCode;

use mpdf_core::benchmark::dataset::LoadedDataset;
use mpdf_core::benchmark::profile::LoadedProfile;
use mpdf_core::benchmark::report::{
    BenchmarkReport, RunReport, BENCHMARK_REPORT_SCHEMA, BENCHMARK_REPORT_SCHEMA_VERSION,
};
use mpdf_core::benchmark::runner::run_raster_benchmark;
use mpdf_core::error::CoreError;
use mpdf_core::report::ReportEnvelope;

use crate::cli::{BenchmarkRunArgs, BenchmarkValidateArgs};
use crate::errors::{self, ExitReason};
use crate::output;

pub fn run(args: BenchmarkRunArgs) -> ExitCode {
    let mut aliases = vec![
        ("dataset", args.dataset.as_path()),
        ("profile", args.profile.as_path()),
    ];
    if let Some(report_path) = &args.report {
        aliases.push(("report", report_path.as_path()));
    }
    if let Err(message) = output::reject_path_aliases(&aliases) {
        return usage_fail(&message, args.output.json, args.output.pretty);
    }

    let dataset = match LoadedDataset::load(&args.dataset) {
        Ok(d) => d,
        Err(e) => return core_fail(&e, args.output.json, args.output.pretty),
    };
    let profile = match LoadedProfile::load(&args.profile) {
        Ok(p) => p,
        Err(e) => return core_fail(&e, args.output.json, args.output.pretty),
    };

    let report = match run_raster_benchmark(&dataset, &profile) {
        Ok(r) => r,
        Err(e) => return core_fail(&e, args.output.json, args.output.pretty),
    };

    let envelope = ReportEnvelope::new(
        BENCHMARK_REPORT_SCHEMA,
        BENCHMARK_REPORT_SCHEMA_VERSION,
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

pub fn validate(args: BenchmarkValidateArgs) -> ExitCode {
    let dataset = match LoadedDataset::load(&args.dataset) {
        Ok(d) => d,
        Err(e) => return core_fail(&e, args.output.json, args.output.pretty),
    };
    let profile = match &args.profile {
        Some(path) => match LoadedProfile::load(path) {
            Ok(p) => Some(p),
            Err(e) => return core_fail(&e, args.output.json, args.output.pretty),
        },
        None => None,
    };

    // Digest computation over every referenced file doubles as "every
    // file exists and is readable," on top of the schema/dimension/ROI
    // checks `LoadedDataset::load` already performed.
    if let Err(e) = dataset.page_file_digests() {
        return core_fail(&e, args.output.json, args.output.pretty);
    }

    if args.output.json {
        let summary = serde_json::json!({
            "dataset_valid": true,
            "dataset_id": dataset.manifest.id,
            "page_count": dataset.manifest.pages.len(),
            "profile_valid": profile.is_some(),
            "profile_id": profile.as_ref().map(|p| p.manifest.id.clone()),
            "run_count": profile.as_ref().map(|p| p.manifest.runs.len()),
        });
        output::print_json(&summary, args.output.pretty);
    } else if !args.output.quiet {
        println!(
            "Dataset '{}' is valid: {} pages.",
            dataset.manifest.id,
            dataset.manifest.pages.len()
        );
        if let Some(profile) = &profile {
            println!(
                "Profile '{}' is valid: {} runs.",
                profile.manifest.id,
                profile.manifest.runs.len()
            );
        }
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

fn core_fail(error: &CoreError, json: bool, pretty: bool) -> ExitCode {
    let (_, reason) = errors::classify(error);
    if json {
        output::print_json(&errors::core_error_envelope(error, &[]), pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}

fn print_human(report: &BenchmarkReport) {
    println!(
        "Dataset:  {} ({} pages)",
        report.dataset.id, report.dataset.page_count
    );
    println!("Profile:  {}", report.profile.id);
    println!(
        "Level:    {}",
        match report.environment.benchmark_level {
            mpdf_core::benchmark::report::BenchmarkLevel::Raster => "raster",
            mpdf_core::benchmark::report::BenchmarkLevel::Pdf => "pdf",
        }
    );
    println!();
    println!(
        "{:<22} {:>9} {:>9} {:>9} {:>10} {:>9}",
        "Run", "Macro F1", "Micro F1", "Mean DRD", "Bytes/MP", "Time (ms)"
    );
    for run in &report.runs {
        let total_ms = run
            .aggregate
            .total_processing_duration_us
            .map(|us| us as f64 / 1000.0)
            .unwrap_or(0.0);
        println!(
            "{:<22} {:>9.4} {:>9.4} {:>9.4} {:>10.1} {:>9.1}",
            run.run_id,
            run.aggregate.macro_f1,
            run.aggregate.micro_f1,
            run.aggregate.mean_drd,
            run.aggregate.mean_bytes_per_megapixel.unwrap_or(0.0),
            total_ms,
        );
    }
    println!();
    for run in &report.runs {
        print_worst_page(run);
    }
}

fn print_worst_page(run: &RunReport) {
    if let Some(worst) = &run.worst_pages.lowest_f1 {
        println!(
            "{}: lowest F1 page is '{}' ({:.4})",
            run.run_id, worst.page_id, worst.value
        );
    }
}
