//! Equivalent authoring formats must retain the bundle and resume identity.
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-authoring-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, text: &str) {
        fs::write(self.0.join(name), text).unwrap();
    }
    fn command(&self, command: &str) -> Command {
        let mut process = Command::new(env!("CARGO_BIN_EXE_mas-host"));
        process.current_dir(&self.0).arg(command);
        process
    }
    fn build(&self, extra: &[&str]) -> Output {
        self.command("build")
            .args(["--project", ".", "--sdk-root"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .args(extra)
            .output()
            .unwrap()
    }
    fn run(&self, bundle: &str, file: &str, format: Option<&str>, stdin: Option<&str>) -> Output {
        let mut command = self.command("run");
        command.args([
            "--bundle", bundle, "--agent", "echo", "--input", file, "--state", "state", "--run",
            "same_run",
        ]);
        if let Some(format) = format {
            command.args(["--input-format", format]);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        child.wait_with_output().unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_json(project: &Project) {
    project.write("agents.json", &json!({"version":"mas-agents/2","connections":{"model":{"kind":"mock","output":{"approved":true}}},"agents":[
        {"name":"echo","implementation":{"kind":"workflow","plan":"plan.json"}},
        {"name":"assessor","implementation":{"kind":"llm","connection":"model","model":"test-model","instructions":"instructions.md","output_schema":"schema.json"}}
    ]}).to_string());
    project.write("topology.json", r#"{"version":"mas-topology/1","edges":[["client","echo"],["echo","client"],["client","assessor"],["assessor","client"]]}"#);
    project.write("plan.json", r#"{"type":"value","output":{"$input":""}}"#);
    project.write("profile.json", r#"{"version":"mas-connections/1","connections":{"model":{"kind":"mock","output":{"approved":false}}}}"#);
    project.write("instructions.md", "Approve the input.");
    project.write("schema.json", r#"{"type":"object","properties":{"approved":{"type":"boolean"}},"required":["approved"],"additionalProperties":false}"#);
}

#[test]
fn toml_yaml_and_json_preserve_bundle_digest_and_resume_input() {
    let project = Project::new();
    configure_json(&project);
    let first = success(project.build(&["--connections", "profile.json"]));
    let bundle = first["bundle"].as_str().unwrap();
    let resolved: Value = serde_json::from_slice(&fs::read(bundle).unwrap()).unwrap();
    assert_eq!(
        resolved["agents"][0]["configuration"]["connection"]["output"]["approved"],
        false
    );
    let exact = r#"{"n":1234567890123456789012345678901234567890,"ok":true}"#;
    project.write("input.json", exact);
    let expected = success(project.run(bundle, "input.json", None, None));
    assert_eq!(
        expected["output"],
        serde_json::from_str::<Value>(exact).unwrap()
    );

    for file in ["agents.json", "topology.json", "plan.json"] {
        fs::remove_file(project.0.join(file)).unwrap();
    }
    project.write(
        "agents.toml",
        r#"
version = 'mas-agents/2'
[connections.model]
kind = 'mock'
output = { approved = true }
[[agents]]
name = 'echo'
[agents.implementation]
kind = 'workflow'
plan = 'plan.yaml'
[[agents]]
name = 'assessor'
[agents.implementation]
kind = 'llm'
connection = 'model'
model = 'test-model'
instructions = 'instructions.md'
output_schema = 'schema.json'
"#,
    );
    project.write("topology.yaml", "version: mas-topology/1\nedges:\n  - [client, echo]\n  - [echo, client]\n  - [client, assessor]\n  - [assessor, client]\n");
    project.write(
        "plan.yaml",
        "# The same resolved plan\ntype: value\noutput: {$input: ''}\n",
    );
    project.write("profile.toml", "version = 'mas-connections/1'\n[connections.model]\nkind = 'mock'\noutput = { approved = false }\n");
    let second = success(project.build(&["--connections", "profile.toml"]));
    assert_eq!(
        first["bundle"], second["bundle"],
        "format-only changes must preserve the content-addressed bundle"
    );
    project.write(
        "input.yaml",
        "n: 1234567890123456789012345678901234567890\nok: true\n",
    );
    project.write(
        "input.toml",
        "n = 1234567890123456789012345678901234567890\nok = true\n",
    );
    for file in ["input.yaml", "input.toml"] {
        assert_eq!(success(project.run(bundle, file, None, None)), expected);
    }
    assert_eq!(
        success(project.run(bundle, "-", None, Some(exact))),
        expected
    );
    assert_eq!(
        success(project.run(
            bundle,
            "-",
            Some("yaml"),
            Some("n: 1234567890123456789012345678901234567890\nok: true")
        )),
        expected
    );
    project.write("input.data", exact);
    assert_eq!(
        success(project.run(bundle, "input.data", Some("json"), None)),
        expected
    );
}

#[test]
fn ambiguous_defaults_require_explicit_selection_before_build() {
    let project = Project::new();
    configure_json(&project);
    project.write("agents.toml", "malformed");
    project.write("topology.yaml", "malformed");
    rejected(project.build(&[]), "ambiguous agents");
    assert!(!project.0.join(".mas").exists());
    rejected(
        project.build(&["--agents", "agents.json"]),
        "ambiguous topology",
    );
    success(project.build(&["--agents", "agents.json", "--topology", "topology.json"]));
}

#[test]
fn invalid_authoring_fails_before_build_or_state_creation() {
    let project = Project::new();
    configure_json(&project);
    project.write(
        "agents.json",
        "{\"version\":\"mas-agents/2\",\"version\":\"mas-agents/1\"}",
    );
    rejected(project.build(&[]), "DUPLICATE");
    assert!(!project.0.join(".mas").exists());
    project.write("input.yaml", "n: 1.5");
    rejected(
        project.run("missing-bundle.json", "input.yaml", None, None),
        "floating-point",
    );
    assert!(!project.0.join("state").exists());
    rejected(
        project.run("missing-bundle.json", "input.yaml", Some("xml"), None),
        "unsupported authoring format",
    );
}
