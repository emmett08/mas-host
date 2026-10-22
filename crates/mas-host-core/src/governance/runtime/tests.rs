use super::*;
use crate::{digest, lock, read_step, Engine};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
const PRIVATE: &str = "private_person_4387@example.invalid";

struct Fixture {
    path: PathBuf,
    config: Value,
    authority: Value,
    bundle: Value,
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mas-governed-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("key"), [7_u8; 32]).unwrap();
        fs::set_permissions(path.join("key"), fs::Permissions::from_mode(0o600)).unwrap();
        let schema = object(
            json!({"n":{"type":"integer","minimum":0,"maximum":100}}),
            &["n"],
        );
        let descriptor = object(
            json!({"format":{"type":"string","enum":["fixture/1"]},"n":{"type":"integer","minimum":0,"maximum":100}}),
            &["format", "n"],
        );
        let evidence = object(
            json!({"execution":{"type":"string","enum":["completed","unknown","not_started","rejected"]}}),
            &["execution"],
        );
        let mut agents = Vec::new();
        let mut rules = Vec::new();
        for actor in ["worker", "other"] {
            agents.push(json!({"actor":actor,"destination":format!("local-{actor}"),"input_schema":schema,"output_schema":schema,
                "effects":[{"format":"fixture/1","destination":"external-fixture","descriptor_schema":descriptor,"output_schema":schema,"evidence_schema":evidence}]}));
            for (operation, recipient, destination) in [
                (
                    "invoke",
                    format!("worker:{actor}"),
                    format!("local-{actor}"),
                ),
                ("deliver", "principal-a".into(), "client".into()),
                (
                    "effect",
                    "operation:fixture/1".into(),
                    "external-fixture".into(),
                ),
            ] {
                rules.push(json!({"operation":operation,"principal":"principal-a","actor":actor,"producer":"host","recipient":recipient,
                    "purpose":"diagnosis","destination":destination,"location":"uk","classifications":["restricted"],
                    "obligations":["inline-retention-approved"],"max_bytes":65536}));
            }
        }
        let expires = crate::now_ms().unwrap() + 60_000;
        let authority = json!({"version":"mas-governed-authority/1","grants":[{
            "version":"mas-current-authority/1","status":"active","tenant":"tenant-a","principal":"principal-a","grant_ref":"grant-a",
            "grant_epoch":1,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires}],
            "policies":[{"version":"mas-data-use-policy/1","id":"policy-a","tenant":"tenant-a","policy_version":"policy-v1",
                "valid_from_ms":0,"expires_at_ms":expires,"rules":rules}]});
        fs::write(path.join("authority.json"), authority.to_string()).unwrap();
        let config = json!({"version":"mas-governed-local/1","authority_file":path.join("authority.json"),"commitment_key_file":path.join("key"),
            "commitment_key_id":"key-v1","location":"uk","agents":agents});
        let executable = fs::canonicalize("/usr/bin/true").unwrap();
        let hash = digest(&fs::read(&executable).unwrap());
        let bundle = json!({"version":"mas-bundle/3","agents":[
            {"name":"worker","executable":executable,"sha256":hash,"handler":"Assess","operation":"Assess","environment":[],"working_directory":path},
            {"name":"other","executable":executable,"sha256":hash,"handler":"Assess","operation":"Assess","environment":[],"working_directory":path}],
            "topology":{"agents":["client","worker","other"],"edges":[["client","worker"],["worker","client"],["worker","other"],["other","worker"]],
                "application":"agent-task/1","max_calls":5,"wall_seconds":60}});
        Self {
            path,
            config,
            authority,
            bundle,
        }
    }
    fn session(&self) -> GovernedSession {
        GovernedSession::from_value(
            &self.config,
            "tenant-a",
            "principal-a",
            "diagnosis",
            "grant-a",
        )
        .unwrap()
    }
    fn engine(&self) -> Engine {
        Engine::open_governed(&self.bundle, &self.path, "governed-run", self.session()).unwrap()
    }
    fn update(&self, value: &Value) {
        fs::write(self.path.join("authority.json"), value.to_string()).unwrap();
    }
    fn invocation(&self, engine: &Engine) -> crate::Step {
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn structured_projection_hides_originals_but_preserves_conflicts() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let a = json!({"n":1,"private":PRIVATE});
    let b = json!({"n":1,"private":"different-private-person"});
    assert_eq!(session.project_input("worker", &a).unwrap(), json!({"n":1}));
    assert_ne!(
        session.commitment(&a).unwrap(),
        session.commitment(&b).unwrap()
    );
    let reordered =
        mas_protocol_codec::parse_json(&format!("{{\"private\":\"{PRIVATE}\",\"n\":1}}")).unwrap();
    assert_eq!(
        session.commitment(&a).unwrap(),
        session.commitment(&reordered).unwrap()
    );
    let engine = fixture.engine();
    let step = engine
        .inner
        .prepare("", PRIVATE, "request", "client", "worker", &a)
        .unwrap();
    assert_eq!(step.input, json!({"n":1}));
    assert!(engine
        .inner
        .prepare("", PRIVATE, "request", "client", "worker", &b)
        .unwrap_err()
        .contains("original-input-conflict"));
    let snapshot = engine.status().unwrap().to_string();
    assert!(!snapshot.contains(PRIVATE));
    drop(engine);
    assert!(
        !fs::read(fixture.path.join("governed-run/execution.sqlite"))
            .unwrap()
            .windows(PRIVATE.len())
            .any(|v| v == PRIVATE.as_bytes())
    );
}

