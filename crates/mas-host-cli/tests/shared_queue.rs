#![cfg(feature = "shared-native")]
//! Real queue authority traversal. The callback checks scheduling only; native
//! worker execution has separate shared-native integration tests.
#[path = "../src/shared_queue.rs"]
mod shared_queue;
use mas_protocol_codec::postgres_repository::{AuthorityBinding, PostgresRepository};
use postgres::{Client, NoTls};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn apply(repo: &mut PostgresRepository, id: &str, op: &str, p: Value) -> Value {
    repo.host_command(id, op, &p).unwrap()["receipt"].clone()
}
struct Fixture {
    dsn: String,
    schema: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(&self.dsn, NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}

#[test]
#[ignore = "requires actual PostgreSQL; exercises 1002 durable task submissions"]
fn postgres_paged_claim_reaches_task_after_one_thousand_resource_blocked_candidates() {
    let dsn = std::env::var("MAS_TEST_POSTGRES_DSN").expect("actual PostgreSQL required");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let f = Fixture {
        dsn,
        schema: format!("host_queue_{}_{}", std::process::id(), stamp),
    };
    let binding = AuthorityBinding {
        deployment: "queue-test".into(),
        controller: "controller".into(),
        epoch: 1,
    };
    let mut repo =
        PostgresRepository::initialise(Client::connect(&f.dsn, NoTls).unwrap(), &f.schema, binding)
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
    apply(
        &mut repo,
        "configure",
        "task.configure",
        json!({"max_tasks":3000,"max_pending":3000,"max_revisions":2}),
    );
    apply(
        &mut repo,
        "principal",
        "task.configure_principal",
        json!({"principal":"alice","max_tasks":3000,"max_pending":3000}),
    );
    apply(
        &mut repo,
        "scheduler",
        "task.configure_scheduler",
        json!({"policy":{"max_workers":2,"max_per_principal":2,"workflows":{},"resources":{"X":1,"Y":1}}}),
    );
    let source = b"bounded queue scheduling fixture; never executed as a native worker\n";
    let package = json!({"format":"mas-native-implementation/1","bundle_digest":"a".repeat(64),
        "files":[{"path":"fixture.txt","sha256":digest(source),"bytes":source.to_vec()}]});
    let package_text = package.to_string();
    let implementation = digest(package_text.as_bytes());
    apply(
        &mut repo,
        "chunk",
        "definition.chunk",
        json!({"digest":implementation,"part":0,"data":package_text}),
    );
    apply(
        &mut repo,
        "manifest",
        "definition.manifest",
        json!({"digest":implementation,"parts":1}),
    );
    let register = |repo: &mut PostgresRepository, resource: &str| {
        let descriptor = json!({"implementation_digest":implementation,"bundle_digest":"a".repeat(64),"resource_keys":[resource],
            "timing_policy":{"format":"mas-timing-policy/1","business_seconds":3600,"active_seconds":60,"queue_seconds":1800,"pause_seconds":1800}});
        let identity = digest(descriptor.to_string().as_bytes());
        apply(
            repo,
            &format!("register-{resource}"),
            "definition.register",
            json!({"kind":"workflow","name":"native","identity":identity,
            "descriptor":descriptor,"implementation_digest":implementation,"current":true}),
        );
        identity
    };
    let blocked = register(&mut repo, "X");
    let submit = |repo: &mut PostgresRepository, index: usize, identity: &str| {
        let id = format!("task-{index}");
        apply(
            repo,
            &format!("submit-{index}"),
            "task.submit",
            json!({"principal":"alice","namespace":"mas-unified/1","epoch":"queue-one",
            "key":id,"digest":digest(id.as_bytes()),"task":id,"workflow":"native","identity":identity,"input":{},"metadata":{}}),
        );
    };
    submit(&mut repo, 0, &blocked);
    apply(
        &mut repo,
        "hold-X",
        "task.claim",
        json!({"owner":"holder","task":"task-0"}),
    );
    for index in 1..=1000 {
        submit(&mut repo, index, &blocked);
    }
    let ready = register(&mut repo, "Y");
    submit(&mut repo, 1001, &ready);
    assert_eq!(
        apply(
            &mut repo,
            "unqualified-claim",
            "task.claim",
            json!({"owner":"scheduler"})
        ),
        Value::Null
    );
    let turn = shared_queue::Turn {
        owner: "scheduler",
        turn_id: "turn-one",
        after_seq: 0,
        page_size: 1000,
        max_pages: 2,
        max_claims: 1,
        workflow: "native",
        workflow_identity: &ready,
    };
    let mut selected = Vec::new();
    let result = shared_queue::drain(&mut repo, &turn, |_, task| {
        selected.push(task["task_id"].as_str().unwrap().to_owned());
        assert!(shared_queue::task_deadline_ms(task).is_ok());
        Ok(shared_queue::DispatchOutcome::Completed(
            json!({"scheduler_fixture":true}),
        ))
    })
    .unwrap();
    assert_eq!(selected, vec!["task-1001"]);
    assert_eq!(result["candidates_examined"], 1001);
    assert_eq!(result["fresh_claims"], 1);
    assert_eq!(
        repo.host_query(
            "task.inspect",
            &json!({"principal":"alice","task":"task-1001"})
        )
        .unwrap()["state"],
        "completed"
    );
    assert_eq!(
        repo.host_query("task.scheduler", &json!({})).unwrap()["occupied"],
        1
    );
}
