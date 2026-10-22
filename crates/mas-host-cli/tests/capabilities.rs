//! Exercise approved package substitution and retained-runtime identity through the real CLI.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project {
    root: PathBuf,
    project: PathBuf,
}
fn digest(value: &Value) -> String {
    sha(value.to_string().as_bytes())
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn write(path: &Path, value: &Value) {
    fs::write(path, value.to_string()).unwrap();
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn rejected(output: Output, expected: &str) {
    assert!(!output.status.success(), "unexpected success");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-capability-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let project = root.join("project");
        let prepared = Command::new("python3")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/structured-evidence/prepare.py"),
            )
            .args([
                "--language",
                "python",
                "--runtime",
                "/usr/bin/python3",
                "--destination",
            ])
            .arg(&project)
            .output()
            .unwrap();
        assert!(
            prepared.status.success(),
            "{}",
            String::from_utf8_lossy(&prepared.stderr)
        );
        Self { root, project }
    }
    fn command(&self, command: &str) -> Command {
        let mut process = Command::new(env!("CARGO_BIN_EXE_mas-host"));
        process.current_dir(&self.project).arg(command);
        process
    }
    fn build(&self) -> Output {
        self.command("build")
            .args([
                "--project",
                ".",
                "--capabilities",
                "capabilities.json",
                "--capability-policy",
                "approvals.json",
            ])
            .output()
            .unwrap()
    }
    fn package(&self) -> Value {
        read(&self.project.join("capabilities.json"))["packages"][0].clone()
    }
    fn set_package(&self, package: Value, approve: bool) {
        write(&self.project.join("capability.json"), &package);
        write(
            &self.project.join("capabilities.json"),
            &json!({"version":"mas-capabilities/1","packages":[package]}),
        );
        if approve {
            let mut policy = read(&self.project.join("approvals.json"));
            policy["approved_packages"] = json!([digest(&package)]);
            write(&self.project.join("approvals.json"), &policy);
        }
    }
    fn run(&self, bundle: &str, run: &str, policy: bool) -> Output {
        let mut command = self.command("run");
        command.args([
            "--bundle",
            bundle,
            "--agent",
            "evidence_python",
            "--input",
            "input.json",
            "--state",
            "state",
            "--run",
            run,
        ]);
        if policy {
            command.args(["--capability-policy", "approvals.json"]);
        }
        command.output().unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exact_approval_binds_real_collector_and_retained_observation_replay() {
    let project = Project::new();
    let checked = success(
        project
            .command("check-capability")
            .args(["--declaration", "capability.json"])
            .output()
            .unwrap(),
    );
    assert_eq!(checked["sha256"], digest(&project.package()));
    assert_eq!(checked["approval_granted"], false);
    let built = success(project.build());
    let bundle = built["bundle"].as_str().unwrap();
    rejected(
        project.run(bundle, "without-policy", false),
        "explicit operator policy",
    );
    let first = success(project.run(bundle, "approved", true));
    assert_eq!(first["status"], "completed");
    assert_eq!(
        first["output"]["operationId"],
        read(&project.project.join("input.json"))["operationId"]
    );
    let before = success(
        project
            .command("inspect")
            .args(["--state", "state", "--run", "approved"])
            .output()
            .unwrap(),
    );
    // Changing authoring files cannot alter the retained package's original read.
    fs::write(project.project.join("fixture.json"), "changed-source").unwrap();
    assert_eq!(success(project.run(bundle, "approved", true)), first);
    let after = success(
        project
            .command("inspect")
            .args(["--state", "state", "--run", "approved"])
            .output()
            .unwrap(),
    );
    assert_eq!(before["protocol"]["calls"], after["protocol"]["calls"]);
    rejected(project.build(), "digest mismatch");
}

#[test]
fn same_name_substitution_and_missing_static_requirements_reject_before_publication() {
    let project = Project::new();
    success(project.build());
    let previous = fs::read(project.project.join(".mas/build/bundle.json")).unwrap();
    let mut changed = project.package();
    let mut fixture = fs::read(project.project.join("fixture.json")).unwrap();
    fixture.push(b' ');
    fs::write(project.project.join("fixture.json"), &fixture).unwrap();
    for record in changed["files"].as_array_mut().unwrap() {
        if record["path"] == "fixture.json" {
            record["sha256"] = json!(sha(&fixture));
        }
    }
    project.set_package(changed, false);
    rejected(project.build(), "lacks operator approval");
    assert_eq!(
        fs::read(project.project.join(".mas/build/bundle.json")).unwrap(),
        previous
    );
    let package = project.package();
    project.set_package(package, true);
    let mut policy = read(&project.project.join("approvals.json"));
    policy["requirements"]["grants"] = json!([]);
    write(&project.project.join("approvals.json"), &policy);
    rejected(project.build(), "missing static requirement");
    policy["requirements"]["grants"] = json!(["synthetic-deployment-read"]);
    policy["requirements"]["runtime_ids"] = json!([]);
    write(&project.project.join("approvals.json"), &policy);
    rejected(project.build(), "incompatible runtime");
}

#[test]
fn closed_versions_signature_requirements_and_unsupported_schemas_reject() {
    let project = Project::new();
    let original = project.package();
    for changed in [
        {
            let mut p = original.clone();
            p["version"] = json!("mas-capability/999");
            p
        },
        {
            let mut p = original.clone();
            p["credential_value"] = json!("forbidden");
            p
        },
    ] {
        project.set_package(changed, true);
        assert!(!project.build().status.success());
    }
    project.set_package(original, true);
    let mut policy = read(&project.project.join("approvals.json"));
    policy["require_attestation"] = json!(true);
    write(&project.project.join("approvals.json"), &policy);
    rejected(project.build(), "attestation required");
    policy["require_attestation"] = json!(false);
    write(&project.project.join("approvals.json"), &policy);
    let mut schema = read(&project.project.join("schemas/input.json"));
    schema["pattern"] = json!(".*");
    write(&project.project.join("schemas/input.json"), &schema);
    rejected(project.build(), "unsupported schema keyword");
}

#[test]
fn file_escape_missing_argument_and_changed_argv_cannot_keep_old_approval() {
    let project = Project::new();
    let original = project.package();
    let mut escape = original.clone();
    escape["files"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"../outside", "sha256":"0".repeat(64)}));
    project.set_package(escape, true);
    rejected(project.build(), "relative project path");
    std::os::unix::fs::symlink(
        project.project.join("fixture.json"),
        project.project.join("linked-fixture"),
    )
    .unwrap();
    let mut symlink = original.clone();
    symlink["files"].as_array_mut().unwrap().push(json!({"path":"linked-fixture", "sha256":sha(&fs::read(project.project.join("fixture.json")).unwrap())}));
    project.set_package(symlink, true);
    rejected(project.build(), "symlink unsupported");
    let mut missing = original.clone();
    missing["files"]
        .as_array_mut()
        .unwrap()
        .retain(|f| f["path"] != "python/collector.py");
    project.set_package(missing, true);
    rejected(project.build(), "omits command file");
    project.set_package(original, true);
    let config_path = project.project.join("agents.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace("--fixture", "--changed-target");
    fs::write(config_path, config).unwrap();
    rejected(project.build(), "implementation binding mismatch");
}

#[test]
fn changed_runtime_is_rejected_by_cli_and_existing_command_dispatch_gate() {
    let project = Project::new();
    let runtime = project.root.join("runtime-data");
    fs::write(&runtime, "original-runtime").unwrap();
    let mut package = project.package();
    package["runtime"]["files"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":runtime,"sha256":sha(b"original-runtime")}));
    project.set_package(package, true);
    let built = success(project.build());
    let bundle = built["bundle"].as_str().unwrap();
    fs::write(&runtime, "changed-runtime").unwrap();
    rejected(
        project.run(bundle, "changed-runtime", true),
        "digest mismatch",
    );
    // An ordinary Engine path without package preflight still executes the
    // maintained command asset gate before its external effect is admitted.
    let mut ordinary = read(Path::new(bundle));
    ordinary["agents"][0]["configuration"]
        .as_object_mut()
        .unwrap()
        .remove("capability");
    let ordinary_path = project.project.join("ordinary.json");
    write(&ordinary_path, &ordinary);
    rejected(
        project.run(ordinary_path.to_str().unwrap(), "dispatch-gate", false),
        "worker asset digest differs",
    );
    let inspected = success(
        project
            .command("inspect")
            .args(["--state", "state", "--run", "dispatch-gate"])
            .output()
            .unwrap(),
    );
    assert_eq!(inspected["protocol"]["calls"].as_array().unwrap().len(), 0);
}

#[test]
fn changed_executable_bytes_cannot_reuse_approved_package() {
    let project = Project::new();
    let first = success(project.build());
    let original_bundle = read(Path::new(first["bundle"].as_str().unwrap()));
    let mut implementation = original_bundle["agents"][0]["configuration"]["capability"]
        ["source_implementation"]
        .clone();
    let original = implementation["executable"].as_str().unwrap().to_owned();
    let pinned = project.root.join("pinned-interpreter");
    fs::copy(&original, &pinned).unwrap();
    implementation["executable"] = json!(pinned);
    let config_path = project.project.join("agents.toml");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace(&original, pinned.to_str().unwrap()),
    )
    .unwrap();
    let mut package = project.package();
    package["implementation_sha256"] = json!(digest(&implementation));
    package["runtime"]["files"][0]["path"] = json!(pinned);
    project.set_package(package, true);
    let built = success(project.build());
    fs::write(&pinned, "changed-executable").unwrap();
    rejected(
        project.run(built["bundle"].as_str().unwrap(), "changed-binary", true),
        "digest mismatch",
    );
    rejected(project.build(), "digest mismatch");
}

#[test]
fn retained_package_cannot_switch_implementation_runner_or_schemas() {
    let project = Project::new();
    let built = success(project.build());
    let original = read(Path::new(built["bundle"].as_str().unwrap()));
    let destination = project.project.join("substituted.json");
    let mut kind = original.clone();
    kind["agents"][0]["configuration"]["kind"] = json!("workflow");
    kind["agents"][0]["configuration"]["plan"] =
        json!({"type":"value","output":{"unapproved":true}});
    write(&destination, &kind);
    rejected(
        project.run(destination.to_str().unwrap(), "kind-substitution", true),
        "packaged command configuration",
    );
    let mut runner = original.clone();
    runner["agents"][0]["executable"] = json!("/usr/bin/python3");
    runner["agents"][0]["sha256"] = json!(sha(&fs::read("/usr/bin/python3").unwrap()));
    write(&destination, &runner);
    rejected(
        project.run(destination.to_str().unwrap(), "runner-substitution", true),
        "maintained command runner",
    );
    let mut arguments = original.clone();
    arguments["agents"][0]["arguments"] = json!(["__builtin", "unapproved"]);
    write(&destination, &arguments);
    rejected(
        project.run(
            destination.to_str().unwrap(),
            "runner-argv-substitution",
            true,
        ),
        "maintained command runner",
    );
    for schema in ["input_schema", "output_schema"] {
        let mut substituted = original.clone();
        substituted["agents"][0]["configuration"][schema] = json!({"type":"object"});
        assert_ne!(
            substituted["agents"][0]["configuration"][schema],
            substituted["agents"][0][schema]
        );
        write(&destination, &substituted);
        rejected(
            project.run(destination.to_str().unwrap(), schema, true),
            "retained schema mismatch",
        );
    }
    let mut unknown = original;
    unknown["agents"][0]["configuration"]["unapproved_configuration"] = json!(true);
    write(&destination, &unknown);
    rejected(
        project.run(destination.to_str().unwrap(), "unknown-setting", true),
        "unknown field",
    );
}

#[test]
fn aggregate_bundle_limit_preserves_previous_valid_publication() {
    let project = Project::new();
    success(project.build());
    let previous = fs::read(project.project.join(".mas/build/bundle.json")).unwrap();
    let mut schema = read(&project.project.join("schemas/output.json"));
    schema["description"] = json!("x".repeat(35000));
    write(&project.project.join("schemas/output.json"), &schema);
    let mut package = project.package();
    package["output_schema_sha256"] = json!(digest(&schema));
    for record in package["files"].as_array_mut().unwrap() {
        if record["path"] == "schemas/output.json" {
            record["sha256"] = json!(sha(
                &fs::read(project.project.join("schemas/output.json")).unwrap()
            ));
        }
    }
    project.set_package(package, true);
    rejected(project.build(), "generated bundle: SIZE");
    assert_eq!(
        fs::read(project.project.join(".mas/build/bundle.json")).unwrap(),
        previous
    );
}
