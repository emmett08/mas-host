//! Exercise disclosure, revision and replay through the real CLI and workers.
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn host(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(args)
        .output()
        .unwrap()
}
fn ok(args: &[&str]) -> Value {
    let r = host(args);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    serde_json::from_slice(&r.stdout).unwrap()
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn revision_replays_immutable_records_and_filters_all_public_channels() {
    let root = std::env::temp_dir().join(format!(
        "mas-revision-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("app.py"),r#"from mas_host import Context, EffectResult

def draft(ctx: Context, value: dict) -> dict:
    return {'safe': False, 'text': value['sources']['incident']['text']}

def revise(ctx: Context, value: dict) -> dict:
    return {'safe': bool(value['feedback']), 'text': value['candidate']['output']['text']}

def assess(ctx: Context, value: dict) -> dict:
    safe = value['candidate']['output']['safe']
    return {'status':'completed','findings':[{'criterion_id':'safe','verdict':'pass' if safe else 'fail','score':int(safe),'evidence':[value['evidence_refs'][-1]],'explanation':'Checked explicit safe flag.'}]}

def attack(ctx: Context, value: dict) -> dict:
    ctx.request('hidden', 'revise', {'secret':'attempt'})
    return {'status':'completed','findings':[]}

def effect(ctx: Context, value: dict) -> dict:
    return ctx.effect('delete', {'format':'remediate/1'}, lambda _: ({'done':True},{'proof':True}))

def quota(ctx: Context, value: dict) -> dict:
    for i in range(2):
        ctx.effect('model-' + str(i), {'format':'mas-inference/2'}, lambda _: EffectResult({'ok':True},{'fixture':True}))
    return {'status':'completed','findings':[]}
"#).unwrap();
    let agents=["draft","revise","assess","attack","effect","quota"].iter().map(|a|json!({"name":a,"implementation":{"kind":"native","language":"python","source":"app.py","handler":a}})).collect::<Vec<_>>();
    write(
        &root.join("agents.json"),
        &json!({"version":"mas-agents/3","agents":agents,"limits":{"max_calls":8,"wall_seconds":120}}),
    );
    let edges = ["draft", "revise", "assess", "attack", "effect", "quota"]
        .iter()
        .flat_map(|a| [json!(["client", a]), json!([a, "client"])])
        .chain([json!(["attack", "revise"]), json!(["revise", "attack"])])
        .collect::<Vec<_>>();
    write(
        &root.join("topology.json"),
        &json!({"version":"mas-topology/1","edges":edges}),
    );
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let built = ok(&[
        "build",
        "--project",
        root.to_str().unwrap(),
        "--sdk-root",
        sdk.to_str().unwrap(),
    ]);
    let input = json!({"sources":[{"id":"incident","kind":"evidence","content":{"text":"PRIVATE-INCIDENT"},"readers":["draft","revise","assess","attack","effect","quota"]}]});
    write(&root.join("input.json"), &input);
    let mut plan = json!({"version":"mas-host-revision/1","draft_agent":"draft","revision_agent":"revise","max_revisions":1,"max_invocations":4,"draft_inputs":["incident"],"revision_inputs":["incident"],"allowed_agents":[],"feedback_evaluators":["checker"],"rubric":{"schema_version":1,"id":"safety","criteria":[{"id":"safe","measurement":"deterministic","minimum":0,"maximum":1}]},"evaluators":[{"actor":"assess","identity":{"id":"checker","version":"1","kind":"deterministic"},"inputs":["incident"]}]});
    let plan_path = root.join("revision.json");
    write(&plan_path, &plan);
    let state = root.join("state");
    let input_path = root.join("input.json");
    let run = |id: &str| {
        ok(&[
            "run",
            "--bundle",
            built["bundle"].as_str().unwrap(),
            "--agent",
            "draft",
            "--input",
            input_path.to_str().unwrap(),
            "--state",
            state.to_str().unwrap(),
            "--run",
            id,
            "--revision-plan",
            plan_path.to_str().unwrap(),
        ])
    };
    let first = run("restricted");
    assert_eq!(first["status"], "completed");
    assert!(first["output"].is_null());
    assert!(!first.to_string().contains("PRIVATE-INCIDENT"));
    let replay = run("restricted");
    assert_eq!(first, replay);
    let observed = ok(&[
        "observe",
        "--state",
        state.to_str().unwrap(),
        "--run",
        "restricted",
        "--include-payloads",
        "true",
    ]);
    assert!(!observed.to_string().contains("PRIVATE-INCIDENT"));
    assert_eq!(observed["revision"]["budget"]["used_invocations"], 4);
    let db = rusqlite::Connection::open(state.join("restricted/execution.sqlite")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM revision_contexts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4);
    let assessments: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM revision_records WHERE kind='assessment'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(assessments, 2);
    let mut public = input.clone();
    public["sources"][0]["readers"] = json!(["*"]);
    write(&input_path, &public);
    let completed = run("public");
    assert_eq!(completed["output"]["safe"], true);
    assert_eq!(
        completed["revision"]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let revised = &completed["revision"]["candidates"][1];
    assert_eq!(revised["parent"]["id"], "candidate-0");
    assert_eq!(revised["feedback"][0]["id"], "assessment-0-0");
    let mut mixed = plan.clone();
    mixed["max_invocations"] = json!(6);
    mixed["evaluators"].as_array_mut().unwrap().push(json!({"actor":"assess","identity":{"id":"judgement","version":"1","kind":"model"},"inputs":["incident"],"rubric":{"schema_version":1,"id":"judgement-rubric","criteria":[{"id":"safe","measurement":"model","minimum":0,"maximum":1}]}}));
    write(&plan_path, &mixed);
    let multi = run("mixed");
    assert_eq!(multi["status"], "completed");
    assert_eq!(
        multi["revision"]["assessments"].as_array().unwrap().len(),
        4
    );
    assert_eq!(
        multi["revision"]["assessments"][1]["evaluator"]["kind"],
        "model"
    );
    assert_ne!(
        multi["revision"]["assessments"][0]["rubric"],
        multi["revision"]["assessments"][1]["rubric"]
    );
    plan["max_invocations"] = json!(1);
    write(&plan_path, &plan);
    let exhausted = run("exhausted");
    assert_eq!(exhausted["status"], "exhausted");
    assert_eq!(exhausted["revision"]["budget"]["used_invocations"], 1);
    let exhausted_db =
        rusqlite::Connection::open(state.join("exhausted/execution.sqlite")).unwrap();
    let steps: i64 = exhausted_db
        .query_row("SELECT COUNT(*) FROM steps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        steps, 1,
        "exhausted reservation must not retain a contextless step"
    );
    plan["max_invocations"] = json!(4);
    plan["max_revisions"] = json!(0);
    plan["evaluators"][0]["actor"] = json!("attack");
    write(&plan_path, &plan);
    let denied = run("peer-denied");
    assert_eq!(denied["status"], "exhausted");
    assert_eq!(denied["revision"]["assessments"][0]["status"], "error");
    plan["evaluators"][0]["actor"] = json!("effect");
    write(&plan_path, &plan);
    let effect = run("effect-denied");
    assert_eq!(effect["revision"]["assessments"][0]["status"], "error");
    let effect_db =
        rusqlite::Connection::open(state.join("effect-denied/execution.sqlite")).unwrap();
    let effects: i64 = effect_db
        .query_row("SELECT COUNT(*) FROM steps WHERE kind='effect'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(effects, 0);
    let bundle_path = Path::new(built["bundle"].as_str().unwrap());
    let original_bundle: Value = serde_json::from_slice(&fs::read(bundle_path).unwrap()).unwrap();
    let mut limited = original_bundle.clone();
    limited["topology"]["max_calls"] = json!(1);
    limited["agents"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["name"] == "quota")
        .unwrap()["configuration"] = json!({"kind":"llm"});
    write(bundle_path, &limited);
    plan["evaluators"][0]["actor"] = json!("quota");
    plan["evaluators"][0]["identity"]["kind"] = json!("model");
    plan["rubric"]["criteria"][0]["measurement"] = json!("model");
    write(&plan_path, &plan);
    let call_limited = run("call-limit");
    assert_eq!(call_limited["status"], "exhausted");
    assert_eq!(
        call_limited["revision"]["assessments"][0]["status"],
        "error"
    );
    assert_eq!(
        run("call-limit"),
        call_limited,
        "exhausted replay spends no further calls"
    );
    let call_db = rusqlite::Connection::open(state.join("call-limit/protocol.sqlite")).unwrap();
    let calls: i64 = call_db
        .query_row("SELECT COUNT(*) FROM calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(calls, 1);
    let observed = ok(&[
        "observe",
        "--state",
        state.to_str().unwrap(),
        "--run",
        "call-limit",
        "--include-payloads",
        "true",
    ]);
    assert_eq!(observed["revision"]["status"], "exhausted");
    // One request consumes the only message: terminal result admission is refused.
    let mut limited = original_bundle;
    limited["topology"]["max_messages"] = json!(1);
    write(bundle_path, &limited);
    plan["evaluators"][0]["actor"] = json!("assess");
    plan["evaluators"][0]["identity"]["kind"] = json!("deterministic");
    plan["rubric"]["criteria"][0]["measurement"] = json!("deterministic");
    write(&plan_path, &plan);
    let communication = run("communication-limit");
    assert_eq!(communication["status"], "exhausted");
    assert!(communication["output"].is_null());
    assert_eq!(run("communication-limit"), communication);
    assert!(communication["revision"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["record"]["content"]["admission"]["code"] == "budget_exceeded"));
    fs::remove_dir_all(root).unwrap();
}
