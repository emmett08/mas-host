//! Exercise authenticated handling, revocation and replay through the real HTTP service.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TOKEN: &str = "governed-alice-synthetic-token-0123456789";
const OTHER: &str = "governed-bob-synthetic-token-012345678901";
const PRIVATE: &str = "private-person-4387@example.invalid";
const EXTENSION: &str = "urn:mas:host-task-service:1";

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn write(path: impl AsRef<std::path::Path>, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

struct Project {
    root: PathBuf,
    address: SocketAddr,
    bundle: PathBuf,
    descriptor: String,
}
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-governed-a2a-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("application.py"),r#"from mas_host import Context, EffectResult

def collect(ctx: Context, value: dict) -> dict:
    if value['mode'] == 'delayed':
        import time
        with open('started', 'w') as stream:
            stream.write('started')
        time.sleep(1.5)
    def action(invocation_id):
        with open('calls', 'a') as stream:
            stream.write('x')
        if value['mode'] == 'uncertain':
            raise RuntimeError('private-person-4387@example.invalid')
        if value['mode'] == 'withheld':
            output = {'approved': 'private-person-4387@example.invalid'}
        else:
            output = {'approved': True, 'private_note': 'private-person-4387@example.invalid'}
        return EffectResult(output, {'confirmed': True, 'execution':'completed', 'private_note': 'private-person-4387@example.invalid'})
    return ctx.effect('collect', {'format': 'fixture-effect/1', 'destination': 'fixture'}, action)
"#).unwrap();
        let input = object(
            json!({"mode":{"type":"string","enum":["available","withheld","delayed","uncertain"]}}),
            &["mode"],
        );
        let output = object(json!({"approved":{"type":"boolean"}}), &["approved"]);
        write(root.join("input.json"), &input);
        write(root.join("output.json"), &output);
        fs::write(
            root.join("agents.toml"),
            r#"version = "mas-agents/3"
[[agents]]
name = "collector"
operation = "collect/1"
input_schema = "input.json"
output_schema = "output.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "collect"
"#,
        )
        .unwrap();
        write(
            root.join("topology.json"),
            &json!({"version":"mas-topology/1","edges":[["client","collector"],["collector","client"]]}),
        );
        let build = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["build", "--project"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let built: Value = serde_json::from_slice(&build.stdout).unwrap();
        let bundle = PathBuf::from(built["bundle"].as_str().unwrap());
        let describe = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["describe", "--bundle"])
            .arg(&bundle)
            .output()
            .unwrap();
        assert!(
            describe.status.success(),
            "{}",
            String::from_utf8_lossy(&describe.stderr)
        );
        let descriptor = serde_json::from_slice::<Value>(&describe.stdout).unwrap()
            ["bundle_digest"]
            .as_str()
            .unwrap()
            .to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        fs::write(root.join("commitment.key"), [19u8; 32]).unwrap();
        fs::set_permissions(
            root.join("commitment.key"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        write(
            root.join("governance.json"),
            &json!({"version":"mas-governed-local/1",
            "authority_file":root.join("authority.json"),"commitment_key_file":root.join("commitment.key"),
            "commitment_key_id":"fixture-key-1","location":"local",
            "agents":[{"actor":"collector","destination":"local-worker","input_schema":input,"output_schema":output,
                "effects":[{"format":"fixture-effect/1","destination":"fixture",
                    "descriptor_schema":object(json!({"format":{"type":"string","enum":["fixture-effect/1"]},"destination":{"type":"string","enum":["fixture"]}}),&["format","destination"]),
                    "output_schema":output,"evidence_schema":object(json!({"confirmed":{"type":"boolean"},"execution":{"type":"string","enum":["completed"]}}),&["confirmed","execution"])}]}]}),
        );
        write(
            root.join("gateway.json"),
            &json!({"version":"mas-host-a2a/2","listen":address.to_string(),
            "public_url":format!("http://{address}/a2a"),"name":"Governed fixture",
            "governance_file":root.join("governance.json"),"authentication_file":root.join("authentication.json"),
            "exports":[{"agent":"collector","name":"Collector","description":"Synthetic structured evidence","public":false}]}),
        );
        let project = Self {
            root,
            address,
            bundle,
            descriptor,
        };
        project.authentication(TOKEN);
        project.authority(1, "pv1", true);
        project
    }
    fn authentication(&self, token: &str) {
        write(
            self.root.join("authentication-next.json"),
            &json!({"version":"mas-host-authentication/1","principals":[
            {"tenant":"tenant-a","principal":"alice","purpose":"test","grant_ref":"grant-a","token_sha256":digest(token),"agents":["collector"]},
            {"tenant":"tenant-b","principal":"alice","purpose":"test","grant_ref":"grant-b","token_sha256":digest(OTHER),"agents":["collector"]}]}),
        );
        fs::rename(
            self.root.join("authentication-next.json"),
            self.root.join("authentication.json"),
        )
        .unwrap();
    }
    fn authority(&self, epoch: u64, version: &str, enabled: bool) {
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 300_000;
        let mut grants = Vec::new();
        let mut policies = Vec::new();
        for (tenant, grant) in [("tenant-a", "grant-a"), ("tenant-b", "grant-b")] {
            if enabled || tenant == "tenant-b" {
                grants.push(json!({"version":"mas-current-authority/1","status":"active",
                "tenant":tenant,"principal":"alice","grant_ref":grant,"grant_epoch":epoch,"policy_version":version,
                "valid_from_ms":0,"expires_at_ms":expires}));
            }
            let rules= [("invoke","worker:collector","local-worker"),("deliver","alice","client"),("effect","operation:fixture-effect/1","fixture")]
                .into_iter().map(|(operation,recipient,destination)|json!({"operation":operation,"principal":"alice","actor":"collector",
                    "producer":"host","recipient":recipient,"purpose":"test","destination":destination,"location":"local",
                    "classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":16384})).collect::<Vec<_>>();
            policies.push(json!({"version":"mas-data-use-policy/1","id":format!("policy-{tenant}"),"tenant":tenant,
                "policy_version":version,"valid_from_ms":0,"expires_at_ms":expires,"rules":rules}));
        }
        if let Ok(previous) = fs::read(self.root.join("authority.json")) {
            let previous: Value = serde_json::from_slice(&previous).unwrap();
            if previous["policies"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["policy_version"] == version)
            {
                policies = previous["policies"].as_array().unwrap().clone();
            }
        }
        write(
            self.root.join("authority-next.json"),
            &json!({"version":"mas-governed-authority/1","grants":grants,"policies":policies}),
        );
        fs::rename(
            self.root.join("authority-next.json"),
            self.root.join("authority.json"),
        )
        .unwrap();
    }
    fn start(&self) -> Server {
        let child = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["serve-a2a", "--bundle"])
            .arg(&self.bundle)
            .arg("--state")
            .arg(self.root.join("state"))
            .arg("--config")
            .arg(self.root.join("gateway.json"))
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.root.join("stderr.log")).unwrap())
            .spawn()
            .unwrap();
        let mut server = Server {
            child,
            address: self.address,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if TcpStream::connect(self.address).is_ok() {
                return server;
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "{}",
                fs::read_to_string(self.root.join("stderr.log")).unwrap()
            );
            assert!(Instant::now() < deadline, "service startup deadline");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn send(&self, mode: &str, secret: &str) -> Value {
        json!({"message":{"messageId":"fixed-original-id","role":"ROLE_USER","contextId":PRIVATE,
            "parts":[{"data":{"agent":"collector","descriptorDigest":self.descriptor,
                "input":{"mode":mode,"private_note":secret}},"mediaType":"application/json"}]},
            "configuration":{"returnImmediately":true}})
    }
    fn calls(&self) -> usize {
        fs::read(self.root.join("calls")).unwrap_or_default().len()
    }
    fn assert_private_absent(&self) {
        fn walk(path: &std::path::Path) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path);
                } else {
                    let bytes = fs::read(&path).unwrap();
                    assert!(
                        !bytes
                            .windows(PRIVATE.len())
                            .any(|w| w == PRIVATE.as_bytes()),
                        "private content in {}",
                        path.display()
                    );
                }
            }
        }
        walk(&self.root.join("state"));
        assert!(!fs::read_to_string(self.root.join("stderr.log"))
            .unwrap()
            .contains(PRIVATE));
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Server {
    child: Child,
    address: SocketAddr,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn rpc(&self, token: &str, method: &str, params: Value) -> (u16, Value) {
        let body = json!({"jsonrpc":"2.0","id":"safe-request-id","method":method,"params":params})
            .to_string();
        let mut stream = TcpStream::connect(self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(stream,"POST /a2a HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nA2A-Version: 1.0\r\nA2A-Extensions: {EXTENSION}\r\nContent-Length: {}\r\n\r\n{body}",self.address,body.len()).unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        (
            head.split_whitespace().nth(1).unwrap().parse().unwrap(),
            serde_json::from_str(body).unwrap(),
        )
    }
    fn completed(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (_, response) = self.rpc(TOKEN, "GetTask", json!({"id":id}));
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
}
fn rejected(response: (u16, Value)) {
    assert!(
        response.0 >= 400 || response.1.get("error").is_some(),
        "{}",
        response.1
    );
}

#[test]
fn governed_ingress_minimises_before_persistence_preserves_original_conflicts_and_replays() {
    let project = Project::new();
    let server = project.start();
    let request = project.send("available", PRIVATE);
    let (_, accepted) = server.rpc(TOKEN, "SendMessage", request.clone());
    assert!(accepted.get("error").is_none(), "{accepted}");
    let id = accepted["result"]["task"]["id"].as_str().unwrap();
    let completed = server.completed(id);
    assert_eq!(
        completed["artifacts"][0]["parts"][0]["data"],
        json!({"approved":true})
    );
    assert!(!completed.to_string().contains(PRIVATE));
    assert_eq!(project.calls(), 1);
    rejected(server.rpc(OTHER, "GetTask", json!({"id":id})));
    rejected(server.rpc(
        TOKEN,
        "SendMessage",
        project.send("available", "different-original-private-value"),
    ));
    let (_, repeated) = server.rpc(TOKEN, "SendMessage", request.clone());
    assert_eq!(repeated["result"]["task"]["id"], id);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
    drop(server);
    let restarted = project.start();
    let (_, replayed) = restarted.rpc(TOKEN, "SendMessage", request);
    assert_eq!(
        replayed["result"]["task"]["artifacts"],
        completed["artifacts"]
    );
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn current_authority_controls_cached_delivery_and_same_policy_credential_rotation() {
    let project = Project::new();
    let server = project.start();
    let (_, accepted) = server.rpc(TOKEN, "SendMessage", project.send("available", PRIVATE));
    assert!(accepted.get("error").is_none(), "{accepted}");
    let id = accepted["result"]["task"]["id"].as_str().unwrap();
    server.completed(id);
    fs::rename(
        project.root.join("authority.json"),
        project.root.join("authority-offline.json"),
    )
    .unwrap();
    rejected(server.rpc(TOKEN, "GetTask", json!({"id":id})));
    fs::rename(
        project.root.join("authority-offline.json"),
        project.root.join("authority.json"),
    )
    .unwrap();
    project.authority(2, "pv1", false);
    rejected(server.rpc(TOKEN, "GetTask", json!({"id":id})));
    project.authority(3, "pv1", true);
    let rotated = "rotated-alice-synthetic-token-012345678901";
    project.authentication(rotated);
    rejected(server.rpc(TOKEN, "GetTask", json!({"id":id})));
    let (_, retrieved) = server.rpc(rotated, "GetTask", json!({"id":id}));
    assert!(
        retrieved.get("error").is_none(),
        "same-policy rotation should preserve retained identity: {retrieved}"
    );
    project.authority(4, "pv2", true);
    rejected(server.rpc(rotated, "GetTask", json!({"id":id})));
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn known_effect_completion_with_prohibited_output_survives_withholding_and_restart() {
    let project = Project::new();
    let server = project.start();
    let request = project.send("withheld", PRIVATE);
    let (_, accepted) = server.rpc(TOKEN, "SendMessage", request.clone());
    assert!(accepted.get("error").is_none(), "{accepted}");
    let id = accepted["result"]["task"]["id"].as_str().unwrap();
    let task = server.completed(id);
    assert_eq!(
        task["artifacts"][0]["parts"][0]["data"]["payload_disposition"],
        "withheld"
    );
    assert_eq!(
        task["artifacts"][0]["parts"][0]["data"]["external_execution"],
        "completed"
    );
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
    drop(server);
    let restarted = project.start();
    let (_, replayed) = restarted.rpc(TOKEN, "SendMessage", request);
    assert_eq!(replayed["result"]["task"]["artifacts"], task["artifacts"]);
    assert_eq!(project.calls(), 1);
    let db = rusqlite::Connection::open(
        project
            .root
            .join("state/runs")
            .join(id)
            .join("execution.sqlite"),
    )
    .unwrap();
    let completed: i64 = db
        .query_row(
            "SELECT count(*) FROM steps WHERE kind='effect' AND state='completed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(completed, 1);
    project.assert_private_absent();
}

#[test]
fn changed_grant_reference_cannot_reset_request_identity_and_key_changes_stop_use() {
    let project = Project::new();
    let server = project.start();
    let request = project.send("available", PRIVATE);
    let (_, accepted) = server.rpc(TOKEN, "SendMessage", request.clone());
    assert!(accepted.get("error").is_none(), "{accepted}");
    let id = accepted["result"]["task"]["id"].as_str().unwrap();
    server.completed(id);
    let mut authentication: Value =
        serde_json::from_slice(&fs::read(project.root.join("authentication.json")).unwrap())
            .unwrap();
    authentication["principals"][0]["grant_ref"] = json!("grant-replacement");
    let mut authority: Value =
        serde_json::from_slice(&fs::read(project.root.join("authority.json")).unwrap()).unwrap();
    authority["grants"][0]["grant_ref"] = json!("grant-replacement");
    write(project.root.join("authority.json"), &authority);
    write(project.root.join("authentication.json"), &authentication);
    rejected(server.rpc(TOKEN, "SendMessage", request.clone()));
    assert_eq!(project.calls(), 1);
    fs::write(project.root.join("commitment.key"), [20u8; 32]).unwrap();
    rejected(server.rpc(TOKEN, "SendMessage", request));
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn authentication_revocation_stops_later_effect_admission() {
    let project = Project::new();
    let server = project.start();
    let (_, accepted) = server.rpc(TOKEN, "SendMessage", project.send("delayed", PRIVATE));
    assert!(accepted.get("error").is_none(), "{accepted}");
    let id = accepted["result"]["task"]["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !project.root.join("started").exists() {
        assert!(Instant::now() < deadline, "worker did not start");
        thread::sleep(Duration::from_millis(20));
    }
    let mut authentication: Value =
        serde_json::from_slice(&fs::read(project.root.join("authentication.json")).unwrap())
            .unwrap();
    authentication["principals"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    write(
        project.root.join("authentication-next.json"),
        &authentication,
    );
    fs::rename(
        project.root.join("authentication-next.json"),
        project.root.join("authentication.json"),
    )
    .unwrap();
    rejected(server.rpc(TOKEN, "GetTask", json!({"id":id})));
    let db = rusqlite::Connection::open(project.root.join("state/service.sqlite")).unwrap();
    loop {
        let state: String = db
            .query_row("SELECT state FROM tasks WHERE id=?", [id], |r| r.get(0))
            .unwrap();
        if state == "unknown" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "revoked execution did not stop: {state}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(project.calls(), 0);
    project.assert_private_absent();
}

#[test]
fn lost_effect_response_keeps_uncertainty_and_authorised_recovery_ids() {
    let project = Project::new();
    let server = project.start();
    let mut request = project.send("uncertain", PRIVATE);
    request["configuration"]["returnImmediately"] = json!(false);
    let (_, response) = server.rpc(TOKEN, "SendMessage", request.clone());
    assert!(response.get("error").is_some(), "{response}");
    assert!(!response.to_string().contains(PRIVATE));
    let id = response["error"]["data"][0]["metadata"]["taskId"]
        .as_str()
        .expect("authorised recovery id")
        .to_owned();
    let (_, task) = server.rpc(TOKEN, "GetTask", json!({"id":id}));
    assert_eq!(task["result"]["metadata"]["mas"]["state"], "unknown");
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
    drop(server);
    let restarted = project.start();
    let (_, replay) = restarted.rpc(TOKEN, "SendMessage", request);
    assert!(replay.get("error").is_some());
    assert_eq!(project.calls(), 1);
    project.authority(2, "pv1", false);
    let denied = restarted.rpc(TOKEN, "GetTask", json!({"id":id}));
    rejected(denied.clone());
    assert!(denied.1["error"].get("data").is_none());
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}
