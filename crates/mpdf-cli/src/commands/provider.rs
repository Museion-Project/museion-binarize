//! `mpdf provider` — inspect OCR modes and manage model-provider keys.
//!
//! # Why there is no `--api-key`
//!
//! A key on a command line is in the shell history, in `ps` output, in any
//! CI log that echoes the command, and in the process table for every other
//! user on the machine. `mpdf provider credential set` therefore reads the
//! key from stdin and hands it straight to the OS credential store. The same
//! reasoning already governs `MPDF_PDF_PASSWORD`; see `docs/cli.md`.

use std::io::{IsTerminal, Read};
use std::process::ExitCode;

use mpdf_api_client::cloud_ocr::{
    CloudTransportPolicy, GeminiByokFactory, ModelProviderSecretStore, GEMINI_ENDPOINT,
};
use mpdf_api_client::{Secret, SecretStore};
use mpdf_core::ocr_provider::credits;
use mpdf_core::ocr_provider::gemini::{CloudOcrConfig, DEFAULT_MODEL};
use mpdf_core::ocr_provider::structured_bbox;
use mpdf_core::ocr_provider::OcrProviderMode;

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
        requires_credential: mode.uses_network(),
        execution_location: mode.execution_location(),
        production_ready,
        availability: mode.availability(),
        blockers: mode.release_blockers(),
        model: mode.uses_network().then_some(DEFAULT_MODEL),
        structured_bbox_default_enabled: structured_bbox::GATES_VALIDATED_FOR_DEFAULT,
    }
}

pub fn list(args: ProviderListArgs) -> ExitCode {
    let report = ProviderListReport {
        schema: "mpdf-ocr-providers",
        schema_version: "1.0",
        default_mode: OcrProviderMode::Local.id(),
        modes: OcrProviderMode::ALL.into_iter().map(describe).collect(),
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
            let factory = GeminiByokFactory::new(
                CloudOcrConfig::gemini_byok(&args.slot),
                args.endpoint.as_deref().unwrap_or(GEMINI_ENDPOINT),
                CloudTransportPolicy::default(),
                "connection-test",
            );
            // Metadata only: this call transcribes nothing and bills nothing.
            let result = factory.connection_test();
            if args.output_mode.json {
                output::print_json(&result, args.output_mode.pretty);
            } else if !args.output_mode.quiet {
                println!(
                    "{} / {}\n  model available: {}\n  credential slot {}: {}\n  {}",
                    result.provider_name,
                    result.model,
                    result.model_available,
                    result.credential.slot,
                    if result.credential.present {
                        "a key is stored (****)"
                    } else {
                        "no key is stored"
                    },
                    result.diagnostic
                );
            }
            if result.model_available {
                ExitReason::Success.exit_code()
            } else {
                ExitReason::InputError.exit_code()
            }
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
    }
}

#[derive(serde::Serialize)]
struct CredentialReport {
    slot: String,
    present: bool,
    masked: String,
    service: &'static str,
}

fn report(slot: &str, present: bool, args: &crate::cli::OutputArgs) {
    let report = CredentialReport {
        slot: slot.to_owned(),
        present,
        masked: mpdf_core::ocr_provider::redaction::MASKED_CREDENTIAL.to_owned(),
        service: mpdf_api_client::cloud_ocr::GEMINI_CREDENTIAL_SERVICE,
    };
    if args.json {
        output::print_json(&report, args.pretty);
    } else if !args.quiet {
        println!(
            "slot {}: {} (service {})",
            report.slot,
            if present {
                "a key is stored (****)"
            } else {
                "no key is stored"
            },
            report.service
        );
    }
}

pub fn credential_set(args: ProviderCredentialArgs) -> ExitCode {
    if std::io::stdin().is_terminal() {
        eprintln!(
            "error: the key is read from stdin, never from an argument.\n\
             Pipe it in, for example:  pbpaste | mpdf provider credential set --slot default"
        );
        return ExitReason::UsageError.exit_code();
    }
    let mut value = String::new();
    if std::io::stdin().read_to_string(&mut value).is_err() {
        eprintln!("error: could not read the key from stdin");
        return ExitReason::UsageError.exit_code();
    }
    let trimmed = value.trim().to_owned();
    // The buffer is cleared rather than left in the process image longer than
    // it must be. `Secret` zeroizes its own copy on drop.
    value.clear();
    if trimmed.is_empty() || trimmed.len() > 4096 {
        eprintln!("error: the key is empty or implausibly long");
        return ExitReason::UsageError.exit_code();
    }
    match ModelProviderSecretStore.set(&args.slot, Secret::new(trimmed)) {
        Ok(()) => {
            report(&args.slot, true, &args.output_mode);
            ExitReason::Success.exit_code()
        }
        Err(error) => {
            // The error is the store's, never the key's.
            eprintln!("error: {error}");
            ExitReason::InputError.exit_code()
        }
    }
}

pub fn credential_status(args: ProviderCredentialArgs) -> ExitCode {
    let present = ModelProviderSecretStore.describe(&args.slot).present;
    report(&args.slot, present, &args.output_mode);
    ExitReason::Success.exit_code()
}

pub fn credential_delete(args: ProviderCredentialArgs) -> ExitCode {
    match ModelProviderSecretStore.delete(&args.slot) {
        Ok(()) => {
            report(&args.slot, false, &args.output_mode);
            if !args.output_mode.quiet && !args.output_mode.json {
                println!(
                    "Cloud OCR now has no credential; runs fall back to --ocr-provider local."
                );
            }
            ExitReason::Success.exit_code()
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitReason::InputError.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_is_listed_and_only_local_is_the_default() {
        let modes: Vec<_> = OcrProviderMode::ALL.into_iter().map(describe).collect();
        assert_eq!(modes.len(), 3);
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
    fn gemini_is_beta_not_production_ready() {
        let gemini = describe(OcrProviderMode::GeminiByok);
        assert_eq!(gemini.availability, "beta");
        assert!(!gemini.production_ready);
        assert!(!gemini.blockers.is_empty());
    }

    #[test]
    fn no_mode_advertises_structured_boxes_as_a_default() {
        for mode in OcrProviderMode::ALL {
            assert!(!describe(mode).structured_bbox_default_enabled);
        }
    }
}
