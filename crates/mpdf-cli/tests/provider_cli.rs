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
fn provider_list_reports_three_modes_and_names_local_as_the_default() {
    let result = mpdf(&["provider", "list", "--json"]);
    assert!(result.status.success());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["default_mode"], "local");
    let modes = report["modes"].as_array().unwrap();
    assert_eq!(modes.len(), 3);
    let ids: Vec<&str> = modes
        .iter()
        .map(|mode| mode["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["local", "gemini-byok", "mpdf-credits"]);

    let local = &modes[0];
    assert_eq!(local["default"], true);
    assert_eq!(local["uses_network"], false);
    assert_eq!(local["requires_credential"], false);
}

#[test]
fn provider_list_never_claims_credits_is_production_ready() {
    let result = mpdf(&["provider", "list", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let credits = &report["modes"][2];
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
fn a_cloud_run_without_explicit_consent_is_refused_before_anything_is_opened() {
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "gemini-byok",
    ]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--cloud-consent"), "{stderr}");
    // The refusal is about consent, not about the missing file: nothing was
    // opened, so the source path was never even resolved.
    assert!(!stderr.contains("No such file"), "{stderr}");
}

#[test]
fn production_binary_compiles_out_brokered_execution() {
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
    assert!(
        stderr.contains("not compiled into this production build"),
        "{stderr}"
    );
    assert!(stderr.contains("dev-credits"), "{stderr}");
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
    ]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("not compiled into this production build")
    );
}

#[test]
fn a_dry_run_describes_the_upload_without_opening_or_calling_anything() {
    // The source does not exist. A dry run must still succeed, because it is
    // a description of intent, not an execution — and it must never reach a
    // provider.
    let result = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--ocr-provider",
        "gemini-byok",
        "--cloud-consent",
        "--dry-run",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("dry run"), "{stdout}");
    assert!(stdout.contains("uploads:"), "{stdout}");
    assert!(stdout.contains("****"), "{stdout}");
    // A dry run of a local conversion says plainly that nothing leaves.
    let local = mpdf(&[
        "run",
        "/nonexistent/source.pdf",
        "--output",
        "/nonexistent/out.pdf",
        "--dry-run",
    ]);
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stdout)
        .contains("nothing; local OCR makes no network request"));
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
fn the_credential_command_refuses_to_take_a_key_from_a_terminal() {
    // stdin here is not a terminal (the test harness pipes it), so this
    // exercises the empty-input path rather than the interactive refusal.
    let result = mpdf(&["provider", "credential", "set", "--slot", "default"]);
    assert!(!result.status.success());
}

#[test]
fn a_local_connection_test_makes_no_request_and_says_so() {
    let result = mpdf(&["provider", "test", "--mode", "local"]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("no network"));
}
