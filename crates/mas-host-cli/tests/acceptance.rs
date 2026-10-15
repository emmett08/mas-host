//! Real compiler/worker acceptance: code-only applications, routing and recovery.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn host(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(args)
        .output()
        .unwrap()
}
fn success(args: &[&str]) -> Value {
    let output = host(args);
    assert!(
        output.status.success(),
        "command {args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI returns one JSON document")
}
fn failure(args: &[&str], expected: &str) {
    let output = host(args);
    assert!(!output.status.success(), "unexpected success: {args:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "expected {expected}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn strpath(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn build(project: &Path) -> PathBuf {
    PathBuf::from(
        success(&[
            "build",
            "--project",
            strpath(project),
            "--sdk-root",
            strpath(&root()),
        ])["bundle"]
            .as_str()
            .unwrap(),
    )
}
fn invoke(bundle: &Path, input: &Path, state: &Path, run: &str, agent: &str) -> Value {
    success(&[
        "run",
        "--bundle",
        strpath(bundle),
        "--input",
        strpath(input),
        "--state",
        strpath(state),
        "--run",
        run,
        "--agent",
        agent,
    ])
}
fn inspect(state: &Path, run: &str) -> Value {
    success(&["inspect", "--state", strpath(state), "--run", run])
}

#[test]
#[ignore = "requires Go, C++ and Rust toolchains; run make acceptance"]
fn code_only_apps_mixed_routes_and_real_effect_recovery() {
    let scratch = std::env::temp_dir().join(format!(
        "mas-host-acceptance-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&scratch).unwrap();
    let state = scratch.join("state");
    let input = scratch.join("input.json");
    let go = root().join("examples/go-release");
    let go_bundle = build(&go);
    let proposal = json!({"release":"v1","replicas":1,"demand":250,"per_replica":100,"critical_vulnerabilities":0,"max_replicas":5});
    write(&input, &proposal);
    let first = invoke(&go_bundle, &input, &state, "healthy", "planner");
    assert_eq!(first["output"]["ready"], true);
    assert_eq!(first["output"]["attempts"], 3);
    let before = inspect(&state, "healthy");
    assert_eq!(
        invoke(&go_bundle, &input, &state, "healthy", "planner"),
        first
    );
    assert_eq!(
        inspect(&state, "healthy")["protocol"]["messages"],
        before["protocol"]["messages"]
    );
    let mut bad = proposal.clone();
    bad["critical_vulnerabilities"] = json!(1);
    write(&input, &bad);
    assert_eq!(
        invoke(&go_bundle, &input, &state, "security", "planner")["output"]["ready"],
        false
    );
    failure(
        &[
            "run",
            "--bundle",
            strpath(&go_bundle),
            "--input",
            strpath(&input),
            "--state",
            strpath(&state),
            "--run",
            "healthy",
            "--agent",
            "planner",
        ],
        "different",
    );

    let receipt = scratch.join("receipt.json");
    let mut crash = proposal.clone();
    crash["receipt_path"] = json!(receipt);
    crash["crash_after_receipt"] = json!(true);
    write(&input, &crash);
    let crash_args = [
        "run",
        "--bundle",
        strpath(&go_bundle),
        "--input",
        strpath(&input),
        "--state",
        strpath(&state),
        "--run",
        "recovery",
        "--agent",
        "planner",
    ];
    let crashed = host(&crash_args);
    assert!(!crashed.status.success());
    let receipt_bytes = fs::read(&receipt).expect("worker published durable receipt before exit");
    let observed: Value = serde_json::from_slice(&receipt_bytes).unwrap();
    failure(&crash_args, "uncertain");
    let uncertain = inspect(&state, "recovery");
    assert_eq!(uncertain["protocol"]["calls"].as_array().unwrap().len(), 1);
    let completed: Vec<_> = uncertain["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "request" && s["state"] == "completed")
        .map(|s| s["id"].clone())
        .collect();
    assert!(!completed.is_empty());
    let effect = uncertain["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "effect")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(observed["invocation_id"], effect);
    let output_file = scratch.join("confirmed-output.json");
    write(&output_file, &observed["output"]);
    let evidence = scratch.join("evidence.json");
    write(&evidence, &json!({}));
    let resolve_args = [
        "resolve",
        "--state",
        strpath(&state),
        "--run",
        "recovery",
        "--effect",
        effect,
        "--output",
        strpath(&output_file),
        "--evidence",
        strpath(&evidence),
    ];
    failure(&resolve_args, "non-empty");
    write(
        &evidence,
        &json!({"receipt_path":receipt,"invocation_id":effect}),
    );
    success(&resolve_args);
    assert_eq!(
        invoke(&go_bundle, &input, &state, "recovery", "planner")["output"],
        observed["output"]
    );
    assert_eq!(fs::read(&receipt).unwrap(), receipt_bytes);
    let recovered = inspect(&state, "recovery");
    assert_eq!(recovered["protocol"]["calls"].as_array().unwrap().len(), 1);
    assert_eq!(recovered["protocol"]["calls"][0]["state"], "recorded");
    for id in &completed {
        assert!(recovered["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == *id && s["state"] == "completed"));
    }
    println!(
        "recovery: preserved {} completed child steps, one protocol call, one unchanged receipt",
        completed.len()
    );

    let mut altered: Value = serde_json::from_slice(&fs::read(&go_bundle).unwrap()).unwrap();
    altered["topology"]["max_calls"] = json!(9);
    let altered_bundle = scratch.join("altered-bundle.json");
    write(&altered_bundle, &altered);
    failure(
        &[
            "run",
            "--bundle",
            strpath(&altered_bundle),
            "--input",
            strpath(&input),
            "--state",
            strpath(&state),
            "--run",
            "recovery",
            "--agent",
            "planner",
        ],
        "identity",
    );

    let invoice = json!({"amount_minor":12500,"limit_minor":20000});
    write(&input, &invoice);
    for language in ["rust", "cpp"] {
        let project = root().join(format!("examples/{language}-invoice"));
        let bundle = build(&project);
        let result = invoke(&bundle, &input, &state, language, "invoice");
        assert_eq!(
            result["output"],
            json!({"approved":true,"amount_minor":12500,"limit_minor":20000})
        );
    }
    mixed_application(&scratch, &state, &input);
    fs::remove_dir_all(scratch).unwrap();
}

fn mixed_application(scratch: &Path, state: &Path, input: &Path) {
    let project = scratch.join("mixed");
    fs::create_dir_all(project.join("native")).unwrap();
    for (from, to) in [
        ("examples/rust-invoice/invoice.rs", "native/invoice.rs"),
        ("examples/cpp-invoice/invoice.cpp", "native/invoice.cpp"),
    ] {
        fs::copy(root().join(from), project.join(to)).unwrap();
    }
    fs::write(project.join("go.mod"),format!("module example.com/mixed\n\ngo 1.24.0\n\nrequire github.com/emmett08/mas-host/sdk/go v0.0.0\nreplace github.com/emmett08/mas-host/sdk/go => {}\n",root().join("sdk/go").display())).unwrap();
    fs::write(project.join("workflow.go"),r#"package mixed
import (
 "encoding/json"
 mas "github.com/emmett08/mas-host/sdk/go"
)
type Invoice struct { Amount uint64 `json:"amount_minor"`; Limit uint64 `json:"limit_minor"` }
func Compare(ctx *mas.Context, input Invoice) (map[string]json.RawMessage,error) {
 values,err:=ctx.Parallel([]mas.RequestSpec{{Key:"rust_check",Target:"rust_invoice",Input:input},{Key:"cpp_check",Target:"cpp_invoice",Input:input}})
 if err!=nil { return nil,err }
 return map[string]json.RawMessage{"rust":values[0],"cpp":values[1]},nil
}
"#).unwrap();
    write(
        &project.join("agents.json"),
        &json!({"version":"mas-agents/1","agents":[
        {"name":"planner","language":"go","source":".","handler":"Compare"},
        {"name":"rust_invoice","language":"rust","source":"native/invoice.rs","handler":"assess"},
        {"name":"cpp_invoice","language":"cpp","source":"native/invoice.cpp","handler":"Assess"}]}),
    );
    let mut topology = json!({"version":"mas-topology/1","edges":[["client","planner"],["planner","client"],["planner","rust_invoice"],["rust_invoice","planner"],["planner","cpp_invoice"],["cpp_invoice","planner"]]});
    write(&project.join("topology.json"), &topology);
    let bundle = build(&project);
    let output = invoke(&bundle, input, state, "mixed", "planner");
    assert_eq!(output["output"]["rust"], output["output"]["cpp"]);
    assert_eq!(output["output"]["rust"]["approved"], true);
    topology["edges"].as_array_mut().unwrap().truncate(4);
    write(&project.join("topology.json"), &topology);
    let denied = build(&project);
    failure(
        &[
            "run",
            "--bundle",
            strpath(&denied),
            "--input",
            strpath(input),
            "--state",
            strpath(state),
            "--run",
            "denied",
            "--agent",
            "planner",
        ],
        "route forbidden",
    );
    println!("mixed language: Go coordinator joined Rust and C++; denied route rejected by MAS");
}
