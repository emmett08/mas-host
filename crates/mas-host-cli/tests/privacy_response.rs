//! Exercise response effects through the real CLI guardian and governed A2A ingress.
use std::{path::Path, process::Command};

fn scenario(name: &str) {
    let result = Command::new("python3")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/privacy-response/verify.py"),
        )
        .args(["--host", env!("CARGO_BIN_EXE_mas-host"), "--scenario", name])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["scenario"], name);
    assert_eq!(result["status"], "PASS");
}

#[test]
fn completed_withheld_collection_creates_safe_case_and_one_notification_across_restart() {
    scenario("accepted");
}
#[test]
fn throttling_remains_refused_with_retry_eligibility_and_replay_does_not_resend() {
    scenario("throttled");
}
#[test]
fn lost_notification_acknowledgement_retains_intent_and_original_unknown_without_redispatch() {
    scenario("unknown");
}
#[test]
fn revocation_after_safe_intent_blocks_fresh_notification_admission() {
    scenario("revoke-before-notify");
}
