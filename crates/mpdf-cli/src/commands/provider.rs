//! `mpdf provider` — inspect current OCR product modes.
//!
//! Historical credential commands still parse so scripts receive one stable
//! migration error, but they do not read stdin or consult a credential store.

use std::process::ExitCode;

use mpdf_core::ocr_provider::credits;
use mpdf_core::ocr_provider::structured_bbox;
use mpdf_core::ocr_provider::{OcrProviderMode, BYOK_DISABLED_MESSAGE};

use crate::cli::{OcrProviderModeArg, ProviderCredentialArgs, ProviderListArgs, ProviderTestArgs};
use crate::errors::ExitReason;
use crate::output;

#[derive(serde::Serialize)]
struct ProviderModeReport {
    id: &'static str,
    display_name: &'static str,
    default: bool,
    uses_network: bool,
    requires_credential: bool,
    execution_location: &'static str,
    production_ready: bool,
    availability: &'static str,
    blockers: Vec<&'static str>,
    model: Option<&'static str>,
    structured_bbox_default_enabled: bool,
}

#[derive(serde::Serialize)]
struct ProviderListReport {
    schema: &'static str,
    schema_version: &'static str,
    default_mode: &'static str,
    modes: Vec<ProviderModeReport>,
}

fn describe(mode: OcrProviderMode) -> ProviderModeReport {
    let production_ready = mode.availability() == "stable";
    ProviderModeReport {
        id: mode.id(),
        display_name: mode.display_name(),
        default: mode.is_default(),
        uses_network: mode.uses_network(),
        requires_credential: false,
        execution_location: mode.execution_location(),
        production_ready,
        availability: mode.availability(),
        blockers: mode.release_blockers(),
        // No production complete-OCR backend/model is selected yet.
        model: None,
        structured_bbox_default_enabled: structured_bbox::GATES_VALIDATED_FOR_DEFAULT,
    }
}

pub fn list(args: ProviderListArgs) -> ExitCode {
    let report = ProviderListReport {
        schema: "mpdf-ocr-providers",
        schema_version: "1.0",
        default_mode: OcrProviderMode::Local.id(),
        modes: OcrProviderMode::PRODUCT_MODES
            .into_iter()
            .map(describe)
            .collect(),
    };
    if args.output_mode.json {
        output::print_json(&report, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        for mode in &report.modes {
            println!(
                "{}{}\n  {}\n  network: {}   credential: {}   execution: {}",
                mode.id,
                if mode.default { "  (default)" } else { "" },
                mode.display_name,
                if mode.uses_network { "yes" } else { "no" },
                if mode.requires_credential {
                    "yes"
                } else {
                    "no"
                },
                mode.execution_location,
            );
            if !mode.production_ready {
                println!("  availability: {}", mode.availability);
                for blocker in &mode.blockers {
                    println!("    - {blocker}");
                }
            }
            println!();
        }
    }
    ExitReason::Success.exit_code()
}

pub fn test(args: ProviderTestArgs) -> ExitCode {
    match args.mode {
        OcrProviderModeArg::Local => {
            if !args.output_mode.quiet {
                println!("local: no connection test is needed; local OCR uses no network.");
            }
            ExitReason::Success.exit_code()
        }
        OcrProviderModeArg::GeminiByok => {
            let _ = args;
            byok_disabled()
        }
        OcrProviderModeArg::MpdfCredits => {
            eprintln!(
                "error: M PDF Cloud OCR has no production service in this build.\n{}",
                credits::release_blockers()
                    .iter()
                    .map(|blocker| format!("  - {blocker}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            ExitReason::UsageError.exit_code()
        }
        OcrProviderModeArg::BrokerTest => {
            eprintln!("broker-test is hidden and test-only; use mpdf run with explicit consent, fail fallback and a broker-test config. No model was contacted.");
            ExitReason::UsageError.exit_code()
        }
    }
}

fn byok_disabled() -> ExitCode {
    eprintln!("error: {BYOK_DISABLED_MESSAGE}");
    ExitReason::UsageError.exit_code()
}

pub fn credential_set(args: ProviderCredentialArgs) -> ExitCode {
    let _ = args;
    byok_disabled()
}

pub fn credential_status(args: ProviderCredentialArgs) -> ExitCode {
    let _ = args;
    byok_disabled()
}

pub fn credential_delete(args: ProviderCredentialArgs) -> ExitCode {
    let _ = args;
    byok_disabled()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_product_modes_are_listed_and_local_is_the_default() {
        let modes: Vec<_> = OcrProviderMode::PRODUCT_MODES
            .into_iter()
            .map(describe)
            .collect();
        assert_eq!(modes.len(), 2);
        let defaults: Vec<_> = modes.iter().filter(|mode| mode.default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].id, "local");
        assert!(!defaults[0].uses_network);
    }

    #[test]
    fn credits_is_reported_as_not_production_ready_with_its_blockers() {
        let credits = describe(OcrProviderMode::MpdfCredits);
        assert!(!credits.production_ready);
        assert!(!credits.blockers.is_empty());
    }

    #[test]
    fn no_mode_advertises_structured_boxes_as_a_default() {
        for mode in OcrProviderMode::PRODUCT_MODES {
            assert!(!describe(mode).structured_bbox_default_enabled);
        }
    }
}
