//! Ordinary CLI integration: argv fidelity, retained code, admission and recovery.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-command-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("work")).unwrap();
        let project = Self(root);
        project.write(
            "tool.py",
            include_str!("../../../tests/fixtures/command/tool.py"),
        );
        project.json("schema.json", &json!({"type":"object"}));
        project.json(
            "topology.json",
            &json!({"version":"mas-topology/1","edges":[["client","tool"],["tool","client"]]}),
        );
        project.configure(json!({}));
        project
    }
    fn write(&self, name: &str, data: &str) {
        fs::write(self.0.join(name), data).unwrap();
    }
    fn json(&self, name: &str, data: &Value) {
        self.write(name, &data.to_string());
    }
    fn configure(&self, overrides: Value) {
        let mut implementation = json!({"kind":"command","executable":"/usr/bin/python3",
            "arguments":[{"file":"tool.py"},{"literal":"--service"},{"input":"/service"},{"literal":"--invocation"},{"invocation_id":true}],
            "stdin":"json","timeout_seconds":5});
        for (key, value) in overrides.as_object().unwrap() {
            implementation[key] = value.clone();
        }
        self.json("agents.json", &json!({"version":"mas-agents/3","agents":[{
            "name":"tool","operation":"check/1","implementation":implementation,
            "output_schema":"schema.json","environment":["MAS_COMMAND_DECLARED"],"working_directory":"work"
        }],"limits":{"wall_seconds":60,"max_calls":8}}));
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .current_dir(&self.0)
            .args(args)
            .env("MAS_COMMAND_DECLARED", "scoped")
            .env("MAS_COMMAND_UNDECLARED", "ambient")
            .output()
            .unwrap()
    }
    fn build(&self) -> String {
        success(
            self.command(&[
                "build",
                "--project",
                ".",
                "--sdk-root",
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .to_str()
                    .unwrap(),
            ]),
        )["bundle"]
            .as_str()
            .unwrap()
            .into()
    }
    fn run(&self, bundle: &str, id: &str, input: &Value) -> Output {
        self.json("input.json", input);
        self.command(&[
            "run",
            "--bundle",
            bundle,
            "--agent",
            "tool",
            "--input",
            "input.json",
            "--state",
            "state",
            "--run",
            id,
        ])
    }
    fn inspect(&self, id: &str) -> Value {
        success(self.command(&["inspect", "--state", "state", "--run", id]))
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
fn failure(output: Output, message: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn argv_stdin_environment_and_directory_are_explicit_and_replay_is_cached() {
    let project = Project::new();
    let bundle = project.build();
    let input =
        json!({"service":"billing ; $(touch injected) with spaces","n":9007199254740993_u64});
    let first = success(project.run(&bundle, "exact", &input));
    assert_eq!(first["output"]["service"], input["service"]);
    assert_eq!(first["output"]["n"], input["n"]);
    assert_eq!(first["output"]["declared"], "scoped");
    assert_eq!(first["output"]["ambient"], Value::Null);
    assert_eq!(first["output"]["channel"], Value::Null);
    assert_eq!(first["output"]["cwd"], json!(project.0.join("work")));
    assert!(!project.0.join("work/injected").exists());
    let before = project.inspect("exact");
    assert_eq!(success(project.run(&bundle, "exact", &input)), first);
    assert_eq!(project.inspect("exact"), before);
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
}

#[test]
fn retained_script_survives_source_change_and_new_bundle_cannot_resume() {
    let project = Project::new();
    let old = project.build();
    let input = json!({"service":"billing","n":7});
    let first = success(project.run(&old, "original", &input));
    let source = fs::read_to_string(project.0.join("tool.py")).unwrap();
    project.write("tool.py", &source.replace("REVISION = 1", "REVISION = 2"));
    let new = project.build();
    assert_ne!(old, new);
    failure(project.run(&new, "original", &input), "identity differs");
    assert_eq!(
        success(project.run(&old, "old_fresh", &input))["output"]["revision"],
        first["output"]["revision"]
    );
    assert_eq!(
        success(project.run(&new, "new_fresh", &input))["output"]["revision"],
        2
    );
    fs::remove_file(project.0.join("tool.py")).unwrap();
    assert_eq!(
        success(project.run(&old, "after_removal", &input))["output"]["revision"],
        1
    );
}

#[test]
fn interrupted_external_write_requires_resolution_and_is_never_repeated() {
    let project = Project::new();
    let bundle = project.build();
    let receipt = project.0.join("receipt.json");
    let input = json!({"service":"billing","receipt":receipt,"mode":"crash"});
    failure(project.run(&bundle, "crash", &input), "uncertain");
    let bytes = fs::read(&receipt).unwrap();
    let published: Value = serde_json::from_slice(&bytes).unwrap();
    let inspection = project.inspect("crash");
    let effect = inspection["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "effect")
        .unwrap();
    assert_eq!(published["invocation_id"], effect["id"]);
    failure(project.run(&bundle, "crash", &input), "uncertain");
    assert_eq!(fs::read(&receipt).unwrap(), bytes);
    project.json(
        "confirmed.json",
        &json!({"status":"completed","output":published["output"]}),
    );
    project.json(
        "evidence.json",
        &json!({"receipt":receipt,"invocation_id":effect["id"]}),
    );
    success(project.command(&[
        "resolve",
        "--state",
        "state",
        "--run",
        "crash",
        "--effect",
        effect["id"].as_str().unwrap(),
        "--output",
        "confirmed.json",
        "--evidence",
        "evidence.json",
    ]));
    assert_eq!(
        success(project.run(&bundle, "crash", &input))["output"],
        published["output"]
    );
    assert_eq!(fs::read(&receipt).unwrap(), bytes);
    assert_eq!(
        project.inspect("crash")["protocol"]["calls"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn invalid_output_is_a_recorded_observation_and_bad_arguments_never_dispatch() {
    let project = Project::new();
    let bundle = project.build();
    failure(
        project.run(&bundle, "bad_argument", &json!({"service":[]})),
        "argument must",
    );
    assert!(project.inspect("bad_argument")["protocol"]["calls"]
        .as_array()
        .unwrap()
        .is_empty());
    for mode in ["float", "duplicate", "text"] {
        let receipt = project.0.join(format!("{mode}.json"));
        let input = json!({"service":"billing","mode":mode,"receipt":receipt});
        failure(
            project.run(&bundle, mode, &input),
            "invalid operation output",
        );
        let bytes = fs::read(&receipt).unwrap();
        failure(
            project.run(&bundle, mode, &input),
            "invalid operation output",
        );
        assert_eq!(fs::read(&receipt).unwrap(), bytes);
        assert_eq!(
            project.inspect(mode)["protocol"]["calls"][0]["state"],
            "recorded"
        );
    }
    project.configure(json!({"stdout":"text"}));
    let bundle = project.build();
    assert_eq!(
        success(project.run(
            &bundle,
            "plain",
            &json!({"service":"billing","mode":"text"})
        ))["output"],
        json!({"stdout":"plain output\n"})
    );
}

#[test]
fn timeout_and_output_overflow_remain_uncertain_without_retry() {
    let project = Project::new();
    project.configure(json!({"timeout_seconds":1,"max_output_bytes":128}));
    let bundle = project.build();
    for mode in ["sleep", "overflow"] {
        let child_pid = project.0.join("child.pid");
        let input = json!({"service":"billing","mode":mode,"child_pid":child_pid});
        failure(project.run(&bundle, mode, &input), "uncertain");
        failure(project.run(&bundle, mode, &input), "uncertain");
        assert_eq!(
            project.inspect(mode)["protocol"]["calls"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        if mode == "sleep" {
            let pid = fs::read_to_string(child_pid).unwrap();
            assert!(
                !Path::new(&format!("/proc/{}", pid.trim())).exists(),
                "guardian must reap the CLI's descendant"
            );
        }
    }
}

#[test]
fn absolute_executable_drift_blocks_dispatch_and_failed_spawn_is_known() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    project.write("external", "#!/bin/sh\nprintf '%s' '{\"ok\":true}'\n");
    let executable = project.0.join("external");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    project.configure(json!({"executable":executable,"arguments":[],"stdin":"none"}));
    let bundle = project.build();
    project.write("external", "#!/bin/sh\nprintf '%s' '{\"ok\":false}'\n");
    failure(
        project.run(&bundle, "drift", &json!({})),
        "asset digest differs",
    );
    assert!(project.inspect("drift")["protocol"]["calls"]
        .as_array()
        .unwrap()
        .is_empty());

    project.write("external", "#!/missing/command-interpreter\n");
    let bundle = project.build();
    failure(
        project.run(&bundle, "not_started", &json!({})),
        "not started",
    );
    assert_eq!(
        project.inspect("not_started")["protocol"]["calls"][0]["state"],
        "recorded"
    );
}

#[test]
fn operation_schemas_apply_to_workflows_and_commands_before_dispatch_and_completion() {
    let project = Project::new();
    project.json(
        "input-schema.json",
        &json!({"type":"object","properties":{"service":{"type":"string"}},"required":["service"]}),
    );
    let mut agents: Value =
        serde_json::from_slice(&fs::read(project.0.join("agents.json")).unwrap()).unwrap();
    agents["agents"][0]["input_schema"] = json!("input-schema.json");
    project.json("agents.json", &agents);
    let bundle = project.build();
    assert!(!project
        .run(&bundle, "invalid_schema", &json!({"service":8}))
        .status
        .success());
    assert!(project.inspect("invalid_schema")["protocol"]["calls"]
        .as_array()
        .unwrap()
        .is_empty());
    project.json(
        "schema.json",
        &json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}),
    );
    project.json(
        "plan.json",
        &json!({"type":"value","output":{"ok":"wrong type"}}),
    );
    agents["agents"][0]["implementation"] = json!({"kind":"workflow","plan":"plan.json"});
    project.json("agents.json", &agents);
    let bundle = project.build();
    assert!(!project
        .run(
            &bundle,
            "invalid_workflow_output",
            &json!({"service":"billing"})
        )
        .status
        .success());
    assert!(!project.inspect("invalid_workflow_output")["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["state"] == "completed"));
}
