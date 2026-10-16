//! Independent acceptance counterexamples across the CLI, journal and observer.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn host(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(args)
        .output()
        .unwrap()
}
fn successful(args: &[&str]) -> Value {
    let result = host(args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

struct Fixture {
    root: PathBuf,
    bundle: String,
}
impl Fixture {
    fn new(seconds: u64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-revision-adversarial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("app.py"), r#"from mas_host import Context
from pathlib import Path
import time

def draft(ctx: Context, value: dict) -> dict:
    source = value['sources']['incident']
    if source.get('marker'):
        Path(source['marker']).write_text('started')
    time.sleep(source.get('delay', 0))
    return {'safe': True}

def assess(ctx: Context, value: dict) -> dict:
    marker = value['sources']['incident'].get('assess_marker')
    if marker and not Path(marker).exists():
        Path(marker).write_text('started')
        time.sleep(15)
    explanation = value['sources'].get('private', {}).get('text', 'Public check passed.')
    return {'status':'completed','findings':[{'criterion_id':'safe','verdict':'pass','score':1,'evidence':[value['evidence_refs'][-1]],'explanation':explanation}]}
"#).unwrap();
        let agents = ["draft", "public_check", "private_check"]
            .iter()
            .map(|name| json!({"name":name,"implementation":{"kind":"native","language":"python","source":"app.py","handler":if *name=="draft" {"draft"} else {"assess"}}}))
            .collect::<Vec<_>>();
        write(
            &root.join("agents.json"),
            &json!({"version":"mas-agents/3","agents":agents,"limits":{"wall_seconds":seconds,"max_calls":4,"max_messages":32,"max_bytes":65536,"mailbox_capacity":8}}),
        );
        let edges = ["draft", "public_check", "private_check"]
            .iter()
            .flat_map(|actor| [json!(["client", actor]), json!([actor, "client"])])
            .collect::<Vec<_>>();
        write(
            &root.join("topology.json"),
            &json!({"version":"mas-topology/1","edges":edges}),
        );
        let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let built = successful(&[
            "build",
            "--project",
            root.to_str().unwrap(),
            "--sdk-root",
            sdk.to_str().unwrap(),
        ]);
        let fixture = Self {
            root,
            bundle: built["bundle"].as_str().unwrap().to_owned(),
        };
        write(
            &fixture.root.join("input.json"),
            &json!({"sources":[
                {"id":"incident","kind":"evidence","content":{"signal":"ready"},"readers":["*"]},
                {"id":"private","kind":"peer_history","content":{"text":"PRIVATE-PEER-FINDING"},"readers":["private_check"]}
            ]}),
        );
        write(
            &fixture.root.join("revision.json"),
            &json!({"version":"mas-host-revision/1","draft_agent":"draft","revision_agent":"draft","max_revisions":0,"max_invocations":3,"draft_inputs":["incident"],"revision_inputs":["incident"],"allowed_agents":[],"feedback_evaluators":[],"rubric":{"schema_version":1,"id":"safe-rubric","criteria":[{"id":"safe","measurement":"deterministic","minimum":0,"maximum":1}]},"evaluators":[
                {"actor":"public_check","identity":{"id":"public-evaluator","version":"1","kind":"deterministic"},"inputs":["incident"]},
                {"actor":"private_check","identity":{"id":"private-evaluator","version":"1","kind":"deterministic"},"inputs":["incident","private"]}
            ]}),
        );
        fixture
    }
    fn args(&self, id: &str) -> Vec<String> {
        vec![
            "run".into(),
            "--bundle".into(),
            self.bundle.clone(),
            "--agent".into(),
            "draft".into(),
            "--input".into(),
            self.root.join("input.json").display().to_string(),
            "--state".into(),
            self.root.join("state").display().to_string(),
            "--run".into(),
            id.into(),
            "--revision-plan".into(),
            self.root.join("revision.json").display().to_string(),
        ]
    }
    fn run(&self, id: &str) -> Output {
        host(&self.args(id).iter().map(String::as_str).collect::<Vec<_>>())
    }
    fn observe(&self, id: &str) -> Value {
        successful(&[
            "observe",
            "--state",
            self.root.join("state").to_str().unwrap(),
            "--run",
            id,
            "--include-payloads",
            "true",
        ])
    }
    fn slow_draft(&self) -> PathBuf {
        let marker = self.root.join("worker-started");
        let path = self.root.join("input.json");
        let mut input: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        input["sources"][0]["content"] = json!({"delay":15,"marker":marker});
        write(&path, &input);
        marker
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn shared_evidence_keeps_its_own_readers_across_conflicting_contexts() {
    let fixture = Fixture::new(30);
    let response = fixture.run("shared-source");
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    let value: Value = serde_json::from_slice(&response.stdout).unwrap();
    assert_eq!(value["status"], "completed");
    assert_eq!(
        value["revision"]["assessments"].as_array().unwrap().len(),
        1
    );
    assert!(!value.to_string().contains("PRIVATE-PEER-FINDING"));
    assert!(!fixture
        .observe("shared-source")
        .to_string()
        .contains("PRIVATE-PEER-FINDING"));
    let db = rusqlite::Connection::open(fixture.root.join("state/shared-source/execution.sqlite"))
        .unwrap();
    let readers: String = db
        .query_row(
            "SELECT readers FROM revision_records WHERE kind='evidence' AND id='incident'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&readers).unwrap(),
        json!(["*"])
    );
    let assessment_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM revision_records WHERE kind='assessment'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        assessment_count, 2,
        "conflicting disclosure contexts must retain both assessments"
    );
    drop(db);
    let input_path = fixture.root.join("input.json");
    let mut input: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    input["sources"][1]["readers"] = json!(["*"]);
    write(&input_path, &input);
    let rebound = fixture.run("shared-source");
    assert!(
        !rebound.status.success(),
        "replay must reject changed disclosure policy"
    );
    assert!(String::from_utf8_lossy(&rebound.stderr).contains("differs from retained run"));
}

#[test]
fn cancellation_is_terminal_and_replay_does_not_restart_the_worker() {
    let fixture = Fixture::new(30);
    let marker = fixture.slow_draft();
    let child = Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(fixture.args("cancelled"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !marker.exists() && started.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        marker.exists(),
        "worker did not reach the cancellation boundary"
    );
    successful(&[
        "cancel",
        "--state",
        fixture.root.join("state").to_str().unwrap(),
        "--run",
        "cancelled",
    ]);
    assert!(!child.wait_with_output().unwrap().status.success());
    assert_eq!(
        fixture.observe("cancelled")["revision"]["status"],
        "cancelled"
    );
    fs::remove_file(&marker).unwrap();
    assert!(!fixture.run("cancelled").status.success());
    assert!(
        !marker.exists(),
        "cancelled replay dispatched the worker again"
    );
}

#[test]
fn deadline_is_terminal_and_replay_does_not_extend_it() {
    let fixture = Fixture::new(1);
    let marker = fixture.slow_draft();
    let response = fixture.run("expired");
    assert!(!response.status.success());
    assert!(marker.exists(), "worker must start before deadline expires");
    let observed = fixture.observe("expired");
    assert_eq!(observed["revision"]["status"], "expired");
    let original_deadline = observed["run"]["deadline"].clone();
    fs::remove_file(&marker).unwrap();
    assert!(!fixture.run("expired").status.success());
    assert!(
        !marker.exists(),
        "expired replay dispatched the worker again"
    );
    assert_eq!(
        fixture.observe("expired")["run"]["deadline"],
        original_deadline
    );
}

#[test]
fn crash_during_assessment_reuses_the_completed_draft() {
    use std::os::fd::AsRawFd;
    let fixture = Fixture::new(30);
    let draft_marker = fixture.root.join("draft-started");
    let assessor_marker = fixture.root.join("assessor-started");
    let input_path = fixture.root.join("input.json");
    let mut input: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    input["sources"][0]["content"] = json!({"marker":draft_marker,"assess_marker":assessor_marker});
    write(&input_path, &input);
    let mut child = Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .args(fixture.args("restarted"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !assessor_marker.exists() && started.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        assessor_marker.exists(),
        "assessor did not reach crash boundary"
    );
    assert!(draft_marker.exists());
    let before = fixture.observe("restarted");
    assert!(before["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["key"] == "draft" && step["state"] == "completed"));
    child.kill().unwrap();
    child.wait().unwrap();
    // The guardian retains the same owner lock until worker cleanup completes.
    let owner = fs::File::open(fixture.root.join("state/restarted/owner.lock")).unwrap();
    let cleanup = Instant::now();
    loop {
        if unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            assert_eq!(unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_UN) }, 0);
            break;
        }
        assert!(
            cleanup.elapsed() < Duration::from_secs(10),
            "guardian did not release ownership"
        );
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_file(&draft_marker).unwrap();
    let resumed = fixture.run("restarted");
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(resumed["status"], "completed");
    assert!(
        !draft_marker.exists(),
        "completed draft was executed again after restart"
    );
    assert_eq!(resumed["revision"]["budget"]["used_invocations"], 3);
    assert_eq!(
        fixture.observe("restarted")["run"]["deadline"],
        before["run"]["deadline"]
    );
}
