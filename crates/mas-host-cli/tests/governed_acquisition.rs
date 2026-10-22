//! Exercise source acquisition and inherited restrictions through the real CLI.
use std::{path::Path, process::Command};

#[test]
fn acquired_source_restricts_baseline_actions_and_survives_restart_without_recollection() {
    let result = Command::new("python3")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/governed-acquisition/verify.py"),
        )
        .args(["--host", env!("CARGO_BIN_EXE_mas-host")])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(receipt["scenario"], "governed-acquisition");
    assert_eq!(receipt["status"], "PASS");
    print!("{}", String::from_utf8_lossy(&result.stdout));
}
