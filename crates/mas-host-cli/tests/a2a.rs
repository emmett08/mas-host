//! Exercise the optional gateway through its actual CLI and HTTP boundary.
//! A deterministic workflow invokes a local provider fixture; no API key is used.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EXTENSION: &str = "urn:mas:host-task-service:1";
const ALICE: &str = "alice-test-token-012345678901234567890";
const BOB: &str = "bob-test-token-01234567890123456789012";

#[derive(Clone, Copy)]
enum Provider {
    Completed,
    Delayed,
    Unknown,
}

struct Project {
    root: PathBuf,
    bundle: PathBuf,
    address: SocketAddr,
}

impl Project {
    fn new(provider: Provider) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-a2a-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("instructions.md"), "private-review-prompt-marker").unwrap();
        write(
            &root.join("input-schema.json"),
            &json!({"type":"object","properties":{"proposal":{"type":"string"}},
                "required":["proposal"],"additionalProperties":false}),
        );
        write(
            &root.join("output-schema.json"),
            &json!({"type":"object","properties":{"approved":{"type":"boolean"}},
                "required":["approved"],"additionalProperties":false}),
        );
        let response = match provider {
            Provider::Unknown => json!({"version":"mas-provider/1","status":"unknown","evidence":{}}),
            _ => json!({"version":"mas-provider/1","status":"completed", "output":{"approved":true},"evidence":{}}),
        }.to_string();
        let pause = if matches!(provider, Provider::Delayed) {
            "/bin/sleep 5; "
        } else {
            ""
        };
        write(
            &root.join("agents.json"),
            &json!({"version":"mas-agents/3",
            "connections":{"fixture":{"kind":"command","command":["/bin/sh","-c",
                format!("/bin/cat >/dev/null; printf x >> \"$1\"; {pause}printf '%s' '{response}'"),
                "private-provider-command-marker",root.join("calls")]}},
            "agents":[
                {"name":"review","operation":"review/1","input_schema":"input-schema.json",
                    "output_schema":"output-schema.json",
                    "implementation":{"kind":"workflow","plan":"plan.json"}},
                {"name":"assessor","operation":"assess/1","output_schema":"output-schema.json",
                    "implementation":{"kind":"llm","connection":"fixture","model":"fixture-model",
                        "instructions":"instructions.md","max_output_tokens":128}}
            ]}),
        );
        write(
            &root.join("plan.json"),
            &json!({"type":"sequence","steps":[
                {"type":"request","key":"assess","target":"assessor","input":{"$input":""}},
                {"type":"value","output":{"approved":{"$previous":"/approved"}}}
            ]}),
        );
        write(
            &root.join("topology.json"),
            &json!({"version":"mas-topology/1","edges":[
                ["client","review"],["review","client"],
                ["review","assessor"],["assessor","review"]]}),
        );
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["build", "--project"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let bundle = PathBuf::from(result["bundle"].as_str().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        write(
            &root.join("gateway.json"),
            &json!({"version":"mas-host-a2a/1","listen":address.to_string(),
            "public_url":format!("http://{address}/a2a"),"name":"Nehemiah","max_tasks":1,"max_active":1,
            "exports":[{"agent":"review","name":"Review","description":"Assess a proposal","public":true}],
            "principals":[
                {"name":"alice","token_sha256":digest(ALICE),"agents":["review"]},
                {"name":"bob","token_sha256":digest(BOB),"agents":["review"]}
            ]}),
        );
        Self {
            root,
            bundle,
            address,
        }
    }

    fn descriptor(&self) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["describe", "--bundle"])
            .arg(&self.bundle)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn calls(&self) -> usize {
        fs::read(self.root.join("calls")).unwrap_or_default().len()
    }

    fn start(&self) -> Server {
        let errors = self.root.join("gateway-stderr");
        let child = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["serve-a2a", "--bundle"])
            .arg(&self.bundle)
            .arg("--state")
            .arg(self.root.join("state"))
            .arg("--config")
            .arg(self.root.join("gateway.json"))
            .stdout(Stdio::null())
            .stderr(fs::File::create(&errors).unwrap())
            .spawn()
            .unwrap();
        let mut server = Server {
            child,
            address: self.address,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(self.address).is_ok() {
                return server;
            }
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "gateway exited {status}: {}",
                    fs::read_to_string(&errors).unwrap()
                );
            }
            assert!(
                Instant::now() < deadline,
                "gateway did not listen: {}",
                fs::read_to_string(&errors).unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        }
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

