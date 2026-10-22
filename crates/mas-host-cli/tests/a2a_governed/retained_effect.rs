//! Challenge the authenticated reader without granting execution or lookup authority.
use super::*;

const READ_EXTENSION: &str = "urn:mas:host-governed-effect-read:1";

fn permit_reads(project: &Project) {
    let path = project.root.join("authority.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for policy in value["policies"].as_array_mut().unwrap() {
        let mut rule = policy["rules"][0].clone();
        rule["operation"] = json!("resolve");
        rule["recipient"] = json!("alice");
        rule["destination"] = json!("client");
        policy["rules"].as_array_mut().unwrap().push(rule);
    }
    write(path, &value);
}

fn exchange(
    server: &Server,
    token: &str,
    extensions: Option<&str>,
    method: &str,
    params: Value,
) -> (u16, String, Value) {
    let body =
        json!({"jsonrpc":"2.0","id":"reader-test","method":method,"params":params}).to_string();
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let extensions = extensions
        .map(|value| format!("A2A-Extensions: {value}\r\n"))
        .unwrap_or_default();
    write!(stream, "POST /a2a HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nA2A-Version: 1.0\r\n{extensions}Content-Length: {}\r\n\r\n{body}", server.address, body.len()).unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        head.to_lowercase(),
        serde_json::from_str(body).unwrap(),
    )
}

fn read(server: &Server, token: &str, id: &str, key: &str) -> (u16, String, Value) {
    exchange(
        server,
        token,
        Some(&format!("{EXTENSION}, {READ_EXTENSION}")),
        "GetRetainedEffect",
        json!({"id":id,"key":key}),
    )
}

fn submitted(project: &Project, server: &Server, mode: &str) -> String {
    let (_, response) = server.rpc(TOKEN, "SendMessage", project.send(mode, PRIVATE));
    assert!(response.get("error").is_none(), "{response}");
    response["result"]["task"]["id"].as_str().unwrap().into()
}

fn assert_denied(value: &(u16, String, Value)) {
    assert_eq!(value.0, 200);
    assert_eq!(
        value.2["error"],
        json!({"code":-32001,"message":"effect_not_found"})
    );
    assert!(value.2.get("result").is_none(), "{}", value.2);
    assert!(!value.2.to_string().contains(PRIVATE));
}

