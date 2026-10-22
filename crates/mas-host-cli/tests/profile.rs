//! Profile preflight reports implemented capabilities without creating execution state.
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct ProfileFile(PathBuf);

impl ProfileFile {
    fn new(value: &Value) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mas-profile-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("profile.json"), value.to_string()).unwrap();
        Self(directory)
    }

    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .current_dir(&self.0)
            .args(["check-profile", "--profile", "profile.json"])
            .output()
            .unwrap()
    }
}

impl Drop for ProfileFile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn trusted_local_preflight_has_no_execution_side_effects() {
    let profile = ProfileFile::new(&json!({
        "version":"mas-host-profile/1",
        "profile":"trusted-local",
        "required_capabilities":["trusted-local-execution","local-effect-replay"]
    }));
    let result = profile.check();
    assert!(result.status.success(), "{:?}", result);
    let output: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output["profile"], "trusted-local");
    assert_eq!(fs::read_dir(&profile.0).unwrap().count(), 1);
}

#[test]
fn managed_and_shared_profiles_cannot_self_declare_runtime_support() {
    for name in ["managed-cluster", "shared-enterprise"] {
        let profile = ProfileFile::new(&json!({
            "version":"mas-host-profile/1",
            "profile":name,
            "required_capabilities":["authenticated-scope","pre-persistence-handling",
                "current-disclosure-admission","governed-payload-lifecycle",
                "effect-output-handling","shared-authority","fenced-scheduling",
                "shared-quotas"]
        }));
        let result = profile.check();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        let error: Value = serde_json::from_slice(&result.stderr).unwrap();
        assert_eq!(error["status"], "error");
        assert!(error["error"].as_str().unwrap().contains("capabilit"));
        assert_eq!(fs::read_dir(&profile.0).unwrap().count(), 1);
    }
}

#[test]
fn unknown_profile_fields_and_versions_are_rejected() {
    let base = json!({"version":"mas-host-profile/1","profile":"trusted-local",
        "required_capabilities":[]});
    let mut unknown = base.clone();
    unknown["permit_all"] = json!(true);
    let mut future = base;
    future["version"] = json!("mas-host-profile/2");
    for value in [unknown, future] {
        let profile = ProfileFile::new(&value);
        let result = profile.check();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
}
