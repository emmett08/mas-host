//! Real Python worker framing, typed peer calls and uncertain-effect recovery.
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command, time::SystemTime};

fn host(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(args)
        .output()
        .unwrap()
}

fn success(args: &[&str]) -> Value {
    let result = host(args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn write(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn python_dataclasses_peer_calls_and_crashed_effect_reconcile_without_repetition() {
    let root = std::env::temp_dir().join(format!(
        "mas-python-recovery-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let project = root.to_str().unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::write(
        root.join("application.py"),
        r#"from dataclasses import dataclass
import json
import os
from mas_host import Context, EffectResult

@dataclass
class Payload:
    value: int
    receipt: str

@dataclass
class Checked:
    value: int

def check(ctx: Context, value: Payload) -> Checked:
    return Checked(value.value + 1)

def plan(ctx: Context, value: Payload) -> Checked:
    checked = ctx.request('check', 'checker', value, output_type=Checked)
    def publish(invocation_id):
        # Exclusive creation proves the recovered effect never executes twice.
        with open(value.receipt, 'x') as stream:
            json.dump({'output': {'value': checked.value}, 'invocation_id': invocation_id}, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os._exit(17)
    result = ctx.effect('receipt', {'path': value.receipt, 'value': checked.value}, publish)
    return Checked(result['value'])
"#,
    )
    .unwrap();
    fs::write(
        root.join("agents.toml"),
        r#"version = "mas-agents/3"
[[agents]]
name = "planner"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "plan"
[[agents]]
name = "checker"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "check"
"#,
    )
    .unwrap();
    fs::write(
        root.join("topology.yaml"),
        "version: mas-topology/1\nedges: [[client, planner], [planner, client], [planner, checker], [checker, planner]]\n",
    )
    .unwrap();
    let built = success(&[
        "build",
        "--project",
        project,
        "--sdk-root",
        sdk.to_str().unwrap(),
    ]);
    let bundle = built["bundle"].as_str().unwrap();
    let receipt = root.join("receipt.json");
    let input = root.join("input.json");
    write(
        &input,
        json!({"value":9007199254740993_u64,"receipt":receipt}),
    );
    let state = root.join("state");
    let invoke = [
        "run",
        "--bundle",
        bundle,
        "--agent",
        "planner",
        "--input",
        input.to_str().unwrap(),
        "--state",
        state.to_str().unwrap(),
        "--run",
        "recovery",
    ];
    assert!(!host(&invoke).status.success());
    let receipt_bytes = fs::read(&receipt).expect("Python wrote receipt before exiting");
    let observed: Value = serde_json::from_slice(&receipt_bytes).unwrap();
    let retry = host(&invoke);
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("uncertain"));
    let inspect = [
        "inspect",
        "--state",
        state.to_str().unwrap(),
        "--run",
        "recovery",
    ];
    let before = success(&inspect);
    assert_eq!(before["protocol"]["calls"].as_array().unwrap().len(), 1);
    let confirmation = root.join("confirmed.json");
    let evidence = root.join("evidence.json");
    write(&confirmation, observed["output"].clone());
    write(&evidence, json!({"receipt":receipt}));
    success(&[
        "resolve",
        "--state",
        state.to_str().unwrap(),
        "--run",
        "recovery",
        "--effect",
        observed["invocation_id"].as_str().unwrap(),
        "--output",
        confirmation.to_str().unwrap(),
        "--evidence",
        evidence.to_str().unwrap(),
    ]);
    // The retained executable contains the application and SDK source.
    fs::remove_file(root.join("application.py")).unwrap();
    assert_eq!(
        success(&invoke)["output"],
        json!({"value":9007199254740994_u64})
    );
    assert_eq!(fs::read(receipt).unwrap(), receipt_bytes);
    let after = success(&inspect);
    assert_eq!(after["protocol"]["calls"].as_array().unwrap().len(), 1);
    assert_eq!(after["protocol"]["calls"][0]["state"], "recorded");
    fs::remove_dir_all(root).unwrap();
}