impl Server {
    fn http(&self, method: &str, path: &str, headers: &str, body: &str) -> (u16, Value) {
        let (status, _, response) = self.exchange(method, path, headers, body);
        (status, response)
    }

    fn exchange(
        &self,
        method: &str,
        path: &str,
        headers: &str,
        body: &str,
    ) -> (u16, String, Value) {
        let mut stream = TcpStream::connect(self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}", self.address, body.len()).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let response = String::from_utf8(bytes).unwrap();
        let (head, body) = response
            .split_once("\r\n\r\n")
            .expect("valid HTTP response");
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (
            status,
            head.to_owned(),
            serde_json::from_str(body).unwrap_or_else(|_| json!({"body":body})),
        )
    }

    fn rpc_with(&self, token: &str, method: &str, params: Value, extension: bool) -> (u16, Value) {
        let mut headers = format!("Authorization: Bearer {token}\r\nA2A-Version: 1.0\r\n");
        if extension {
            headers.push_str(&format!("A2A-Extensions: {EXTENSION}\r\n"));
        }
        self.http(
            "POST",
            "/a2a",
            &headers,
            &json!({"jsonrpc":"2.0","id":"request-1","method":method,"params":params}).to_string(),
        )
    }

    fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let (status, response) = self.rpc_with(token, method, params, true);
        assert_eq!(status, 200, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn task(&self, id: &str) -> Value {
        self.rpc(ALICE, "GetTask", json!({"id":id}))
    }

    fn completed(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let task = self.task(id);
            if task["metadata"]["mas"]["state"] == "completed" {
                return task;
            }
            assert!(
                !["failed", "unknown", "cancelled"]
                    .contains(&task["metadata"]["mas"]["state"].as_str().unwrap()),
                "{task}"
            );
            assert!(Instant::now() < deadline, "task did not complete: {task}");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn send(message_id: &str, descriptor_digest: &str) -> Value {
    json!({"message":{"messageId":message_id,"role":"ROLE_USER","parts":[{
        "data":{"agent":"review","input":{"proposal":"release"},"descriptorDigest":descriptor_digest},
        "mediaType":"application/json"}]},"configuration":{"returnImmediately":true}})
}

fn denied(response: (u16, Value)) {
    assert!(
        response.0 >= 400 || response.1.get("error").is_some(),
        "unexpected acceptance: {}",
        response.1
    );
}

#[test]
fn descriptor_exposes_contracts_without_execution_configuration() {
    let project = Project::new(Provider::Completed);
    let descriptor = project.descriptor();
    assert_eq!(descriptor["version"], "mas-host-descriptor/1");
    assert_eq!(descriptor["bundle_digest"].as_str().unwrap().len(), 64);
    let agents = descriptor["agents"].as_array().unwrap();
    let review = agents
        .iter()
        .find(|agent| agent["name"] == "review")
        .unwrap();
    assert_eq!(review["operation"], "review/1");
    assert_eq!(review["input_schema"]["required"], json!(["proposal"]));
    assert_eq!(review["output_schema"]["required"], json!(["approved"]));
    let encoded = descriptor.to_string();
    for private in [
        "private-review-prompt-marker",
        "private-provider-command-marker",
        "fixture-model",
        project.root.to_str().unwrap(),
    ] {
        assert!(!encoded.contains(private), "descriptor disclosed {private}");
    }
    assert_eq!(project.calls(), 0, "discovery must not invoke workers");
}

#[test]
fn gateway_enforces_profile_owner_and_exact_retry_and_retains_completed_tasks() {
    let project = Project::new(Provider::Completed);
    let descriptor = project.descriptor();
    let digest = descriptor["bundle_digest"].as_str().unwrap();
    let server = project.start();
    let (status, card) = server.http("GET", "/.well-known/agent-card.json", "", "");
    assert_eq!(status, 200);
    assert_eq!(card["name"], "Nehemiah");
    assert!(card.to_string().contains("Review"));
    assert!(!card.to_string().contains("assessor"));
    assert!(!card.to_string().contains("private-review-prompt-marker"));
    let extended = server.rpc(ALICE, "GetExtendedAgentCard", json!({}));
    let advertised = &extended["capabilities"]["extensions"][0]["params"]["descriptor"];
    assert_eq!(advertised["version"], "mas-host-descriptor/1");
    assert_eq!(advertised["bundle_digest"], digest);
    assert_eq!(advertised["agents"].as_array().unwrap().len(), 1);
    assert_eq!(advertised["agents"][0]["name"], "review");

    denied(server.rpc_with("wrong-token", "GetExtendedAgentCard", json!({}), true));
    let request = send("assessment-1", digest);
    let headers = format!(
        "Authorization: Bearer {ALICE}\r\nA2A-Version: 1.0\r\nA2A-Extensions: {EXTENSION}\r\n"
    );
    for invalid_id in [json!(true), json!({"id":"nested"}), json!([]), Value::Null] {
        let (status, rejected) = server.http(
            "POST",
            "/a2a",
            &headers,
            &json!({"jsonrpc":"2.0","id":invalid_id,"method":"SendMessage","params":request})
                .to_string(),
        );
        assert_eq!(status, 200, "{rejected}");
        assert_eq!(rejected["error"]["code"], -32600, "{rejected}");
        assert_eq!(
            rejected["id"],
            Value::Null,
            "invalid request IDs must not be echoed"
        );
    }
    let (status, missing_method) = server.http(
        "POST",
        "/a2a",
        &headers,
        &json!({"jsonrpc":"2.0","id":"missing-method","params":request}).to_string(),
    );
    assert_eq!(status, 200);
    assert_eq!(missing_method["error"]["code"], -32600);
    assert_eq!(missing_method["id"], "missing-method");
    for invalid_params in [json!([]), json!("invalid")] {
        let (status, rejected) = server.rpc_with(ALICE, "SendMessage", invalid_params, true);
        assert_eq!(status, 200);
        assert_eq!(rejected["error"]["code"], -32602, "{rejected}");
        assert_eq!(rejected["id"], "request-1");
    }
    let mut mixed_modes = request.clone();
    mixed_modes["configuration"]["acceptedOutputModes"] = json!(["application/json", true]);
    let (_, rejected) = server.rpc_with(ALICE, "SendMessage", mixed_modes, true);
    assert_eq!(rejected["error"]["code"], -32005);
    let (status, malformed_json) = server.http("POST", "/a2a", &headers, "{");
    assert_eq!(status, 200);
    assert_eq!(malformed_json["error"]["code"], -32700);
    assert_eq!(malformed_json["id"], Value::Null);
    let (status, duplicate_header) = server.http(
        "POST",
        "/a2a",
        &format!("{headers}A2A-Version: 1.0\r\n"),
        &json!({"jsonrpc":"2.0","id":"duplicate-header","method":"SendMessage","params":request})
            .to_string(),
    );
    assert_eq!(status, 400);
    assert_eq!(duplicate_header["error"], "duplicate_header");
    denied(server.http("POST", "/a2a", &format!("Authorization: Bearer {ALICE}\r\nA2A-Version: 9.0\r\nA2A-Extensions: {EXTENSION}\r\n"),
        &json!({"jsonrpc":"2.0","id":"unsupported","method":"SendMessage","params":request}).to_string()));
    denied(server.rpc_with("wrong-token", "SendMessage", request.clone(), true));
    let missing_extension = server.rpc_with(ALICE, "SendMessage", request.clone(), false);
    assert_eq!(missing_extension.1["error"]["code"], -32008);
    denied(missing_extension);
    denied(server.rpc_with(ALICE, "SendMessage", send("stale", &"0".repeat(64)), true));
    let mut private = request.clone();
    private["message"]["parts"][0]["data"]["agent"] = json!("assessor");
    denied(server.rpc_with(ALICE, "SendMessage", private, true));
    assert_eq!(project.calls(), 0, "rejected requests must not execute");

    let result = server.rpc(ALICE, "SendMessage", request.clone());
    let id = result["task"]["id"].as_str().unwrap();
    let task = server.completed(id);
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(
        task["artifacts"][0]["parts"][0]["data"],
        json!({"approved":true})
    );
    assert_eq!(project.calls(), 1);
    let repeated = server.rpc(ALICE, "SendMessage", request.clone());
    assert_eq!(repeated["task"]["id"], id);
    let mut changed = request.clone();
    changed["message"]["parts"][0]["data"]["input"]["proposal"] = json!("different");
    denied(server.rpc_with(ALICE, "SendMessage", changed, true));
    denied(server.rpc_with(BOB, "GetTask", json!({"id":id}), true));
    denied(server.rpc_with(BOB, "CancelTask", json!({"id":id}), true));
    denied(server.rpc_with(ALICE, "GetTask", json!({"id":id}), false));
    denied(server.rpc_with(
        ALICE,
        "SendMessage",
        send("beyond-retention-limit", digest),
        true,
    ));
    assert_eq!(project.calls(), 1);
    drop(server);

    let restarted = project.start();
    assert_eq!(restarted.task(id)["artifacts"], task["artifacts"]);
    assert_eq!(
        restarted.rpc(ALICE, "SendMessage", request)["task"]["id"],
        id
    );
    assert_eq!(
        project.calls(),
        1,
        "restart or retry repeated provider work"
    );
}

#[test]
fn interrupted_task_becomes_unknown_and_retry_does_not_repeat_work() {
    let project = Project::new(Provider::Delayed);
    let descriptor = project.descriptor();
    let request = send(
        "interrupted-1",
        descriptor["bundle_digest"].as_str().unwrap(),
    );
    let server = project.start();
    let result = server.rpc(ALICE, "SendMessage", request.clone());
    let id = result["task"]["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while project.calls() == 0 {
        assert!(Instant::now() < deadline, "provider was not started");
        thread::sleep(Duration::from_millis(20));
    }
    let repeated = server.rpc(ALICE, "SendMessage", request.clone());
    assert_eq!(repeated["task"]["id"], id);
    denied(server.rpc_with(ALICE, "CancelTask", json!({"id":id}), true));
    let cancelling = server.task(id);
    assert_eq!(cancelling["metadata"]["mas"]["cancelRequested"], true);
    assert_ne!(cancelling["status"]["state"], "TASK_STATE_CANCELED");
    assert_eq!(
        project.calls(),
        1,
        "concurrent retry repeated provider work"
    );
    drop(server);
    let restarted = project.start();
    let task = restarted.task(id);
    assert_eq!(task["metadata"]["mas"]["state"], "unknown");
    assert_eq!(task["metadata"]["mas"]["cancelRequested"], true);
    assert_eq!(task["status"]["state"], "TASK_STATE_UNSPECIFIED");
    let repeated = restarted.rpc(ALICE, "SendMessage", request);
    assert_eq!(repeated["task"]["id"], id);
    assert_eq!(repeated["task"]["metadata"]["mas"]["state"], "unknown");
    assert_eq!(project.calls(), 1, "uncertain execution was repeated");
}

#[test]
fn blocking_unknown_outcome_preserves_task_identity_and_negotiated_extension() {
    let project = Project::new(Provider::Unknown);
    let descriptor = project.descriptor();
    let server = project.start();
    let mut request = send(
        "blocking-unknown-1",
        descriptor["bundle_digest"].as_str().unwrap(),
    );
    request["configuration"]["returnImmediately"] = json!(false);
    request["message"]["contextId"] = json!("caller-context-unknown-1");
    let (status, headers, response) = server.exchange("POST", "/a2a",
        &format!("Authorization: Bearer {ALICE}\r\nA2A-Version: 1.0\r\nA2A-Extensions: {EXTENSION},urn:unknown:optional\r\n"),
        &json!({"jsonrpc":"2.0","id":"blocking-request","method":"SendMessage","params":request}).to_string());
    assert_eq!(status, 200, "{response}");
    let activated: Vec<_> = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("a2a-extensions"))
        .map(|(_, value)| value.trim())
        .collect();
    assert_eq!(
        activated,
        vec![EXTENSION],
        "only supported extensions may be acknowledged"
    );
    assert_eq!(response["error"]["code"], -32603, "{response}");
    let information = response["error"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["@type"] == "type.googleapis.com/google.rpc.ErrorInfo")
        .unwrap();
    assert_eq!(information["reason"], "RETAINED_TASK_REQUIRES_OBSERVATION");
    let identity = &information["metadata"];
    let id = identity["taskId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .unwrap();
    assert_eq!(identity["contextId"], "caller-context-unknown-1");

    // A slow runner may return its bounded response deadline first. Both
    // outcomes must identify retained work; observe that work rather than
    // inventing a replacement request or treating uncertainty as completion.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let task = server.task(id);
        assert_eq!(task["id"], id);
        assert_eq!(task["contextId"], identity["contextId"]);
        if task["metadata"]["mas"]["state"] == "unknown" {
            assert_eq!(task["status"]["state"], "TASK_STATE_UNSPECIFIED");
            assert!(task.get("artifacts").is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider uncertainty was not retained: {task}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let (status, repeated) = server.rpc_with(ALICE, "SendMessage", request, true);
    assert_eq!(status, 200);
    assert_eq!(
        repeated["error"]["message"],
        "outcome_unknown_poll_original_task"
    );
    assert_eq!(repeated["error"]["data"], response["error"]["data"]);
    assert_eq!(
        project.calls(),
        1,
        "blocking retry repeated uncertain provider work"
    );
}
