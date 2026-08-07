use std::process::ExitCode;

use museion_binarize_core::error::CoreError;
use museion_binarize_core::report::ReportEnvelope;
use museion_binarize_core::settings::SUPPORTED_DPI;
use museion_binarize_core::{analysis, pipeline, ProjectInfo};
use serde::Serialize;

use crate::cli::InfoArgs;
use crate::errors::{self, ExitReason};
use crate::output;

pub const INFO_REPORT_SCHEMA: &str = "museion-binarize-info";
pub const INFO_REPORT_SCHEMA_VERSION: &str = "1.0";

#[derive(Serialize)]
struct InfoReport {
    name: String,
    phase: String,
    version: String,
    build_profile: &'static str,
    target_arch: &'static str,
    target_os: &'static str,
    supported_dpi: Vec<u16>,
    report_schemas: Vec<SchemaVersion>,
    pdfium: PdfiumInfo,
    limitations: Vec<&'static str>,
}

#[derive(Serialize)]
struct SchemaVersion {
    schema: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct PdfiumInfo {
    /// Whether `--probe-pdfium` was requested. `info` never touches
    /// PDFium otherwise.
    probed: bool,
    resolved: Option<String>,
    error: Option<String>,
}

/// Prints project information. Never initializes PDFium unless
/// `--probe-pdfium` is passed; a failed probe exits non-zero even though
/// the rest of the report is still printed.
pub fn run(args: InfoArgs) -> ExitCode {
    let mut probe_error: Option<CoreError> = None;
    let pdfium = if args.probe_pdfium {
        match pipeline::describe_pdfium_library(&args.pdfium.to_config()) {
            Ok(description) => PdfiumInfo {
                probed: true,
                resolved: Some(description),
                error: None,
            },
            Err(e) => {
                let info = PdfiumInfo {
                    probed: true,
                    resolved: None,
                    error: Some(e.to_string()),
                };
                probe_error = Some(e);
                info
            }
        }
    } else {
        PdfiumInfo {
            probed: false,
            resolved: None,
            error: None,
        }
    };

    let report = build_report(pdfium);

    if args.output.json {
        output::print_json(&report, args.output.pretty);
    } else if !args.output.quiet {
        print_human(&report.result);
    }

    match probe_error {
        Some(e) => errors::classify(&e).1.exit_code(),
        None => ExitReason::Success.exit_code(),
    }
}

fn build_report(pdfium: PdfiumInfo) -> ReportEnvelope<InfoReport> {
    let project = ProjectInfo::current();
    let result = InfoReport {
        name: project.name.to_string(),
        phase: project.phase.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        target_arch: std::env::consts::ARCH,
        target_os: std::env::consts::OS,
        supported_dpi: SUPPORTED_DPI.to_vec(),
        report_schemas: vec![
            SchemaVersion {
                schema: INFO_REPORT_SCHEMA,
                version: INFO_REPORT_SCHEMA_VERSION,
            },
            SchemaVersion {
                schema: analysis::ANALYSIS_REPORT_SCHEMA,
                version: analysis::ANALYSIS_REPORT_SCHEMA_VERSION,
            },
            SchemaVersion {
                schema: pipeline::PROCESS_REPORT_SCHEMA,
                version: pipeline::PROCESS_REPORT_SCHEMA_VERSION,
            },
            SchemaVersion {
                schema: museion_binarize_core::report::ERROR_SCHEMA,
                version: museion_binarize_core::report::ERROR_SCHEMA_VERSION,
            },
        ],
        pdfium,
        limitations: vec![
            "PDFium is supplied separately; see docs/pdfium.md",
            "only aarch64-apple-darwin has been runtime-verified against a real PDFium build",
            "the desktop GUI is not connected to this pipeline yet",
            "no OCR, hidden text layers, bookmarks, annotations, forms, or attachments are preserved",
            "no Ancient Greek or polytonic typography preservation claim is made",
        ],
    };
    ReportEnvelope::new(INFO_REPORT_SCHEMA, INFO_REPORT_SCHEMA_VERSION, result)
}

fn print_human(info: &InfoReport) {
    println!("{} — {}", info.name, info.phase);
    println!("Version:     {}", info.version);
    println!(
        "Build:       {} ({}-{})",
        info.build_profile, info.target_arch, info.target_os
    );
    println!(
        "Supported DPI: {}",
        info.supported_dpi
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if info.pdfium.probed {
        match (&info.pdfium.resolved, &info.pdfium.error) {
            (Some(resolved), _) => println!("PDFium:      {resolved}"),
            (None, Some(error)) => println!("PDFium:      probe failed: {error}"),
            (None, None) => unreachable!("a probe always sets resolved or error"),
        }
    } else {
        println!("PDFium:      not probed (pass --probe-pdfium to check)");
    }
    println!("Commands:    info, inspect, analyze, process, preview (see --help)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes_with_the_documented_schema() {
        let report = build_report(PdfiumInfo {
            probed: false,
            resolved: None,
            error: None,
        });
        assert_eq!(report.schema, INFO_REPORT_SCHEMA);
        assert_eq!(report.schema_version, INFO_REPORT_SCHEMA_VERSION);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"supported_dpi\""));
        assert!(json.contains("\"limitations\""));
    }

    #[test]
    fn unprobed_pdfium_info_reports_no_resolution_and_no_error() {
        let report = build_report(PdfiumInfo {
            probed: false,
            resolved: None,
            error: None,
        });
        assert!(!report.result.pdfium.probed);
        assert!(report.result.pdfium.resolved.is_none());
        assert!(report.result.pdfium.error.is_none());
    }
}
