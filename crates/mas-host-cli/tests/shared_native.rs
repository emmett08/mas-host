#![cfg(feature = "shared-native")]
//! Real PostgreSQL and native-worker conformance, deliberately opt-in.
use mas_protocol_codec::postgres_repository::{AuthorityBinding, PostgresRepository};
use postgres::{Client, NoTls};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    dsn: String,
    schema: String,
    root: PathBuf,
    binding: AuthorityBinding,
    bundle: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dsn = std::env::var("MAS_TEST_POSTGRES_DSN")
            .expect("real PostgreSQL test connection required");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("host_native_{}_{}", std::process::id(), stamp);
        let root = std::env::temp_dir().join(&schema);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("native.rs"),
            r#"use mas_host_sdk::{Context,Json,Result,json};
use std::{fs::OpenOptions,io::Write};
pub fn apply(_ctx:&mut Context,input:Json)->Result<Json>{
 let path=input["ledger"].as_str().ok_or("ledger")?;
 let mut f=OpenOptions::new().append(true).create(true).open(path).map_err(|e|e.to_string())?;
 writeln!(f,"{}",input["id"].as_str().ok_or("id")?).map_err(|e|e.to_string())?;
 f.sync_all().map_err(|e|e.to_string())?;
 if input["lose_reply"]==true {std::process::exit(73);}
 Ok(json!({"accepted":true}))
}"#,
        )
        .unwrap();
        write(
            &root.join("input.json"),
            &json!({"type":"object","properties":{"ledger":{"type":"string"},"id":{"type":"string"},"lose_reply":{"type":"boolean"}},"required":["ledger","id","lose_reply"],"additionalProperties":false}),
        );
        write(
            &root.join("output.json"),
            &json!({"type":"object","properties":{"accepted":{"type":"boolean"}},"required":["accepted"],"additionalProperties":false}),
        );
        write(
            &root.join("agents.json"),
            &json!({"version":"mas-agents/3","agents":[{"name":"native","operation":"native/1","input_schema":"input.json","output_schema":"output.json","implementation":{"kind":"native","language":"rust","source":"native.rs","handler":"apply"}}]}),
        );
        write(
            &root.join("topology.json"),
            &json!({"version":"mas-topology/1","edges":[["client","native"],["native","client"]],"max_calls":1,"wall_seconds":120}),
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
        let built: Value = serde_json::from_slice(&output.stdout).unwrap();
        let bundle = PathBuf::from(built["bundle"].as_str().unwrap());
        let binding = AuthorityBinding {
            deployment: "host-native-test".into(),
            controller: "controller".into(),
            epoch: 1,
        };
        let mut repo = PostgresRepository::initialise(
            Client::connect(&dsn, NoTls).unwrap(),
            &schema,
            binding.clone(),
        )
        .unwrap();
        repo.install_host_repository().unwrap();
        repo.install_controller_repository().unwrap();
        repo.install_service_repository().unwrap();
        repo.install_accounting_repository().unwrap();
        repo.install_recovery_repository().unwrap();
        repo.install_jobs_repository().unwrap();
        repo.install_command_repository(None).unwrap();
        repo.install_retention_repository().unwrap();
        repo.renew_authority(3600).unwrap();
        for scope in ["deployment", "principal/tester"] {
            repo.host_command(
                &format!("scope-{}", scope.replace('/', "-")),
                "configure_scope",
                &json!({"scope":scope,"max_runs":10,"max_calls":10,"max_microusd":10}),
            )
            .unwrap();
        }
        write(
            &root.join("authority.json"),
            &json!({"version":"mas-host-shared-native-authority/1","schema":schema,"deployment":binding.deployment,"controller":binding.controller,"epoch":1,"principal":"tester","owner":"native-owner"}),
        );
        Self {
            dsn,
            schema,
            root,
            binding,
            bundle,
        }
    }
    fn run(&self, id: &str, lose: bool) -> Value {
        let input = self.root.join(format!("{id}.json"));
        write(
            &input,
            &json!({"id":id,"ledger":self.root.join("ledger"),"lose_reply":lose}),
        );
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .env("MAS_POSTGRES_DSN", &self.dsn)
            .args(["shared-native", "--authority"])
            .arg(self.root.join("authority.json"))
            .arg("--bundle")
            .arg(&self.bundle)
            .arg("--input")
            .arg(&input)
            .arg("--state")
            .arg(self.root.join("lifetime"))
            .args(["--run", id, "--agent", "native"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn queue(&self, id: &str) -> String {
        let mut repo = self.repo();
        let bundle: Value = serde_json::from_slice(&fs::read(&self.bundle).unwrap()).unwrap();
        let bundle_digest = mas_host_core::describe(&bundle).unwrap()["bundle_digest"].clone();
        let source = fs::read(self.root.join("native.rs")).unwrap();
        let package = json!({"format":"mas-native-implementation/1","bundle_digest":bundle_digest,"files":[{"path":"native.rs","sha256":digest(&source),"bytes":source}]});
        let package_text = package.to_string();
        let implementation = digest(package_text.as_bytes());
        repo.host_command(
            "native-chunk",
            "definition.chunk",
            &json!({"digest":implementation,"part":0,"data":package_text}),
        )
        .unwrap();
        repo.host_command(
            "native-package",
            "definition.manifest",
            &json!({"digest":implementation,"parts":1}),
        )
        .unwrap();
        let definition = json!({"implementation_digest":implementation,"bundle_digest":bundle_digest,"resource_keys":[],"timing_policy":{"format":"mas-timing-policy/1","business_seconds":120,"active_seconds":30,"queue_seconds":120,"pause_seconds":120}});
        let identity = digest(definition.to_string().as_bytes());
        repo.host_command("native-definition","definition.register",&json!({"kind":"workflow","name":"native","identity":identity,"descriptor":definition,"implementation_digest":implementation,"current":true})).unwrap();
        repo.host_command(
            "task-policy",
            "task.configure",
            &json!({"max_tasks":10,"max_pending":10,"max_revisions":1}),
        )
        .unwrap();
        repo.host_command(
            "task-principal",
            "task.configure_principal",
            &json!({"principal":"tester","max_tasks":10,"max_pending":10}),
        )
        .unwrap();
        repo.host_command("task-scheduler","task.configure_scheduler",&json!({"policy":{"max_workers":1,"max_per_principal":1,"workflows":{},"resources":{}}})).unwrap();
        let input = json!({"id":id,"ledger":self.root.join("ledger"),"lose_reply":false});
        repo.host_command("submit-native","task.submit",&json!({"principal":"tester","namespace":"mas-unified/1","epoch":"native-test-epoch","key":id,"digest":digest(input.to_string().as_bytes()),"task":id,"workflow":"native","identity":identity,"expected_identity":identity,"input":input,"metadata":{}})).unwrap();
        identity
    }
    fn drain(&self, identity: &str) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_mas-host"))
            .env("MAS_POSTGRES_DSN", &self.dsn)
            .args(["shared-drain", "--authority"])
            .arg(self.root.join("authority.json"))
            .arg("--bundle")
            .arg(&self.bundle)
            .arg("--state")
            .arg(self.root.join("lifetime"))
            .args([
                "--agent",
                "native",
                "--workflow",
                "native",
                "--workflow-identity",
                identity,
                "--turn",
                "first-turn",
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
    fn repo(&self) -> PostgresRepository {
        PostgresRepository::open(
            Client::connect(&self.dsn, NoTls).unwrap(),
            &self.schema,
            self.binding.clone(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.dsn, NoTls) {
            let _ = c.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn write(p: &Path, v: &Value) {
    fs::write(p, serde_json::to_vec(v).unwrap()).unwrap();
}

#[test]
#[ignore = "requires actual PostgreSQL and native Rust compilation"]
fn postgres_native_completion_and_lost_response_never_redispatch() {
    let fixture = Fixture::new();
    let first = fixture.run("completed", false);
    assert_eq!(first["admission"], "fresh");
    assert_eq!(first["dispatched"], true);
    assert_eq!(first["invocation"]["state"], "completed");
    assert_eq!(first["invocation"]["output"], json!({"accepted":true}));
    let replay = fixture.run("completed", false);
    assert_eq!(replay["admission"], "retained");
    assert_eq!(replay["dispatched"], false);
    assert_eq!(
        fs::read_to_string(fixture.root.join("ledger")).unwrap(),
        "completed\n"
    );
    let lost = fixture.run("lost", true);
    assert_eq!(lost["invocation"]["state"], "unknown");
    assert_eq!(fixture.run("lost", true)["dispatched"], false);
    assert_eq!(
        fs::read_to_string(fixture.root.join("ledger")).unwrap(),
        "completed\nlost\n"
    );
    let scope = fixture
        .repo()
        .host_query("scope", &json!({"scope":"deployment"}))
        .unwrap();
    assert_eq!(scope["calls"], 2, "replay changed original reservations");
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
#[ignore = "requires actual PostgreSQL and native Rust compilation"]
fn postgres_queued_native_cli_preserves_task_identity_and_executes_once() {
    let fixture = Fixture::new();
    let identity = fixture.queue("queued-native");
    let first = fixture.drain(&identity);
    assert_eq!(first["fresh_claims"], 1);
    assert_eq!(first["task_summaries"][0]["state"], "completed");
    let task = fixture
        .repo()
        .host_query(
            "task.inspect",
            &json!({"principal":"tester","task":"queued-native"}),
        )
        .unwrap();
    assert_eq!(task["state"], "completed");
    assert_eq!(task["result"], json!({"accepted":true}));
    assert_eq!(fixture.drain(&identity)["fresh_claims"], 0);
    assert_eq!(
        fs::read_to_string(fixture.root.join("ledger")).unwrap(),
        "queued-native\n"
    );
    assert_eq!(
        fixture
            .repo()
            .host_query("scope", &json!({"scope":"deployment"}))
            .unwrap()["calls"],
        1
    );
}
