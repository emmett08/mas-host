//! Exercise bounded queued response effects through the actual governed host.
use std::{path::Path, process::Command};

fn run_scenario(name: &str, composed: bool) {
    let mut command = Command::new("python3");
    command
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/privacy-lifecycle/verify.py"),
        )
        .args(["--host", env!("CARGO_BIN_EXE_mas-host"), "--scenario", name])
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if composed {
        command.arg("--with-inspector");
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(receipt["scenario"], name);
    assert_eq!(receipt["status"], "PASS");
}

#[test]
fn queued_retries_and_read_only_reconciliation_preserve_original_action_on_replay() {
    for name in [
        "retry-once",
        "reconcile-accepted",
        "reconcile-absent",
        "reconcile-unknown",
    ] {
        run_scenario(name, false);
    }
}

#[test]
fn recipient_acknowledgement_and_timed_escalation_remain_distinct_from_acceptance() {
    for name in ["acknowledged", "escalate"] {
        run_scenario(name, false);
    }
}

#[test]
fn contextual_inspection_custody_notification_recovery_and_adjudication_compose() {
    run_scenario("retry-once", true);
}
