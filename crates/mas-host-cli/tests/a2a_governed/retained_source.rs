//! Exercise source ancestry through authenticated HTTP without reopening execution.
use super::*;

const SOURCE_EXTENSION: &str = "urn:mas:host-governed-source-read:1";
const EFFECT_EXTENSION: &str = "urn:mas:host-governed-effect-read:1";

fn configured_source(project: &Project) {
    let path = project.root.join("governance.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["version"] = json!("mas-governed-local/2");
    config["baseline_policy_refs"] = json!(["baseline"]);
    config["agents"][0]["effects"][0]["source_policy_refs"] = json!(["collected"]);
    write(path, &config);
    let path = project.root.join("authority.json");
    let mut authority: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut policies = Vec::new();
    for policy in authority["policies"].as_array().unwrap() {
        let mut baseline = policy.clone();
        baseline["id"] = json!("baseline");
        let mut rule = baseline["rules"][0].clone();
        rule["operation"] = json!("resolve");
        rule["recipient"] = json!("alice");
        rule["destination"] = json!("client");
        baseline["rules"].as_array_mut().unwrap().push(rule);
        let mut source = baseline.clone();
        source["id"] = json!("collected");
        policies.extend([baseline, source]);
    }
    authority["policies"] = json!(policies);
    write(path, &authority);
}

fn exchange(
    server: &Server,
    token: &str,
    extensions: Option<&str>,
    method: &str,
    params: Value,
) -> (u16, String, Value) {
    let body =
        json!({"jsonrpc":"2.0","id":"source-test","method":method,"params":params}).to_string();
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let extensions = extensions
        .map(|value| format!("A2A-Extensions: {value}\r\n"))
        .unwrap_or_default();
    write!(stream,"POST /a2a HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nA2A-Version: 1.0\r\n{extensions}Content-Length: {}\r\n\r\n{body}",server.address,body.len()).unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        head.to_lowercase(),
        serde_json::from_str(body).unwrap(),
    )
}

fn source(server: &Server, token: &str, id: &str, key: &str) -> (u16, String, Value) {
    exchange(
        server,
        token,
        Some(&format!("{EXTENSION}, {SOURCE_EXTENSION}")),
        "GetRetainedSource",
        json!({"id":id,"key":key}),
    )
}

fn submitted(project: &Project, server: &Server, mode: &str) -> String {
    let original = project.send(mode, PRIVATE);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (_, value) = server.rpc(TOKEN, "SendMessage", original.clone());
        if value["error"] == json!({"code":-32001,"message":"task_not_found"}) {
            assert!(Instant::now() < deadline, "root admission deadline");
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        assert!(value.get("error").is_none(), "{value}");
        return value["result"]["task"]["id"].as_str().unwrap().to_owned();
    }
}

