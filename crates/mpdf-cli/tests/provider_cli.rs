//! Contract for the OCR provider surface of the CLI.
//!
//! None of these tests needs PDFium, a network, a credential or a model: they
//! assert the *refusals and disclosures* that happen before any of that is
//! touched. That is deliberate — the guarantees worth testing here are "this
//! command does not call anything" and "this command tells the truth about
//! what it would do", both of which are only meaningful offline.

use std::process::Command;

fn mpdf(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args(args)
        .output()
        .expect("mpdf runs")
}

#[test]
fn provider_list_excludes_legacy_byok_and_names_local_as_the_default() {
    let result = mpdf(&["provider", "list", "--json"]);
    assert!(result.status.success());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["default_mode"], "local");
    let modes = report["modes"].as_array().unwrap();
    assert_eq!(modes.len(), 2);
    let ids: Vec<&str> = modes
        .iter()
        .map(|mode| mode["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["local", "mpdf-credits"]);

    let local = &modes[0];
    assert_eq!(local["default"], true);
    assert_eq!(local["uses_network"], false);
    assert_eq!(local["requires_credential"], false);
}

#[test]
fn current_help_does_not_advertise_byok_or_credential_management() {
    for args in [vec!["run", "--help"], vec!["provider", "--help"]] {
        let result = mpdf(&args);
        assert!(result.status.success());
        let help = String::from_utf8_lossy(&result.stdout);
        assert!(!help.contains("gemini-byok"), "{args:?}: {help}");
        assert!(!help.contains("\n  credential"), "{args:?}: {help}");
    }
}

#[test]
fn provider_list_never_claims_credits_is_production_ready() {
    let result = mpdf(&["provider", "list", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let credits = &report["modes"][1];
    assert_eq!(credits["id"], "mpdf-credits");
    assert_eq!(credits["production_ready"], false);
    let blockers = credits["blockers"].as_array().unwrap();
    assert!(!blockers.is_empty());
    assert!(blockers
        .iter()
        .any(|blocker| blocker.as_str().unwrap().contains("payment")));
}

#[test]
fn no_command_anywhere_accepts_a_key_as_an_argument() {
    // The rule: a key on a command line is in the shell history, in `ps`, and
    // in every CI log that echoes the command. There must be no way to put
    // one there, so the flag simply does not exist.
    for args in [
        vec!["run", "--help"],
        vec!["provider", "--help"],
        vec!["provider", "credential", "set", "--help"],
    ] {
        let result = mpdf(&args);
        let help = String::from_utf8_lossy(&result.stdout);
        assert!(!help.contains("--api-key"), "{args:?} offers --api-key");
        assert!(!help.contains("--key"), "{args:?} offers --key");
        assert!(!help.contains("--secret"), "{args:?} offers --secret");
        assert!(!help.contains("--token"), "{args:?} offers --token");
    }
}

#[test]
fn a_brokered_cloud_run_without_explicit_consent_is_refused_before_anything_is_opened() {
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "mpdf-credits",
    ]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--cloud-consent"), "{stderr}");
    // The refusal is about consent, not about the missing file: nothing was
    // opened, so the source path was never even resolved.
    assert!(!stderr.contains("No such file"), "{stderr}");
}

#[test]
fn brokered_execution_requires_a_cost_limit_before_reporting_backend_unavailability() {
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "mpdf-credits",
        "--cloud-consent",
    ]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--max-credits"), "{stderr}");
}

#[test]
fn a_custom_broker_endpoint_cannot_bypass_the_production_compile_gate() {
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "mpdf-credits",
        "--cloud-consent",
        "--cloud-endpoint",
        "https://credits.example.invalid",
        "--max-credits",
        "100",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("complete coordinate OCR contract"));
}

#[test]
fn legacy_byok_is_disabled_before_consent_files_credentials_or_a_dry_run() {
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "gemini-byok",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("disabled in this version"), "{stderr}");
    assert!(stderr.contains("complete coordinate OCR"), "{stderr}");
    assert!(!stderr.contains("--cloud-consent"), "{stderr}");
    assert!(!stderr.contains("No such file"), "{stderr}");

    // A dry run of a local conversion says plainly that nothing leaves.
    let local = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--dry-run",
    ]);
    assert!(local.status.success());
    let stdout = String::from_utf8_lossy(&local.stdout);
    assert!(stdout.contains("argument/policy checks only"), "{stdout}");
    assert!(
        stdout.contains("input and optional OCR plugin/runtime were not inspected"),
        "{stdout}"
    );
    assert!(stdout.contains("nothing; local OCR makes no network request"));
    assert!(stdout.contains("reliable native-text PDFs work without it"));
    assert!(stdout.contains("scanned pages require it"));
}

#[test]
fn dry_run_help_does_not_claim_runtime_or_input_validation() {
    let result = mpdf(&["run", "--help"]);
    assert!(result.status.success());
    let help = String::from_utf8_lossy(&result.stdout);
    assert!(
        help.contains("Checks argument/policy consistency only"),
        "{help}"
    );
    assert!(help.contains("does not open the input"), "{help}");
    assert!(
        help.contains("inspect the optional OCR plugin/runtime"),
        "{help}"
    );
    assert!(!help.contains("Validate the configuration"), "{help}");
}

#[test]
fn consent_on_a_local_run_is_a_usage_error_rather_than_silently_ignored() {
    // "I ticked the cloud box and it ran locally" is the confusion that makes
    // someone believe they used a model they did not.
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--cloud-consent",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("local OCR uploads nothing"));
}

#[test]
fn legacy_credential_commands_return_the_disabled_error_without_reading_a_key() {
    for verb in ["set", "status", "delete"] {
        let result = mpdf(&["provider", "credential", verb, "--slot", "../ignored"]);
        assert!(!result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains("disabled in this version"), "{stderr}");
        assert!(stderr.contains("complete coordinate OCR"), "{stderr}");
    }
}

#[test]
fn a_local_connection_test_makes_no_request_and_says_so() {
    let result = mpdf(&["provider", "test", "--mode", "local"]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("no network"));
}

#[test]
fn legacy_byok_connection_test_is_disabled_without_reading_credentials() {
    let result = mpdf(&["provider", "test", "--mode", "gemini-byok"]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("disabled in this version"), "{stderr}");
}