fn logical_history(project: &Project, id: &str) -> Value {
    let db = rusqlite::Connection::open(
        project
            .root
            .join("state/runs")
            .join(id)
            .join("execution.sqlite"),
    )
    .unwrap();
    let mut query = db.prepare("SELECT id,parent,key,kind,actor,target,state,input,output,evidence FROM steps ORDER BY id").unwrap();
    let rows = query
        .query_map([], |row| {
            let mut fields = Vec::new();
            for index in 0..10 {
                fields.push(row.get::<_, Option<String>>(index)?);
            }
            Ok(fields)
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let completions: i64 = db
        .query_row("SELECT count(*) FROM governed_completions", [], |r| {
            r.get(0)
        })
        .unwrap();
    json!({"steps":rows,"completions":completions})
}

fn pause_effect(project: &mut Project) {
    let path = project.root.join("application.py");
    let source = fs::read_to_string(&path).unwrap();
    let source=source.replace("    def action(invocation_id):\n", "    def action(invocation_id):\n        import os,time\n        with open('effect-started','w') as stream: stream.write('x')\n        while not os.path.exists('effect-release'): time.sleep(0.02)\n");
    fs::write(path, source).unwrap();
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

#[test]
fn retained_reader_requires_both_extensions_and_advertises_only_on_governed_services() {
    let project = Project::new();
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    for (extensions, message) in [
        (None, "required_extension_missing"),
        (Some(READ_EXTENSION), "required_extension_missing"),
        (Some(EXTENSION), "required_effect_extension_missing"),
    ] {
        let response = exchange(
            &server,
            TOKEN,
            extensions,
            "GetRetainedEffect",
            json!({"id":id,"key":"collect"}),
        );
        assert_eq!(response.2["error"]["message"], message);
        assert!(response.2.get("result").is_none());
        assert!(!response
            .1
            .contains(&format!("a2a-extensions: {EXTENSION}, {READ_EXTENSION}")));
    }
    let response = exchange(
        &server,
        TOKEN,
        Some(&format!(
            "urn:unknown:optional, {EXTENSION}, {READ_EXTENSION}"
        )),
        "GetRetainedEffect",
        json!({"id":id,"key":"collect"}),
    );
    assert!(response.2.get("error").is_none(), "{}", response.2);
    assert!(response
        .1
        .contains(&format!("a2a-extensions: {EXTENSION}, {READ_EXTENSION}")));
    let card = exchange(&server, TOKEN, None, "GetExtendedAgentCard", json!({})).2;
    let extension = card["result"]["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["uri"] == READ_EXTENSION)
        .unwrap();
    assert_eq!(extension["required"], false);
    assert_eq!(extension["params"]["requires"], json!([EXTENSION]));
    assert_eq!(project.calls(), 1);
    drop(server);
    // The unchanged ordinary service has no reader capability, even when the
    // caller activates the new URI itself.
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
    assert!(!card.to_string().contains(READ_EXTENSION));
    assert_eq!(
        read(&server, TOKEN, &id, "collect").2["error"]["message"],
        "method_not_found"
    );
    assert_eq!(ordinary.calls(), 0);
}

#[test]
fn retained_reader_preserves_completion_withholding_and_read_only_history_after_restart() {
    for mode in ["available", "withheld"] {
        let project = Project::new();
        permit_reads(&project);
        let server = project.start();
        let id = submitted(&project, &server, mode);
        server.completed_within(&id, Duration::from_secs(30));
        let history = logical_history(&project, &id);
        let response = read(&server, TOKEN, &id, "collect");
        assert!(response.2.get("error").is_none(), "{}", response.2);
        let view = &response.2["result"];
        assert_eq!(view["version"], "mas-governed-effect-view/1");
        assert_eq!(view["actor"], "collector");
        assert_eq!(view["key"], "collect");
        assert_eq!(view["host_settlement"], "completed");
        assert_eq!(view["external_execution"], "completed");
        assert_eq!(
            view["payload_disposition"],
            if mode == "available" {
                "available"
            } else {
                "withheld"
            }
        );
        if mode == "available" {
            assert_eq!(view["output"], json!({"approved":true}));
        } else {
            for field in ["descriptor", "output", "evidence"] {
                assert!(view[field].is_null());
            }
        }
        assert_eq!(logical_history(&project, &id), history);
        assert_eq!(project.calls(), 1);
        project.assert_private_absent();
        drop(server);
        let restarted = project.start();
        assert_eq!(read(&restarted, TOKEN, &id, "collect").2, response.2);
        assert_eq!(logical_history(&project, &id), history);
        assert_eq!(project.calls(), 1);
    }
}

#[test]
fn retained_reader_hides_foreign_tasks_and_rejects_scope_or_path_substitution() {
    let project = Project::new();
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    let history = logical_history(&project, &id);
    assert_denied(&read(&server, OTHER, &id, "collect"));
    for forged in [
        "../execution.sqlite".to_owned(),
        "a2a-../../execution.sqlite".to_owned(),
        format!("a2a-{}", "f".repeat(56)),
    ] {
        assert_denied(&read(&server, TOKEN, &forged, "collect"));
    }
    for extra in ["actor", "parent_invocation", "directory", "effect_id"] {
        let mut params = json!({"id":id,"key":"collect"});
        params[extra] = json!(PRIVATE);
        let response = exchange(
            &server,
            TOKEN,
            Some(&format!("{EXTENSION}, {READ_EXTENSION}")),
            "GetRetainedEffect",
            params,
        );
        assert!(response.2.get("error").is_some());
        assert!(response.2.get("result").is_none());
        assert!(!response.2.to_string().contains(PRIVATE));
    }
    let mut authentication: Value =
        serde_json::from_slice(&fs::read(project.root.join("authentication.json")).unwrap())
            .unwrap();
    authentication["principals"][0]["agents"] = json!([]);
    write(
        project.root.join("authentication-next.json"),
        &authentication,
    );
    fs::rename(
        project.root.join("authentication-next.json"),
        project.root.join("authentication.json"),
    )
    .unwrap();
    let denied = read(&server, TOKEN, &id, "collect");
    assert!(denied.0 == 401 || denied.2.get("error").is_some());
    assert!(denied.2.get("result").is_none());
    assert_eq!(logical_history(&project, &id), history);
    assert_eq!(project.calls(), 1);
}

#[test]
fn retained_reader_requires_current_resolve_rights_and_supports_same_scope_credential_rotation() {
    let project = Project::new();
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    assert_denied(&read(&server, TOKEN, &id, "collect"));
    assert_eq!(project.calls(), 1);
    drop(server);
    let project = Project::new();
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    let original = read(&server, TOKEN, &id, "collect").2;
    assert!(original.get("error").is_none(), "{original}");
    let rotated = "reader-rotated-synthetic-token-0123456789";
    project.authentication(rotated);
    assert_eq!(read(&server, TOKEN, &id, "collect").0, 401);
    assert_eq!(read(&server, rotated, &id, "collect").2, original);
    fs::rename(
        project.root.join("authority.json"),
        project.root.join("authority-offline.json"),
    )
    .unwrap();
    let denied = read(&server, rotated, &id, "collect");
    assert!(denied.0 == 401 || denied.2.get("error").is_some());
    assert!(denied.2.get("result").is_none());
    fs::rename(
        project.root.join("authority-offline.json"),
        project.root.join("authority.json"),
    )
    .unwrap();
    project.authority(2, "pv1", false);
    let denied = read(&server, rotated, &id, "collect");
    assert!(denied.0 == 401 || denied.2.get("error").is_some());
    assert!(denied.2.get("result").is_none());
    assert_eq!(project.calls(), 1);
}

#[test]
fn authorised_missing_effect_and_unknown_effect_never_become_unsent_or_executable() {
    let project = Project::new();
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "uncertain");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let task = server.rpc(TOKEN, "GetTask", json!({"id":id})).1;
        if task["result"]["metadata"]["mas"]["state"] == "unknown" {
            break;
        }
        assert!(Instant::now() < deadline, "{task}");
        thread::sleep(Duration::from_millis(20));
    }
    let history = logical_history(&project, &id);
    let unknown = read(&server, TOKEN, &id, "collect").2;
    assert!(unknown.get("error").is_none(), "{unknown}");
    assert_eq!(unknown["result"]["state"], "unknown");
    assert_eq!(unknown["result"]["host_settlement"], "unknown");
    assert_eq!(unknown["result"]["external_execution"], "unknown");
    let missing = read(&server, TOKEN, &id, "no-such-effect").2;
    assert!(missing.get("error").is_none(), "{missing}");
    for field in ["effect_id", "descriptor", "output", "evidence"] {
        assert!(missing["result"][field].is_null());
    }
    assert_eq!(missing["result"]["state"], "not-recorded");
    assert_eq!(missing["result"]["host_settlement"], "unknown");
    assert_eq!(missing["result"]["external_execution"], "unknown");
    assert_eq!(logical_history(&project, &id), history);
    assert_eq!(project.calls(), 1);
}

#[test]
fn live_effect_reader_observes_executing_without_owner_lock_recovery_or_dispatch() {
    let mut project = Project::new();
    pause_effect(&mut project);
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !project.root.join("effect-started").exists() {
        assert!(
            Instant::now() < deadline,
            "effect callback did not receive execute permission"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let history = logical_history(&project, &id);
    for _ in 0..3 {
        let response = read(&server, TOKEN, &id, "collect").2;
        assert!(response.get("error").is_none(), "{response}");
        let view = &response["result"];
        assert_eq!(view["state"], "executing");
        assert_eq!(view["host_settlement"], "pending");
        assert_eq!(view["external_execution"], "unknown");
        assert_eq!(view["payload_disposition"], "unavailable");
        for field in ["descriptor", "output", "evidence"] {
            assert!(view[field].is_null());
        }
    }
    assert_eq!(
        logical_history(&project, &id),
        history,
        "reader must not recover a live executing effect"
    );
    assert_eq!(
        project.calls(),
        0,
        "no external callback side effect has happened"
    );
    fs::write(project.root.join("effect-release"), "x").unwrap();
    server.completed_within(&id, Duration::from_secs(30));
    assert_eq!(
        read(&server, TOKEN, &id, "collect").2["result"]["host_settlement"],
        "completed"
    );
    assert_eq!(project.calls(), 1);
}

#[test]
fn current_payload_inspection_failure_withholds_known_facts_but_query_failure_denies_lookup() {
    let project = Project::new();
    // Restrict partial coverage to a retained output component, while the
    // authorised query stays complete. A global detector failure is a separate
    // query-processing failure and must not fabricate a not-recorded view.
    project.privacy_inspector("complete", 10_000);
    let path = project.root.join("inspection-worker.py");
    let source = fs::read_to_string(&path).unwrap().replace(
        "if mode=='partial':",
        "if mode=='partial' or mode=='payload-partial' and r['text']=='approved':",
    );
    fs::write(&path, &source).unwrap();
    let manifest_path = project.root.join("inspection-manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["worker_sha256"] = json!(digest(&source));
    let raw = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &raw).unwrap();
    let config_path = project.root.join("privacy.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let pin = Sha256::digest(&raw)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    config["detector"]["bundle_sha256"] = json!(pin);
    write(config_path, &config);
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    let history = logical_history(&project, &id);
    fs::write(project.root.join("inspection-mode"), "payload-partial").unwrap();
    let response = read(&server, TOKEN, &id, "collect").2;
    assert!(response.get("error").is_none(), "{response}");
    let view = &response["result"];
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "withheld");
    for field in ["descriptor", "output", "evidence"] {
        assert!(view[field].is_null());
    }
    fs::write(project.root.join("inspection-mode"), "partial").unwrap();
    assert_denied(&read(&server, TOKEN, &id, "collect"));
    assert_eq!(logical_history(&project, &id), history);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn corrupted_completion_or_root_association_returns_uniform_denial_without_repair() {
    let project = Project::new();
    permit_reads(&project);
    let server = project.start();
    let id = submitted(&project, &server, "available");
    server.completed_within(&id, Duration::from_secs(30));
    let db = rusqlite::Connection::open(
        project
            .root
            .join("state/runs")
            .join(&id)
            .join("execution.sqlite"),
    )
    .unwrap();
    let body: String = db
        .query_row("SELECT body FROM governed_completions", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut corrupt: Value = serde_json::from_str(&body).unwrap();
    corrupt["external_execution"] = json!("refused");
    db.execute(
        "UPDATE governed_completions SET body=?",
        [corrupt.to_string()],
    )
    .unwrap();
    assert_denied(&read(&server, TOKEN, &id, "collect"));
    assert_eq!(
        db.query_row("SELECT body FROM governed_completions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        corrupt.to_string()
    );
    db.execute("UPDATE governed_completions SET body=?", [&body])
        .unwrap();
    db.execute(
        "UPDATE steps SET target='other' WHERE kind='request' AND parent=''",
        [],
    )
    .unwrap();
    assert_denied(&read(&server, TOKEN, &id, "collect"));
    assert_eq!(
        db.query_row(
            "SELECT target FROM steps WHERE kind='request' AND parent=''",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "other"
    );
    assert_eq!(project.calls(), 1);
}