#[test]
fn governed_namespace_has_no_ordinary_or_revision_bypass() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    assert_eq!(
        engine.status().unwrap_err(),
        "governed-status-requires-bound-actor"
    );
    assert!(engine
        .revise(&json!({}), &json!({"raw":PRIVATE}))
        .unwrap_err()
        .contains("governed-revision-unsupported"));
    assert!(crate::inspect(&fixture.path, "governed-run").is_err());
    assert!(crate::observe(&fixture.path, "governed-run", true).is_err());
    assert!(crate::resolve(
        &fixture.path,
        "governed-run",
        "unknown",
        &json!({}),
        &json!({"e":1})
    )
    .is_err());
    drop(engine);
    assert!(Engine::open(&fixture.bundle, &fixture.path, "governed-run").is_err());
    let ordinary = Engine::open(&fixture.bundle, &fixture.path, "ordinary-run").unwrap();
    drop(ordinary);
    assert!(Engine::open_governed(
        &fixture.bundle,
        &fixture.path,
        "ordinary-run",
        fixture.session()
    )
    .is_err());
}

#[test]
fn known_completion_with_withheld_payload_survives_restart_without_fresh_permission() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture/1","n":1});
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    assert_eq!(admission["state"], "execute");
    let effect_id = admission["invocation_id"].as_str().unwrap().to_owned();
    let raw = json!({"n":PRIVATE});
    let evidence = json!({"execution":"completed","private":PRIVATE});
    engine
        .inner
        .complete_effect(&invocation, "effect", &raw, &evidence)
        .unwrap();
    let ack = engine
        .inner
        .governed_completion_ack(&effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(ack["external_execution"], "completed");
    assert_eq!(ack["payload_disposition"], "withheld");
    assert_eq!(ack["output"]["version"], "mas-governed-unavailable/1");
    assert_eq!(
        engine.inner.governed_output(&invocation, &ack["output"]),
        ack["output"]
    );
    let mut other = invocation.clone();
    other.target = "other".into();
    assert_eq!(
        engine.inner.governed_output(&other, &ack["output"])["external_execution"],
        "unknown"
    );
    other.target = "worker".into();
    other.id.push_str("-unrelated");
    assert_eq!(
        engine.inner.governed_output(&other, &ack["output"])["external_execution"],
        "unknown"
    );
    engine
        .inner
        .complete_effect(&invocation, "effect", &raw, &evidence)
        .unwrap();
    assert!(engine
        .inner
        .complete_effect(&invocation, "effect", &json!({"n":99}), &evidence)
        .is_err());
    assert!(!engine.status().unwrap().to_string().contains(PRIVATE));
    drop(engine);
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let replay = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    assert_eq!(replay["state"], "cached");
    assert_eq!(replay["output"], ack["output"]);
    let status = engine.status().unwrap();
    assert_eq!(status["protocol"]["usage"]["calls"], 1);
    let step = read_step(&lock(&engine.inner.journal).unwrap(), &effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(step.state, "completed");
    assert_eq!(step.evidence.unwrap()["external_execution"], "completed");
}

#[test]
fn accepted_output_and_evidence_are_projected_once_for_ack_replay_and_both_stores() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture/1","n":1});
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    engine
        .inner
        .complete_effect(
            &invocation,
            "effect",
            &json!({"n":2,"private":PRIVATE}),
            &json!({"execution":"completed","private":PRIVATE}),
        )
        .unwrap();
    let ack = engine
        .inner
        .governed_completion_ack(admission["invocation_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(ack["payload_disposition"], "available");
    assert_eq!(ack["output"], json!({"n":2}));
    assert_eq!(
        engine
            .inner
            .begin_effect(&invocation, "effect", &descriptor)
            .unwrap()["output"],
        ack["output"]
    );
    assert!(!engine.status().unwrap().to_string().contains(PRIVATE));
    assert!(engine
        .inner
        .begin_effect(
            &invocation,
            "other-effect",
            &json!({"format":"fixture/1","n":1,"private":PRIVATE})
        )
        .is_err());
}

#[test]
fn revocation_blocks_new_work_and_delivery_but_preserves_observed_completion() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture/1","n":1});
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    let mut revoked = fixture.authority.clone();
    revoked["grants"] = json!([]);
    fixture.update(&revoked);
    engine
        .inner
        .complete_effect(
            &invocation,
            "effect",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    assert!(engine.status().is_err());
    assert!(engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .is_err());
    let step = read_step(
        &lock(&engine.inner.journal).unwrap(),
        admission["invocation_id"].as_str().unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(step.state, "completed");
    let safe = step.output.unwrap();
    assert_eq!(safe["payload_disposition"], "withheld");
    assert_eq!(safe["external_execution"], "completed");
    assert_eq!(engine.inner.governed_output(&invocation, &safe), safe);
    drop(engine);
    let mut restored = fixture.authority.clone();
    restored["grants"][0]["grant_epoch"] = json!(2);
    fixture.update(&restored);
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let replay = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    assert_eq!(replay["state"], "cached");
    assert_eq!(replay["output"], safe);
    assert_eq!(engine.status().unwrap()["protocol"]["usage"]["calls"], 1);
}

#[test]
fn completion_policy_limit_covers_evidence_and_receipt_not_only_output() {
    let mut fixture = Fixture::new();
    for rule in fixture.authority["policies"][0]["rules"]
        .as_array_mut()
        .unwrap()
    {
        rule["max_bytes"] = json!(256);
    }
    fixture.update(&fixture.authority);
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture/1","n":1});
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    engine
        .inner
        .complete_effect(
            &invocation,
            "effect",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let completion = engine
        .inner
        .governed_completion_ack(admission["invocation_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(completion["payload_disposition"], "withheld");
    assert_eq!(completion["host_settlement"], "completed");
    assert_eq!(completion["external_execution"], "completed");
    assert!(super::boundary::valid_completion_ack(&completion));
}

#[test]
fn application_schema_exception_requires_exact_host_unavailable_marker() {
    let mut fixture = Fixture::new();
    fixture.bundle["agents"][0]["output_schema"] = object(json!({"n":{"type":"integer"}}), &["n"]);
    fixture.config["agents"][0]["output_schema"]["properties"]["version"] =
        json!({"type":"string","enum":["mas-governed-unavailable/1"]});
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let projection = engine.inner.governed_output(
        &invocation,
        &json!({"n":2,"version":"mas-governed-unavailable/1"}),
    );
    assert_eq!(
        projection,
        json!({"n":2,"version":"mas-governed-unavailable/1"})
    );
    assert!(engine
        .inner
        .validate_worker_output("worker", &projection)
        .is_err());
    assert!(engine
        .inner
        .validate_worker_output("worker", &super::session::unavailable("unknown"))
        .is_ok());
    assert!(engine
        .inner
        .validate_worker_output("worker", &json!({"n":2}))
        .is_ok());
}

#[test]
fn credential_rotation_preserves_identity_while_policy_or_key_change_requires_migration() {
    let fixture = Fixture::new();
    let first = fixture.session();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    drop((invocation, engine));
    let mut rotated = fixture.authority.clone();
    rotated["grants"][0]["grant_epoch"] = json!(2);
    fixture.update(&rotated);
    assert!(first.check_current().is_err());
    let second = fixture.session();
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.identity_commitment(&json!("message-a")).unwrap(),
        second.identity_commitment(&json!("message-a")).unwrap()
    );
    drop(
        Engine::open_governed(
            &fixture.bundle,
            &fixture.path,
            "governed-run",
            second.clone(),
        )
        .unwrap(),
    );
    rotated["policies"][0]["rules"][0]["max_bytes"] = json!(1024);
    fixture.update(&rotated);
    assert!(second.check_current().is_err());
    assert!(Engine::open_governed(
        &fixture.bundle,
        &fixture.path,
        "governed-run",
        fixture.session()
    )
    .is_err());
    fixture.update(&fixture.authority);
    fs::write(fixture.path.join("key"), [8_u8; 32]).unwrap();
    assert!(first.check_current().is_err());
    assert!(Engine::open_governed(
        &fixture.bundle,
        &fixture.path,
        "governed-run",
        fixture.session()
    )
    .is_err());
}

#[test]
fn trusted_credential_callback_is_checked_at_every_boundary_and_free_text_is_rejected() {
    let fixture = Fixture::new();
    let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let current = allowed.clone();
    let session = fixture
        .session()
        .with_authentication_check(std::sync::Arc::new(move || {
            if current.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(PRIVATE.into())
            }
        }));
    session.project_input("worker", &json!({"n":1})).unwrap();
    allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        session
            .project_input("worker", &json!({"n":1}))
            .unwrap_err(),
        "governed-authentication-revoked"
    );
    assert!(session.check_delivery("worker", &json!({"n":1})).is_err());
    let mut config = fixture.config.clone();
    config["agents"][0]["input_schema"] =
        object(json!({"text":{"type":"string","maxLength":10}}), &["text"]);
    assert!(GovernedSession::from_value(
        &config,
        "tenant-a",
        "principal-a",
        "diagnosis",
        "grant-a"
    )
    .is_err());
}

#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
