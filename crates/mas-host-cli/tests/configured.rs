//! Exercise configured workers through the real CLI, guardian and MAS store.
//! Provider commands are local deterministic fixtures, with no model credentials
//! or application-language compiler required.
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);

impl Project {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mas-configured-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("instructions.md"),
            "Return the requested assessment.",
        )
        .unwrap();
        write(&path.join("input.json"), &json!({"proposal": "release"}));
        write(&path.join("output.json"), &assessment_schema());
        Self(path)
    }

    fn build(&self) -> PathBuf {
        PathBuf::from(completed(self.build_output())["bundle"].as_str().unwrap())
    }

    fn build_output(&self) -> Output {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        host(&[
            "build",
            "--project",
            path(&self.0),
            "--sdk-root",
            path(&root),
        ])
    }

    fn run(&self, bundle: &Path, run: &str, actor: &str) -> Output {
        host(&[
            "run",
            "--bundle",
            path(bundle),
            "--input",
            path(&self.0.join("input.json")),
            "--state",
            path(&self.0.join("state")),
            "--run",
            run,
            "--agent",
            actor,
        ])
    }

    fn inspect(&self, run: &str) -> Value {
        success(&[
            "inspect",
            "--state",
            path(&self.0.join("state")),
            "--run",
            run,
        ])
    }

    fn messages(&self, run: &str) -> Vec<Value> {
        let connection = Connection::open_with_flags(
            self.0.join("state").join(run).join("protocol.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut query = connection
            .prepare("SELECT content FROM messages ORDER BY seq")
            .unwrap();
        query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| mas_protocol_codec::parse_json(&row.unwrap()).unwrap())
            .collect()
    }

    fn calls(&self) -> usize {
        fs::read(self.0.join("calls")).unwrap_or_default().len()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path(value: &Path) -> &str {
    value.to_str().unwrap()
}

fn write(file: &Path, value: &Value) {
    fs::write(file, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn host(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(arguments)
        .output()
        .unwrap()
}

fn completed(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn success(arguments: &[&str]) -> Value {
    completed(host(arguments))
}

fn rejected(output: Output, expected: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains(expected),
        "expected {expected:?}, got {diagnostic}"
    );
}

fn assessment_schema() -> Value {
    json!({"type":"object","properties":{"approved":{"type":"boolean"}},"required":["approved"],"additionalProperties":false})
}

fn llm(name: &str, connection: &str) -> Value {
    json!({"name":name,"operation":format!("{name}/1"),"output_schema":"output.json","implementation":{
        "kind":"llm","connection":connection,"model":"explicit-fixture-model",
        "instructions":"instructions.md","max_output_tokens":256
    }})
}

fn command(project: &Project, body: &str) -> Value {
    json!({"kind":"command","command":["/bin/sh","-c",
        format!("request=$(/bin/cat); printf x >> \"$1\"; {body}"),
        "mas-provider-fixture",project.0.join("calls")]})
}

fn envelope(output: Value) -> String {
    json!({"version":"mas-provider/1","status":"completed","output":output,"evidence":{}})
        .to_string()
}

fn configure_delegation(project: &Project, permitted: bool) {
    let request = envelope(
        json!({"decision":{"kind":"request","target":"peer","input_json":"{\"question\":\"approve\"}"}}),
    );
    let finish = envelope(json!({"decision":{"kind":"complete","output":{"approved":true}}}));
    // The second turn depends on a real peer result in history, rather than a
    // process-local counter that would disappear when the worker is restarted.
    let body = format!("case \"$request\" in *'\"history\":[]'*) printf '%s' '{request}' ;; *'\"result\":{{\"approved\":true}}'*) printf '%s' '{finish}' ;; *) exit 7 ;; esac");
    let mut planner = llm("planner", "reasoning");
    planner["implementation"]["max_turns"] = json!(2);
    planner["implementation"]["delegates"] = json!(["peer"]);
    write(
        &project.0.join("plan.json"),
        &json!({"type":"sequence","steps":[
            {"type":"request","key":"assess","target":"planner","input":{"$input":""}},
            {"type":"value","output":{"approved":{"$previous":"/approved"},"source":"workflow"}}
        ]}),
    );
    write(
        &project.0.join("agents.json"),
        &json!({"version":"mas-agents/3",
            "connections":{"reasoning":command(project,&body),"peer_model":{"kind":"mock","output":{"approved":true}}},
            "agents":[
                {"name":"workflow","operation":"release/1","implementation":{"kind":"workflow","plan":"plan.json"}},
                planner,llm("peer","peer_model")
            ]
        }),
    );
    let mut edges = vec![
        json!(["client", "workflow"]),
        json!(["workflow", "client"]),
        json!(["workflow", "planner"]),
        json!(["planner", "workflow"]),
    ];
    if permitted {
        edges.extend([json!(["planner", "peer"]), json!(["peer", "planner"])]);
    }
    write(
        &project.0.join("topology.json"),
        &json!({"version":"mas-topology/1","edges":edges}),
    );
}

#[test]
fn reserved_ingress_cannot_be_a_workflow_or_model_delegate() {
    let project = Project::new("ingress-target");
    configure_delegation(&project, true);
    let agents_path = project.0.join("agents.json");
    let mut agents: Value = serde_json::from_slice(&fs::read(&agents_path).unwrap()).unwrap();
    agents["agents"][1]["implementation"]["delegates"] = json!(["client"]);
    write(&agents_path, &agents);
    rejected(project.build_output(), "invalid LLM delegate");

    configure_delegation(&project, true);
    write(
        &project.0.join("plan.json"),
        &json!({"type":"request","key":"ingress","target":"client","input":{}}),
    );
    rejected(project.build_output(), "unknown workflow target client");
    assert_eq!(project.calls(), 0);
    assert!(!project.0.join(".mas/build/bundle.json").exists());
}

fn configure_single_llm(project: &Project) {
    write(
        &project.0.join("agents.json"),
        &json!({"version":"mas-agents/3","connections":{"fixture":{"kind":"mock","output":{"approved":true}}},"agents":[llm("assessor","fixture")]}),
    );
    write(
        &project.0.join("topology.json"),
        &json!({"version":"mas-topology/1","edges":[["client","assessor"],["assessor","client"]]}),
    );
}

#[test]
fn oversized_rebuild_preserves_the_previous_published_bundle() {
    let project = Project::new("bundle-bound");
    configure_single_llm(&project);
    // This document fits as compact JSON but exceeds the bound when indented.
    let properties: serde_json::Map<_, _> = (0..300)
        .map(|n| {
            (
                format!("field_{n}"),
                json!({"type":"string","description":"ordinary field value"}),
            )
        })
        .collect();
    write(
        &project.0.join("output.json"),
        &json!({"type":"object","properties":properties}),
    );
    let original = project.build();
    let published = project.0.join(".mas/build/bundle.json");
    let bytes = fs::read(&published).unwrap();
    let bundle: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(bytes.len() <= 65_536);
    assert!(serde_json::to_vec_pretty(&bundle).unwrap().len() > 65_536);

    // Each source document remains in bounds; embedding the schema twice does not.
    write(
        &project.0.join("output.json"),
        &json!({"type":"object","description":"x".repeat(33_000)}),
    );
    rejected(project.build_output(), "generated bundle: SIZE");
    assert_eq!(fs::read(&published).unwrap(), bytes);
    assert_eq!(fs::read(&original).unwrap(), bytes);
    assert_eq!(
        fs::read_dir(project.0.join(".mas/build/bundles"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        completed(project.run(&original, "retained", "assessor"))["output"],
        json!({"approved":true})
    );
}

#[test]
fn inference_keeps_large_task_input_without_repeating_immutable_configuration() {
    let project = Project::new("descriptor-bound");
    configure_single_llm(&project);
    let mut schema = assessment_schema();
    schema["description"] = json!("schema documentation ".repeat(1_000));
    write(&project.0.join("output.json"), &schema);
    let input = json!({"payload":"x".repeat(28_000)});
    write(&project.0.join("input.json"), &input);
    let bundle_path = project.build();
    let first = completed(project.run(&bundle_path, "large", "assessor"));
    assert_eq!(first["output"], json!({"approved":true}));
    let before = project.inspect("large");
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
    assert_eq!(
        completed(project.run(&bundle_path, "large", "assessor")),
        first
    );
    assert_eq!(project.inspect("large"), before);

    let connection = Connection::open_with_flags(
        project.0.join("state/large/execution.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let descriptor: String = connection
        .query_row("SELECT input FROM steps WHERE kind='effect'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let descriptor: Value = serde_json::from_str(&descriptor).unwrap();
    let bundle: Value = serde_json::from_slice(&fs::read(bundle_path).unwrap()).unwrap();
    let hash: String = Sha256::digest(bundle["agents"][0]["configuration"].to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        descriptor,
        json!({"format":"mas-inference/2","configuration_sha256":hash,"input":input})
    );
}

#[test]
fn configured_workflow_runs_llm_peer_requests_through_mas_and_replays_without_calls() {
    let project = Project::new("delegation");
    configure_delegation(&project, true);
    let bundle = project.build();
    let first = completed(project.run(&bundle, "delegated", "workflow"));
    assert_eq!(
        first["output"],
        json!({"approved":true,"source":"workflow"})
    );
    assert_eq!(
        project.calls(),
        2,
        "exactly two externally observed reasoning turns"
    );
    let before = project.inspect("delegated");
    assert_eq!(
        before["protocol"]["calls"].as_array().unwrap().len(),
        3,
        "two reasoning turns and one peer inference"
    );
    assert!(before["protocol"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .all(|call| call["state"] == "recorded"));
    let messages = project.messages("delegated");
    let requests: Vec<_> = messages
        .iter()
        .filter(|m| m["kind"] == "Request")
        .map(|m| (&m["sender"], &m["recipient"], &m["payload"]["operation"]))
        .collect();
    assert_eq!(
        requests,
        vec![
            (&json!("client"), &json!("workflow"), &json!("release/1")),
            (&json!("workflow"), &json!("planner"), &json!("planner/1")),
            (&json!("planner"), &json!("peer"), &json!("peer/1")),
        ]
    );
    assert_eq!(messages.iter().filter(|m| m["kind"] == "Result").count(), 3);
    assert_eq!(
        completed(project.run(&bundle, "delegated", "workflow")),
        first
    );
    assert_eq!(
        project.calls(),
        2,
        "replay must not dispatch another provider command"
    );
    let after = project.inspect("delegated");
    assert_eq!(after["steps"], before["steps"]);
    assert_eq!(after["protocol"]["calls"], before["protocol"]["calls"]);
    assert_eq!(project.messages("delegated"), messages);
}

#[test]
fn llm_delegates_to_an_ordinary_cli_with_the_same_admission_and_replay() {
    let project = Project::new("cli-delegation");
    configure_delegation(&project, true);
    let agents_path = project.0.join("agents.json");
    let mut agents: Value = serde_json::from_slice(&fs::read(&agents_path).unwrap()).unwrap();
    agents["agents"][2] = json!({"name":"peer","operation":"check_policy/1","output_schema":"output.json",
        "implementation":{"kind":"command","executable":"/usr/bin/printf",
            "arguments":[{"literal":"%s"},{"literal":"{\"approved\":true}"}]}});
    write(&agents_path, &agents);
    let bundle = project.build();
    let first = completed(project.run(&bundle, "cli", "workflow"));
    assert_eq!(
        first["output"],
        json!({"approved":true,"source":"workflow"})
    );
    let before = project.inspect("cli");
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 3);
    assert_eq!(completed(project.run(&bundle, "cli", "workflow")), first);
    assert_eq!(project.inspect("cli"), before);
    let retained: Value = serde_json::from_slice(&fs::read(bundle).unwrap()).unwrap();
    let planner = retained["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "planner")
        .unwrap();
    assert_eq!(
        planner["configuration"]["delegate_contracts"][0]["operation"],
        "check_policy/1"
    );
}

#[test]
fn required_delegations_gate_provider_schema_until_results_are_recorded_and_replay() {
    let project = Project::new("required-delegates");
    configure_delegation(&project, true);
    let agents_path = project.0.join("agents.json");
    let mut agents: Value = serde_json::from_slice(&fs::read(&agents_path).unwrap()).unwrap();
    agents["agents"][1]["implementation"]["max_turns"] = json!(3);
    agents["agents"][1]["implementation"]["min_delegate_calls"] = json!(2);
    agents["connections"]["reasoning"] = json!({"kind":"command","command":["/usr/bin/python3","-c",r#"
import json, pathlib, sys
request = json.load(sys.stdin)
with pathlib.Path(sys.argv[1]).open('a') as calls:
    calls.write('x')
history = request['input']['history']
kinds = {branch['properties']['kind']['enum'][0] for branch in request['output_schema']['properties']['decision']['anyOf']}
assert kinds == ({'request'} if len(history) < 2 else {'request', 'complete'}), kinds
decision = ({'kind':'request','target':'peer','input_json':'{"question":"approve"}'} if len(history) < 2
            else {'kind':'complete','output':{'approved':True}})
print(json.dumps({'version':'mas-provider/1','status':'completed','output':{'decision':decision},'evidence':{}}))
"#,project.0.join("calls")]});
    write(&agents_path, &agents);
    let bundle = project.build();
    let result = completed(project.run(&bundle, "required", "workflow"));
    assert_eq!(
        result["output"],
        json!({"approved":true,"source":"workflow"})
    );
    assert_eq!(project.calls(), 3);
    let before = project.inspect("required");
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 5);
    let messages = project.messages("required");
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["kind"] == "Request" && message["recipient"] == "peer")
            .count(),
        2
    );
    assert_eq!(
        completed(project.run(&bundle, "required", "workflow")),
        result
    );
    assert_eq!(project.calls(), 3);
    assert_eq!(project.inspect("required"), before);
    assert_eq!(project.messages("required"), messages);
}

#[test]
fn early_completion_is_allowed_by_default_but_required_delegation_failure_is_retained() {
    for minimum in [None, Some(0), Some(1)] {
        let project = Project::new("early-completion");
        configure_delegation(&project, true);
        let agents_path = project.0.join("agents.json");
        let mut agents: Value = serde_json::from_slice(&fs::read(&agents_path).unwrap()).unwrap();
        if let Some(minimum) = minimum {
            agents["agents"][1]["implementation"]["min_delegate_calls"] = json!(minimum);
        }
        let finish = envelope(json!({"decision":{"kind":"complete","output":{"approved":true}}}));
        agents["connections"]["reasoning"] = command(&project, &format!("printf '%s' '{finish}'"));
        write(&agents_path, &agents);
        let bundle = project.build();
        let run = || project.run(&bundle, "early", "workflow");
        if minimum == Some(1) {
            rejected(run(), "completed / invalid");
        } else {
            assert_eq!(
                completed(run())["output"],
                json!({"approved":true,"source":"workflow"})
            );
        }
        let before = project.inspect("early");
        let effects: Vec<_> = before["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|step| step["kind"] == "effect")
            .collect();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0]["state"], "completed");
        assert_eq!(effects[0]["output"]["certainty"], "completed");
        assert_eq!(
            effects[0]["output"]["output_status"],
            if minimum == Some(1) {
                "invalid"
            } else {
                "valid"
            }
        );
        assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
        assert_eq!(before["protocol"]["calls"][0]["state"], "recorded");
        let messages = project.messages("early");
        assert!(!messages
            .iter()
            .any(|message| message["recipient"] == "peer"));
        if minimum == Some(1) {
            rejected(run(), "completed / invalid");
        } else {
            completed(run());
        }
        assert_eq!(
            project.calls(),
            1,
            "replay must reuse the known provider outcome"
        );
        assert_eq!(project.inspect("early"), before);
        assert_eq!(project.messages("early"), messages);
    }
}

#[test]
fn known_refusal_and_invalid_output_are_recorded_while_unknown_execution_blocks_replay() {
    let refused = json!({"version":"mas-provider/1","status":"unusable","evidence":{"output_status":"refused"}}).to_string();
    let invalid = envelope(json!({"approved":"incorrect type"}));
    for (label, body, status) in [
        (
            "refused",
            format!("printf '%s' '{refused}'"),
            Some("refused"),
        ),
        (
            "invalid",
            format!("printf '%s' '{invalid}'"),
            Some("invalid"),
        ),
        ("unknown", "exit 3".into(), None),
    ] {
        let project = Project::new(label);
        write(
            &project.0.join("agents.json"),
            &json!({"version":"mas-agents/3","connections":{"fixture":command(&project,&body)},"agents":[llm("assessor","fixture")]}),
        );
        write(
            &project.0.join("topology.json"),
            &json!({"version":"mas-topology/1","edges":[["client","assessor"],["assessor","client"]]}),
        );
        let bundle = project.build();
        rejected(
            project.run(&bundle, label, "assessor"),
            status.unwrap_or("unknown"),
        );
        let before = project.inspect(label);
        let effects: Vec<_> = before["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|step| step["kind"] == "effect")
            .collect();
        assert_eq!(effects.len(), 1);
        assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
        if let Some(status) = status {
            assert_eq!(effects[0]["state"], "completed");
            assert_eq!(effects[0]["output"]["certainty"], "completed");
            assert_eq!(effects[0]["output"]["output_status"], status);
            assert_eq!(before["protocol"]["calls"][0]["state"], "recorded");
            rejected(project.run(&bundle, label, "assessor"), status);
        } else {
            assert_eq!(effects[0]["state"], "unknown");
            assert_eq!(before["protocol"]["calls"][0]["state"], "reserved");
            rejected(project.run(&bundle, label, "assessor"), "uncertain");
        }
        assert_eq!(
            project.calls(),
            1,
            "known and unknown failures must both avoid automatic resubmission"
        );
        assert_eq!(
            project.inspect(label)["protocol"]["calls"],
            before["protocol"]["calls"]
        );
    }
}

#[test]
fn prompt_and_schema_drift_cannot_resume_the_original_configured_run() {
    let project = Project::new("identity");
    write(
        &project.0.join("agents.json"),
        &json!({"version":"mas-agents/3","connections":{"fixture":{"kind":"mock","output":{"approved":true}}},"agents":[llm("assessor","fixture")]}),
    );
    write(
        &project.0.join("topology.json"),
        &json!({"version":"mas-topology/1","edges":[["client","assessor"],["assessor","client"]]}),
    );
    let original = project.build();
    let first = completed(project.run(&original, "identity", "assessor"));
    fs::write(
        project.0.join("instructions.md"),
        "Use a revised assessment instruction.",
    )
    .unwrap();
    let changed_prompt = project.build();
    assert_ne!(original, changed_prompt);
    rejected(
        project.run(&changed_prompt, "identity", "assessor"),
        "identity",
    );
    fs::write(
        project.0.join("instructions.md"),
        "Return the requested assessment.",
    )
    .unwrap();
    let mut schema = assessment_schema();
    schema["properties"]["reason"] = json!({"type":"string"});
    schema["required"] = json!(["approved", "reason"]);
    write(&project.0.join("output.json"), &schema);
    let changed_schema = project.build();
    assert_ne!(original, changed_schema);
    rejected(
        project.run(&changed_schema, "identity", "assessor"),
        "identity",
    );
    assert_eq!(
        completed(project.run(&original, "identity", "assessor")),
        first,
        "the retained original bundle remains usable"
    );
    assert_eq!(
        project.inspect("identity")["protocol"]["calls"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn model_delegation_cannot_bypass_the_frozen_topology() {
    let project = Project::new("denied");
    configure_delegation(&project, false);
    let bundle = project.build();
    rejected(
        project.run(&bundle, "denied", "workflow"),
        "forbidden by frozen topology",
    );
    let evidence = project.inspect("denied");
    assert_eq!(
        project.calls(),
        1,
        "no second reasoning turn follows denied peer admission"
    );
    assert_eq!(
        evidence["protocol"]["calls"].as_array().unwrap().len(),
        1,
        "peer provider must never run"
    );
    assert!(!project
        .messages("denied")
        .iter()
        .any(|m| m["recipient"] == "peer"));
    assert!(evidence["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["target"] == "peer" && step["state"] == "rejected"));
}

#[test]
fn final_turn_does_not_spend_a_peer_call_that_cannot_contribute_to_completion() {
    let project = Project::new("turn-budget");
    configure_delegation(&project, true);
    let agents_path = project.0.join("agents.json");
    let mut agents: Value = serde_json::from_slice(&fs::read(&agents_path).unwrap()).unwrap();
    agents["agents"][1]["implementation"]["max_turns"] = json!(1);
    write(&agents_path, &agents);
    let bundle = project.build();
    rejected(project.run(&bundle, "last-turn", "workflow"), "turn budget");
    let before = project.inspect("last-turn");
    assert_eq!(project.calls(), 1);
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
    assert_eq!(before["protocol"]["calls"][0]["state"], "recorded");
    assert!(!before["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["target"] == "peer"));
    assert!(!project
        .messages("last-turn")
        .iter()
        .any(|message| message["recipient"] == "peer"));
    rejected(project.run(&bundle, "last-turn", "workflow"), "turn budget");
    assert_eq!(
        project.calls(),
        1,
        "replay must reuse the known failed turn"
    );
    let after = project.inspect("last-turn");
    assert_eq!(after["steps"], before["steps"]);
    assert_eq!(after["protocol"]["calls"], before["protocol"]["calls"]);
}