fn completed(server: &Server, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (_, response) = server.rpc(TOKEN, "GetTask", json!({"id":id}));
        if response["error"] == json!({"code":-32001,"message":"task_not_found"}) {
            assert!(Instant::now() < deadline, "root acquisition deadline");
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        assert!(response.get("error").is_none(), "{response}");
        let task = &response["result"];
        let state = task["metadata"]["mas"]["state"].as_str().unwrap();
        if state == "completed" {
            return task.clone();
        }
        assert!(!["failed", "unknown"].contains(&state), "{task}");
        assert!(
            Instant::now() < deadline,
            "task completion deadline: {task}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn denied(value: &(u16, String, Value)) {
    assert_eq!(value.0, 200);
    assert_eq!(
        value.2["error"],
        json!({"code":-32001,"message":"source_not_found"})
    );
    assert!(value.2.get("result").is_none(), "{}", value.2);
    assert!(!value.2.to_string().contains(PRIVATE));
}

fn authority_denied(value: &(u16, String, Value)) {
    assert_eq!(value.0, 401);
    assert_eq!(value.2, json!({"error":"unauthorised"}));
    assert!(value.2.get("result").is_none());
    assert!(!value.2.to_string().contains(PRIVATE));
}

fn journal(project: &Project, id: &str) -> Value {
    let db = rusqlite::Connection::open(
        project
            .root
            .join("state/runs")
            .join(id)
            .join("execution.sqlite"),
    )
    .unwrap();
    let mut result = serde_json::Map::new();
    for table in [
        "steps",
        "governed_inputs",
        "governed_completions",
        "governed_sources",
        "governed_source_steps",
        "governed_exposures",
        "governed_contexts",
        "governed_pending",
    ] {
        let mut query = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = query.column_count();
        let rows = query
            .query_map([], |row| {
                (0..count)
                    .map(|i| match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Ok(Value::Null),
                        rusqlite::types::ValueRef::Text(text) => {
                            Ok(json!(std::str::from_utf8(text).unwrap()))
                        }
                        rusqlite::types::ValueRef::Integer(number) => Ok(json!(number)),
                        _ => panic!("unexpected journal representation"),
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        result.insert(table.into(), json!(rows));
    }
    json!(result)
}

fn rebuild(project: &mut Project) {
    let built = Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(["build", "--project"])
        .arg(&project.root)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    project.bundle = PathBuf::from(
        serde_json::from_slice::<Value>(&built.stdout).unwrap()["bundle"]
            .as_str()
            .unwrap(),
    );
    let described = Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(["describe", "--bundle"])
        .arg(&project.bundle)
        .output()
        .unwrap();
    assert!(
        described.status.success(),
        "{}",
        String::from_utf8_lossy(&described.stderr)
    );
    project.descriptor = serde_json::from_slice::<Value>(&described.stdout).unwrap()
        ["bundle_digest"]
        .as_str()
        .unwrap()
        .into();
}

fn change_authority(project: &Project, change: impl FnOnce(&mut Value)) {
    let path = project.root.join("authority.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut value);
    write(project.root.join("authority-source-next.json"), &value);
    fs::rename(project.root.join("authority-source-next.json"), path).unwrap();
}

fn permit_partial_payload_fixture(project: &Project) {
    let path = project.root.join("inspection-worker.py");
    let worker = fs::read_to_string(&path).unwrap().replace(
        "if mode=='partial':",
        "if mode=='partial' or mode=='payload-partial' and r['text']=='approved':",
    );
    fs::write(path, &worker).unwrap();
    let path = project.root.join("inspection-manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["worker_sha256"] = json!(digest(&worker));
    let raw = serde_json::to_vec(&manifest).unwrap();
    fs::write(path, &raw).unwrap();
    let path = project.root.join("privacy.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["detector"]["bundle_sha256"] = json!(Sha256::digest(&raw)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>());
    write(path, &config);
}

#[test]
fn source_reader_requires_activation_and_advertises_only_for_source_enabled_profiles() {
    let project = Project::new();
    configured_source(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    completed(&server, &id);
    for (extensions, message) in [
        (None, "required_extension_missing"),
        (Some(SOURCE_EXTENSION), "required_extension_missing"),
        (Some(EXTENSION), "required_source_extension_missing"),
    ] {
        let value = exchange(
            &server,
            TOKEN,
            extensions,
            "GetRetainedSource",
            json!({"id":id,"key":"collect"}),
        );
        assert_eq!(value.2["error"]["message"], message);
        assert!(value.2.get("result").is_none());
    }
    let response = exchange(
        &server,
        TOKEN,
        Some(&format!(
            "urn:unknown:optional, {EXTENSION}, {SOURCE_EXTENSION}"
        )),
        "GetRetainedSource",
        json!({"id":id,"key":"collect"}),
    );
    assert!(response.2.get("error").is_none(), "{}", response.2);
    assert!(response
        .1
        .contains(&format!("a2a-extensions: {EXTENSION}, {SOURCE_EXTENSION}")));
    assert!(!response.1.contains("urn:unknown:optional"));
    let card = exchange(&server, TOKEN, None, "GetExtendedAgentCard", json!({})).2;
    let extension = card["result"]["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["uri"] == SOURCE_EXTENSION)
        .unwrap();
    assert_eq!(extension["required"], false);
    assert_eq!(extension["params"]["requires"], json!([EXTENSION]));
    assert_eq!(project.calls(), 1);
    drop(server);
    let legacy = Project::new();
    let server = legacy.start();
    let card = exchange(&server, TOKEN, None, "GetExtendedAgentCard", json!({})).2;
    assert!(!card.to_string().contains(SOURCE_EXTENSION));
    assert_eq!(
        source(&server, TOKEN, &id, "collect").2["error"]["message"],
        "method_not_found"
    );
    assert_eq!(legacy.calls(), 0);
    drop(server);
    let ordinary = Project::new();
    let path = ordinary.root.join("gateway.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config.as_object_mut().unwrap().remove("governance_file");
    config
        .as_object_mut()
        .unwrap()
        .remove("authentication_file");
    config["version"] = json!("mas-host-a2a/1");
    config["principals"] =
        json!([{"name":"alice","token_sha256":digest(TOKEN),"agents":["collector"]}]);
    write(path, &config);
    let server = ordinary.start();
    let card = exchange(&server, TOKEN, None, "GetExtendedAgentCard", json!({})).2;
    assert!(!card.to_string().contains(SOURCE_EXTENSION));
    assert_eq!(
        source(&server, TOKEN, &id, "collect").2["error"]["message"],
        "method_not_found"
    );
    assert_eq!(ordinary.calls(), 0);
}

#[test]
fn source_reader_hides_foreign_tasks_and_rejects_user_selected_scope_or_paths() {
    let project = Project::new();
    configured_source(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    completed(&server, &id);
    denied(&source(&server, OTHER, &id, "collect"));
    for forged in [
        "../execution.sqlite".into(),
        "a2a-../../execution.sqlite".into(),
        format!("a2a-{}", "f".repeat(56)),
    ] {
        denied(&source(&server, TOKEN, &forged, "collect"));
    }
    for extra in [
        "actor",
        "parent_invocation",
        "directory",
        "effect_id",
        "source_id",
        "policy_refs",
    ] {
        let mut params = json!({"id":id,"key":"collect"});
        params[extra] = json!(PRIVATE);
        let response = exchange(
            &server,
            TOKEN,
            Some(&format!("{EXTENSION}, {SOURCE_EXTENSION}")),
            "GetRetainedSource",
            params,
        );
        assert!(response.2.get("error").is_some());
        assert!(response.2.get("result").is_none());
        assert!(!response.2.to_string().contains(PRIVATE));
    }
    for key in ["".to_owned(), "x".repeat(129), "\n".to_owned()] {
        let response = source(&server, TOKEN, &id, &key);
        assert_eq!(response.2["error"]["code"], -32602);
        assert!(response.2.get("result").is_none());
    }
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn source_reader_rechecks_current_authority_and_same_scope_credential_rotation() {
    let project = Project::new();
    configured_source(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    completed(&server, &id);
    let original = source(&server, TOKEN, &id, "collect").2;
    assert!(original.get("error").is_none(), "{original}");
    let rotated = "source-rotated-synthetic-token-0123456789";
    project.authentication(rotated);
    let old = source(&server, TOKEN, &id, "collect");
    assert_eq!(old.0, 401);
    assert!(old.2.get("result").is_none());
    assert_eq!(source(&server, rotated, &id, "collect").2, original);
    fs::rename(
        project.root.join("authority.json"),
        project.root.join("authority-unavailable.json"),
    )
    .unwrap();
    authority_denied(&source(&server, rotated, &id, "collect"));
    fs::rename(
        project.root.join("authority-unavailable.json"),
        project.root.join("authority.json"),
    )
    .unwrap();
    change_authority(&project, |authority| {
        authority["grants"]
            .as_array_mut()
            .unwrap()
            .retain(|grant| grant["grant_ref"] != "grant-a");
    });
    authority_denied(&source(&server, rotated, &id, "collect"));
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

fn available_view(view: &Value) {
    let expected: [&str; 15] = [
        "version",
        "namespace_ref",
        "actor",
        "parent_invocation",
        "key",
        "effect_id",
        "source_id",
        "state",
        "host_settlement",
        "external_execution",
        "payload_disposition",
        "source_metadata",
        "lineage",
        "projection",
        "raw_source_disposition",
    ];
    assert_eq!(view.as_object().unwrap().len(), expected.len(), "{view}");
    for field in expected {
        assert!(view.get(field).is_some(), "missing {field}: {view}");
    }
    assert_eq!(view["version"], "mas-governed-source-view/1");
    assert_eq!(view["actor"], "collector");
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "available");
    assert_eq!(view["projection"], json!({"approved":true}));
    assert_eq!(view["raw_source_disposition"], "not-retained");
    assert_eq!(view["source_metadata"]["version"], "mas-governed-source/1");
    assert_eq!(view["source_metadata"]["kind"], "effect-output");
    assert_eq!(view["source_metadata"]["id"], view["source_id"]);
    assert_eq!(view["source_metadata"]["step_id"], view["effect_id"]);
    assert_eq!(view["source_metadata"]["payload_available"], true);
    assert_eq!(
        view["source_metadata"]["policy_refs"],
        json!(["baseline", "collected"])
    );
    assert!(view["lineage"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record == &view["source_metadata"]));
}

#[test]
fn source_reader_delivers_exact_projection_and_complete_metadata_without_mutating_history() {
    let project = Project::new();
    configured_source(&project);
    project.privacy_inspector("complete", 10_000);
    permit_partial_payload_fixture(&project);
    let server = project.start();
    let (_, admission) = server.rpc(TOKEN, "SendMessage", project.send("available", PRIVATE));
    assert!(admission.get("error").is_none(), "{admission}");
    assert_eq!(
        admission["result"]["task"]["metadata"]["mas"]["state"],
        "submitted"
    );
    assert_eq!(
        admission["result"]["task"]["status"]["state"],
        "TASK_STATE_SUBMITTED"
    );
    assert!(admission["result"]["task"].get("artifacts").is_none());
    let id = admission["result"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    completed(&server, &id);
    let before = journal(&project, &id);
    let response = source(&server, TOKEN, &id, "collect").2;
    assert!(response.get("error").is_none(), "{response}");
    available_view(&response["result"]);
    assert_eq!(response["result"]["key"], "collect");
    fs::write(project.root.join("inspection-mode"), "payload-partial").unwrap();
    let withheld = source(&server, TOKEN, &id, "collect").2;
    assert!(withheld.get("error").is_none(), "{withheld}");
    assert_eq!(withheld["result"]["host_settlement"], "completed");
    assert_eq!(withheld["result"]["external_execution"], "completed");
    assert_eq!(withheld["result"]["payload_disposition"], "withheld");
    assert!(withheld["result"]["projection"].is_null());
    assert_eq!(
        withheld["result"]["source_metadata"],
        response["result"]["source_metadata"]
    );
    assert_eq!(withheld["result"]["lineage"], response["result"]["lineage"]);
    fs::write(project.root.join("inspection-mode"), "partial").unwrap();
    denied(&source(&server, TOKEN, &id, "collect"));
    fs::write(project.root.join("inspection-mode"), "complete").unwrap();
    assert_eq!(source(&server, TOKEN, &id, "collect").2, response);
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    assert!(!fs::read(project.root.join("inspection-calls"))
        .unwrap()
        .is_empty());
    project.assert_private_absent();
    drop(server);
    let restarted = project.start();
    assert_eq!(source(&restarted, TOKEN, &id, "collect").2, response);
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn source_reader_preserves_known_withholding_and_authorised_absence_without_refetch() {
    let project = Project::new();
    configured_source(&project);
    let server = project.start();
    let id = submitted(&project, &server, "withheld");
    completed(&server, &id);
    let before = journal(&project, &id);
    let response = source(&server, TOKEN, &id, "collect").2;
    assert!(response.get("error").is_none(), "{response}");
    let view = &response["result"];
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "withheld");
    assert!(view["source_id"].is_string());
    assert_eq!(view["source_metadata"]["payload_available"], false);
    assert!(view["projection"].is_null());
    assert_eq!(view["raw_source_disposition"], "not-retained");
    let missing = source(&server, TOKEN, &id, "absent-effect").2;
    assert!(missing.get("error").is_none(), "{missing}");
    assert_eq!(missing["result"]["state"], "not-recorded");
    assert_eq!(missing["result"]["host_settlement"], "unknown");
    assert_eq!(missing["result"]["external_execution"], "unknown");
    assert_eq!(missing["result"]["payload_disposition"], "unavailable");
    for field in ["source_id", "source_metadata", "projection"] {
        assert!(missing["result"][field].is_null());
    }
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn source_expiry_survives_same_epoch_grant_renewal_and_old_delivery_methods() {
    let project = Project::new();
    configured_source(&project);
    let server = project.start();
    // Startup is outside this real-expiry fixture's explicit lease window.
    let original_expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 30_000;
    change_authority(&project, |value| {
        value["grants"][0]["expires_at_ms"] = json!(original_expiry)
    });
    let id = submitted(&project, &server, "available");
    completed(&server, &id);
    let initial = source(&server, TOKEN, &id, "collect").2;
    assert!(initial.get("error").is_none(), "{initial}");
    assert_eq!(
        initial["result"]["source_metadata"]["expires_at_ms"],
        original_expiry
    );
    let before = journal(&project, &id);
    while SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        <= original_expiry
    {
        thread::sleep(Duration::from_millis(20));
    }
    change_authority(&project, |value| {
        value["grants"][0]["expires_at_ms"] = json!(original_expiry + 300_000)
    });
    denied(&source(&server, TOKEN, &id, "collect"));
    let task = server.rpc(TOKEN, "GetTask", json!({"id":id})).1;
    assert_eq!(
        task["error"],
        json!({"code":-32001,"message":"task_not_found"})
    );
    assert!(task.get("result").is_none());
    let effect = exchange(
        &server,
        TOKEN,
        Some(&format!("{EXTENSION}, {EFFECT_EXTENSION}")),
        "GetRetainedEffect",
        json!({"id":id,"key":"collect"}),
    )
    .2;
    assert_eq!(
        effect["error"],
        json!({"code":-32001,"message":"effect_not_found"})
    );
    assert!(effect.get("result").is_none());
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

fn nested_consumer(project: &mut Project, permit_sink: bool, child_acquires: bool) {
    let path = project.root.join("application.py");
    let original = fs::read_to_string(&path).unwrap();
    let original = original.replace(
        "from mas_host import Context, EffectResult",
        "from mas_host import Context, EffectResult, HostError",
    );
    let original=original.replace("    return ctx.effect('collect', {'format': 'fixture-effect/1', 'destination': 'fixture'}, action)","    value = ctx.effect('collect', {'format': 'fixture-effect/1', 'destination': 'fixture'}, action)\n    return ctx.request('nested-consumer', 'consumer', value)");
    let child = r#"
def consume(ctx: Context, value: dict) -> dict:
    with open('child-calls', 'a') as stream: stream.write('x')
    def action(invocation_id):
        with open('sink-calls', 'a') as stream: stream.write('x')
        return EffectResult({'approved': True}, {'confirmed': True, 'execution':'completed'})
    try:
        ctx.effect('sink', {'format':'fixture-sink/1', 'destination':'fixture', 'source_refs':[]}, action)
    except HostError as error:
        if 'governed-policy-denied' not in str(error): raise
        with open('sink-blocked', 'w') as stream: stream.write('blocked')
    return {'approved': True}
"#;
    fs::write(path, format!("{original}\n{child}")).unwrap();
    let path = project.root.join("agents.toml");
    let mut agents = fs::read_to_string(&path).unwrap();
    agents.push_str(
        r#"
[[agents]]
name = "consumer"
operation = "consume/1"
input_schema = "output.json"
output_schema = "output.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "consume"
"#,
    );
    fs::write(path, agents).unwrap();
    write(
        project.root.join("topology.json"),
        &json!({"version":"mas-topology/1","edges":[["client","collector"],["collector","client"],["collector","consumer"],["consumer","collector"]]}),
    );
    let path = project.root.join("governance.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let output = config["agents"][0]["output_schema"].clone();
    let evidence = config["agents"][0]["effects"][0]["evidence_schema"].clone();
    config["agents"].as_array_mut().unwrap().push(json!({"actor":"consumer","destination":"local-worker","input_schema":output,"output_schema":output,
        "effects":[{"format":"fixture-sink/1","destination":"fixture","descriptor_schema":object(json!({"format":{"type":"string","enum":["fixture-sink/1"]},"destination":{"type":"string","enum":["fixture"]},"source_refs":{"type":"array","items":{"type":"string","enum":["ignored"]},"maxItems":0}}),&["format","destination","source_refs"]),"output_schema":output,"evidence_schema":evidence}]}));
    write(path, &config);
    change_authority(project, |authority| {
        for policy in authority["policies"].as_array_mut().unwrap() {
            let rules = policy["rules"].as_array().unwrap().clone();
            for mut rule in rules {
                rule["actor"] = json!("consumer");
                policy["rules"].as_array_mut().unwrap().push(rule);
            }
            let mut invoke = policy["rules"][0].clone();
            invoke["actor"] = json!("consumer");
            invoke["recipient"] = json!("worker:consumer");
            policy["rules"].as_array_mut().unwrap().push(invoke);
            if policy["id"] == "baseline" || permit_sink {
                let mut rule = policy["rules"][0].clone();
                rule["actor"] = json!("consumer");
                rule["operation"] = json!("effect");
                rule["recipient"] = json!("operation:fixture-sink/1");
                rule["destination"] = json!("fixture");
                policy["rules"].as_array_mut().unwrap().push(rule);
            }
        }
    });
    if child_acquires {
        fs::write(project.root.join("application.py"),r#"from mas_host import Context, EffectResult, HostError

def collect(ctx: Context, value: dict) -> dict:
    received = ctx.request('nested-consumer', 'consumer', {'approved': True})
    if received != {'approved': True}: raise RuntimeError('unexpected governed child projection')
    def action(invocation_id):
        with open('sink-calls', 'a') as stream: stream.write('x')
        return EffectResult({'approved': True}, {'confirmed': True, 'execution':'completed'})
    try:
        ctx.effect('sink', {'format':'fixture-sink/1', 'destination':'fixture', 'source_refs':[]}, action)
    except HostError as error:
        if 'governed-policy-denied' not in str(error): raise
        with open('sink-blocked', 'w') as stream: stream.write('blocked')
    return {'approved': True}

def consume(ctx: Context, value: dict) -> dict:
    with open('child-calls', 'a') as stream: stream.write('x')
    def action(invocation_id):
        with open('calls', 'a') as stream: stream.write('x')
        return EffectResult({'approved': True, 'private_note':'private-person-4387@example.invalid'}, {'confirmed': True, 'execution':'completed', 'private_note':'private-person-4387@example.invalid'})
    return ctx.effect('collect', {'format':'fixture-effect/1', 'destination':'fixture'}, action)
"#).unwrap();
        let path = project.root.join("governance.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let source = config["agents"][0]["effects"][0].clone();
        let sink = config["agents"][1]["effects"][0].clone();
        config["agents"][0]["effects"] = json!([sink]);
        config["agents"][1]["effects"] = json!([source]);
        write(path, &config);
        change_authority(project, |authority| {
            for policy in authority["policies"].as_array_mut().unwrap() {
                for rule in policy["rules"].as_array_mut().unwrap() {
                    if rule["operation"] == "effect"
                        && rule["recipient"] == "operation:fixture-sink/1"
                    {
                        rule["actor"] = json!("collector");
                    }
                }
            }
        });
    }
    rebuild(project);
}

#[test]
fn nested_native_consumer_cannot_remove_inherited_restrictions_with_empty_lineage() {
    for (child_acquires, permit_sink) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let mut project = Project::new();
        configured_source(&project);
        nested_consumer(&mut project, permit_sink, child_acquires);
        let server = project.start();
        let id = submitted(&project, &server, "available");
        let task = completed(&server, &id);
        assert_eq!(
            task["artifacts"][0]["parts"][0]["data"],
            json!({"approved":true})
        );
        assert_eq!(project.calls(), 1);
        assert_eq!(
            fs::read(project.root.join("sink-calls"))
                .unwrap_or_default()
                .len(),
            usize::from(permit_sink)
        );
        assert_eq!(project.root.join("sink-blocked").exists(), !permit_sink);
        let db = rusqlite::Connection::open(
            project
                .root
                .join("state/runs")
                .join(&id)
                .join("execution.sqlite"),
        )
        .unwrap();
        let sinks: i64 = db
            .query_row(
                "SELECT count(*) FROM steps WHERE kind='effect' AND actor=?",
                [if child_acquires {
                    "collector"
                } else {
                    "consumer"
                }],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sinks, i64::from(permit_sink));
        let (_, replay) = server.rpc(TOKEN, "SendMessage", project.send("available", PRIVATE));
        assert!(replay.get("error").is_none(), "{replay}");
        assert_eq!(replay["result"]["task"]["id"], id);
        assert_eq!(project.calls(), 1);
        assert_eq!(fs::read(project.root.join("child-calls")).unwrap().len(), 1);
        assert_eq!(
            fs::read(project.root.join("sink-calls"))
                .unwrap_or_default()
                .len(),
            usize::from(permit_sink)
        );
        project.assert_private_absent();
    }
}

fn completed_graph(server: &Server, project: &Project, id: &str) {
    let started = Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completed(server, id)));
    if let Err(failure) = outcome {
        let bundle: Value = serde_json::from_slice(&fs::read(&project.bundle).unwrap()).unwrap();
        let db = rusqlite::Connection::open(
            project
                .root
                .join("state/runs")
                .join(id)
                .join("execution.sqlite"),
        )
        .unwrap();
        let mut query=db.prepare("SELECT state,count(*) FROM steps WHERE kind='effect' GROUP BY state ORDER BY state").unwrap();
        let states = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()
            .unwrap();
        let root_state: String = db
            .query_row(
                "SELECT state FROM steps WHERE kind='request' AND parent='' AND actor='client'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let code = fs::read_to_string(project.root.join("graph-failure-code"))
            .unwrap_or_else(|_| "not-recorded".into());
        assert!(
            ["call_budget_exceeded", "deadline", "other", "not-recorded"].contains(&code.as_str())
        );
        eprintln!(
            "bounded graph diagnostic: {}",
            json!({"callback_count":project.calls(),"root_state":root_state,"effect_states":states,"failure_code":code,"max_calls":bundle["topology"]["max_calls"],"wall_seconds":bundle["topology"]["wall_seconds"],"elapsed_ms":started.elapsed().as_millis()})
        );
        std::panic::resume_unwind(failure);
    }
}

#[test]
fn source_reader_preserves_more_than_thirty_two_ancestors_and_rejects_missing_parent() {
    let mut project = Project::new();
    configured_source(&project);
    let path = project.root.join("application.py");
    let application=fs::read_to_string(&path).unwrap().replace(
        "    return ctx.effect('collect', {'format': 'fixture-effect/1', 'destination': 'fixture'}, action)",
        "    for number in range(33):\n        try:\n            result = ctx.effect('collect-' + str(number), {'format': 'fixture-effect/1', 'destination': 'fixture'}, action)\n        except Exception as error:\n            code = 'call_budget_exceeded' if 'call' in str(error) and 'budget' in str(error) else ('deadline' if 'deadline' in str(error) else 'other')\n            with open('graph-failure-code', 'w') as stream: stream.write(code)\n            raise\n    return result");
    fs::write(path, application).unwrap();
    // This workload performs 33 effects; the ordinary fixture default permits
    // only 32 calls. Declare its own bounded budget without changing defaults.
    let path = project.root.join("agents.toml");
    let mut agents = fs::read_to_string(&path).unwrap();
    agents.push_str("\n[limits]\nmax_calls = 40\n");
    fs::write(path, agents).unwrap();
    change_authority(&project, |authority| {
        for policy in authority["policies"].as_array_mut().unwrap() {
            for rule in policy["rules"].as_array_mut().unwrap() {
                rule["max_bytes"] = json!(65_536);
            }
        }
    });
    rebuild(&mut project);
    let bundle: Value = serde_json::from_slice(&fs::read(&project.bundle).unwrap()).unwrap();
    assert_eq!(bundle["topology"]["max_calls"], 40);
    assert_eq!(bundle["topology"]["wall_seconds"], 60);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    completed_graph(&server, &project, &id);
    let before = journal(&project, &id);
    let response = source(&server, TOKEN, &id, "collect-32").2;
    assert!(response.get("error").is_none(), "{response}");
    available_view(&response["result"]);
    let lineage = response["result"]["lineage"].as_array().unwrap();
    assert_eq!(
        lineage
            .iter()
            .filter(|record| record["kind"] == "effect-output")
            .count(),
        33
    );
    assert!(lineage.len() > 33, "{lineage:?}");
    for record in lineage {
        for parent in record["parents"].as_array().unwrap() {
            assert!(
                lineage.iter().any(|ancestor| &ancestor["id"] == parent),
                "missing ancestor {parent}"
            );
        }
    }
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 33);
    project.assert_private_absent();
    drop(server);
    let server = project.start();
    assert_eq!(source(&server, TOKEN, &id, "collect-32").2, response);
    assert_eq!(project.calls(), 33);
    let db = rusqlite::Connection::open(
        project
            .root
            .join("state/runs")
            .join(&id)
            .join("execution.sqlite"),
    )
    .unwrap();
    let ancestor = lineage
        .iter()
        .find(|record| record["kind"] == "request-input")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        db.execute("DELETE FROM governed_sources WHERE id=?", [ancestor])
            .unwrap(),
        1
    );
    let broken = journal(&project, &id);
    denied(&source(&server, TOKEN, &id, "collect-32"));
    assert_eq!(journal(&project, &id), broken);
    assert_eq!(project.calls(), 33);
    project.assert_private_absent();
}
