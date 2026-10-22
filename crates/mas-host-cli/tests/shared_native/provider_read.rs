//! Actual anonymous loopback HTTP under native plus registered quota authority.
use super::*;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

struct Endpoint {
    address: SocketAddr,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Endpoint {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let count = calls.clone();
        let stopping = stop.clone();
        let thread = thread::spawn(move || {
            for accepted in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let mut stream = accepted.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                    let mut byte = [0u8; 1];
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                assert!(head.starts_with("GET /"));
                assert!(!head.to_lowercase().contains("authorization:"));
                count.fetch_add(1, Ordering::SeqCst);
                let (status, extra) = if head.starts_with("GET /throttle ") {
                    ("429 Too Many Requests", "Retry-After: 2\r\n")
                } else if head.starts_with("GET /gone ") {
                    ("410 Gone", "")
                } else if head.starts_with("GET /conditional ") {
                    assert!(head
                        .to_lowercase()
                        .contains("if-none-match: \"registered-etag\""));
                    ("304 Not Modified", "")
                } else {
                    ("200 OK", "")
                };
                let body = "{\"status\":{\"ready\":2},\"private\":\"suppressed-fixture-private@example.invalid\"}";
                write!(stream,"HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: application/json\r\n{extra}X-Private: suppressed-fixture-private@example.invalid\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            address,
            calls,
            stop,
            thread: Some(thread),
        }
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.thread.take() {
            worker.join().unwrap();
        }
    }
}

struct ProviderFixture {
    native: Fixture,
    identity: String,
    policy: Value,
}
impl ProviderFixture {
    fn new(endpoint: &Endpoint, max_requests: i64) -> Self {
        let mut native = Fixture::new();
        let policy = json!({"format":"mas-provider-circuit/2","resource_key":"provider/loopback-fixture",
            "failure_threshold":2,"open_seconds":5,"half_open_seconds":2,
            "window_ms":86_400_000,"max_requests":max_requests});
        let mut requests = json!({});
        for case in ["read", "conditional", "gone", "throttle"] {
            requests[case] = json!({"version":"mas-provider-public-get/1","provider":if case=="gone"{"kubernetes"}else{"github"},
                "source_ref":format!("fixture/{case}"),"quota_scope":policy["resource_key"],
                "endpoint":format!("http://{}/{case}",endpoint.address),
                "operation":if case=="gone"{"watch"}else{"read"},
                "if_none_match":if case=="conditional"{json!("\"registered-etag\"")}else{Value::Null},
                "allow_loopback_http":true,"timeout_ms":1000,"max_body_bytes":32768,
                "projection":{"ready":{"pointer":"/status/ready","type":"integer"}}});
        }
        let source = format!("import json\nfrom read_worker import PublicGET\nREQUESTS=json.loads({:?})\ndef apply(ctx,value):\n return PublicGET(REQUESTS[value['case']]).read(deadline_ms=ctx.deadline_ms)\n",requests.to_string());
        fs::write(native.root.join("application.py"), &source).unwrap();
        fs::write(
            native.root.join("provider.py"),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../adapters/acquisition/provider.py"
            )),
        )
        .unwrap();
        fs::write(
            native.root.join("read_worker.py"),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../adapters/acquisition/read_worker.py"
            )),
        )
        .unwrap();
        write(
            &native.root.join("input.json"),
            &json!({"type":"object","properties":{"case":{"type":"string","enum":["read","conditional","gone","throttle"]}},"required":["case"],"additionalProperties":false}),
        );
        write(&native.root.join("output.json"), &json!({"type":"object"}));
        write(
            &native.root.join("agents.json"),
            &json!({"version":"mas-agents/3","limits":{"max_calls":1,"wall_seconds":120},"agents":[{"name":"native","operation":"registered-public-read/1","input_schema":"input.json","output_schema":"output.json","implementation":{"kind":"native","language":"python","source":"application.py","handler":"apply"}}]}),
        );
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .args(["build", "--project"])
            .arg(&native.root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        native.bundle = PathBuf::from(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["bundle"]
                .as_str()
                .unwrap(),
        );
        let bundle: Value = serde_json::from_slice(&fs::read(&native.bundle).unwrap()).unwrap();
        let bundle_digest = mas_host_core::describe(&bundle).unwrap()["bundle_digest"].clone();
        let files: Vec<Value> = ["application.py", "provider.py", "read_worker.py"]
            .iter()
            .map(|name| {
                let bytes = fs::read(native.root.join(name)).unwrap();
                json!({"path":name,"sha256":digest(&bytes),"bytes":bytes})
            })
            .collect();
        let package=json!({"format":"mas-native-implementation/1","bundle_digest":bundle_digest,"files":files}).to_string();
        let implementation = digest(package.as_bytes());
        let mut repo = native.repo();
        repo.install_provider_quota_index().unwrap();
        let parts: Vec<&[u8]> = package.as_bytes().chunks(20_000).collect();
        for (part, data) in parts.iter().enumerate() {
            repo.host_command(
                &format!("provider-chunk-{part}"),
                "definition.chunk",
                &json!({"digest":implementation,"part":part,"data":std::str::from_utf8(data).unwrap()}),
            )
            .unwrap();
        }
        repo.host_command(
            "provider-package",
            "definition.manifest",
            &json!({"digest":implementation,"parts":parts.len()}),
        )
        .unwrap();
        let descriptor = json!({"implementation_digest":implementation,"bundle_digest":bundle_digest,"resource_keys":[],
            "provider_quota":policy,"timing_policy":{"format":"mas-timing-policy/1","business_seconds":120,"active_seconds":30,"queue_seconds":120,"pause_seconds":120}});
        let identity = digest(descriptor.to_string().as_bytes());
        repo.host_command("provider-definition","definition.register",&json!({"kind":"workflow","name":"native","identity":identity,"descriptor":descriptor,"implementation_digest":implementation,"current":true})).unwrap();
        repo.host_command(
            "provider-task-policy",
            "task.configure",
            &json!({"max_tasks":10,"max_pending":10,"max_revisions":1}),
        )
        .unwrap();
        repo.host_command(
            "provider-principal",
            "task.configure_principal",
            &json!({"principal":"tester","max_tasks":10,"max_pending":10}),
        )
        .unwrap();
        repo.host_command("provider-scheduler","task.configure_scheduler",&json!({"policy":{"max_workers":1,"max_per_principal":1,"workflows":{},"resources":{}}})).unwrap();
        Self {
            native,
            identity,
            policy,
        }
    }
    fn submit(&self, id: &str, case: &str) {
        let input = json!({"case":case});
        self.native.repo().host_command(&format!("provider-submit-{id}"),"task.submit",&json!({"principal":"tester","namespace":"mas-unified/1","epoch":"provider-fixture","key":id,"digest":digest(input.to_string().as_bytes()),"task":id,"workflow":"native","identity":self.identity,"expected_identity":self.identity,"input":input,"metadata":{}})).unwrap();
    }
    fn drain(&self, turn: &str) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .env("MAS_POSTGRES_DSN", &self.native.dsn)
            .args(["shared-drain", "--authority"])
            .arg(self.native.root.join("authority.json"))
            .arg("--bundle")
            .arg(&self.native.bundle)
            .arg("--state")
            .arg(self.native.root.join("lifetime"))
            .args([
                "--agent",
                "native",
                "--workflow",
                "native",
                "--workflow-identity",
                &self.identity,
                "--turn",
                turn,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn task(&self, id: &str) -> Value {
        self.native
            .repo()
            .host_query("task.inspect", &json!({"principal":"tester","task":id}))
            .unwrap()
    }
}

#[test]
#[ignore = "requires actual PostgreSQL and a native Python source worker"]
fn registered_public_reads_consume_native_and_provider_credit_once() {
    let endpoint = Endpoint::start();
    let f = ProviderFixture::new(&endpoint, 3);
    for (index, case) in ["read", "conditional", "gone"].iter().enumerate() {
        let id = format!("provider-{case}");
        f.submit(&id, case);
        assert_eq!(
            f.drain(&format!("provider-turn-{index}"))["fresh_claims"],
            1
        );
        let task = f.task(&id);
        assert_eq!(task["state"], "completed");
        let output = &task["result"];
        assert_eq!(
            output["provider_observation"]["quota_scope"],
            f.policy["resource_key"]
        );
        match *case {
            "read" => assert_eq!(output["projection"], json!({"ready":2})),
            "conditional" => assert_eq!(output["payload_disposition"], "not-modified"),
            "gone" => {
                assert_eq!(
                    output["provider_observation"]["history_coverage"],
                    "lost-interval"
                );
                assert_eq!(
                    output["provider_schedule"]["new_snapshot_repairs_history"],
                    false
                );
            }
            _ => unreachable!(),
        }
        assert!(!output.to_string().contains("suppressed-fixture-private"));
    }
    assert_eq!(endpoint.count(), 3);
    f.submit("quota-exhausted", "read");
    f.drain("provider-deferred-turn");
    let blocked = f.task("quota-exhausted");
    assert_eq!(blocked["state"], "blocked");
    assert_eq!(blocked["error"]["code"], "native_provider_deferred");
    assert_eq!(f.drain("provider-no-replay")["fresh_claims"], 0);
    assert_eq!(
        endpoint.count(),
        3,
        "retained or deferred permission performed HTTP"
    );
    assert_eq!(
        f.native
            .repo()
            .host_query("scope", &json!({"scope":"deployment"}))
            .unwrap()["calls"],
        3
    );
    let status = f
        .native
        .repo()
        .host_query("circuit.status", &json!({"policy":f.policy}))
        .unwrap();
    assert_eq!(status["quota"]["used_requests"], 3);
}

#[test]
#[ignore = "requires actual PostgreSQL and a native Python source worker"]
fn actual_provider_throttle_retains_cooldown_before_any_next_get() {
    let endpoint = Endpoint::start();
    let f = ProviderFixture::new(&endpoint, 10);
    f.submit("original-throttle", "throttle");
    f.drain("throttle-turn");
    let first = f.task("original-throttle");
    assert_eq!(first["state"], "completed");
    assert_eq!(
        first["result"]["provider_observation"]["response_class"],
        "rate-limited"
    );
    let observation = &first["result"]["provider_observation"];
    assert!(
        observation["retry_not_before_ms"].as_i64().unwrap()
            >= observation["source_time_ms"].as_i64().unwrap() + 60_000
    );
    f.submit("next-read", "read");
    f.drain("cooldown-turn");
    assert_eq!(f.task("next-read")["state"], "blocked");
    assert_eq!(f.drain("early-poll-turn")["fresh_claims"], 0);
    assert_eq!(
        endpoint.count(),
        1,
        "provider cooldown did not stop the next GET"
    );
    assert_eq!(
        f.native
            .repo()
            .host_query("scope", &json!({"scope":"deployment"}))
            .unwrap()["calls"],
        1
    );
}
