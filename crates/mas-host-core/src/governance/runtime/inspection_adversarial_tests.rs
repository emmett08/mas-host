//! Challenge inspection bindings and recovery through the actual journal owners.
use super::{GovernedSession, PrivacyInspector};
use crate::{digest, lock, read_step, Engine};
use mas_host_privacy::{DetectorPins, Limits, NeuralAssessment, Source};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const PRIVATE: &str = "person-736@example.invalid";

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

struct Fixture {
    path: PathBuf,
    config: Value,
    authority: Value,
    bundle: Value,
}

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mas-inspection-adversarial-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("key"), [19u8; 32]).unwrap();
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
            json!({"execution":{"type":"string","enum":["completed"]}}),
            &["execution"],
        );
        let expires = crate::now_ms().unwrap() + 600_000;
        let rules = [
            ("invoke", "worker:worker", "local-worker"),
            ("deliver", "principal-a", "client"),
            ("effect", "operation:fixture/1", "external-fixture"),
            ("inspect", "privacy-inspector", "local-detector"),
        ].map(|(operation,recipient,destination)| json!({"operation":operation,"principal":"principal-a","actor":"worker","producer":"host","recipient":recipient,"purpose":"diagnosis","destination":destination,"location":"uk","classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":65536}));
        let authority = json!({"version":"mas-governed-authority/1","grants":[{
            "version":"mas-current-authority/1","status":"active","tenant":"tenant-a","principal":"principal-a","grant_ref":"grant-a","grant_epoch":1,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires}],
            "policies":[{"version":"mas-data-use-policy/1","id":"policy-a","tenant":"tenant-a","policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires,"rules":rules}]});
        fs::write(path.join("authority.json"), authority.to_string()).unwrap();
        let config = json!({"version":"mas-governed-local/1","authority_file":path.join("authority.json"),"commitment_key_file":path.join("key"),"commitment_key_id":"key-v1","location":"uk","agents":[{"actor":"worker","destination":"local-worker","input_schema":schema,"output_schema":schema,"effects":[{"format":"fixture/1","destination":"external-fixture","descriptor_schema":descriptor,"output_schema":schema,"evidence_schema":evidence}]}]});
        let executable = fs::canonicalize("/usr/bin/true").unwrap();
        let bundle = json!({"version":"mas-bundle/3","agents":[{"name":"worker","executable":executable,"sha256":digest(&fs::read(&executable).unwrap()),"handler":"Assess","operation":"Assess","environment":[],"working_directory":path}],"topology":{"agents":["client","worker"],"edges":[["client","worker"],["worker","client"]],"application":"agent-task/1","max_calls":5,"wall_seconds":600}});
        Self {
            path,
            config,
            authority,
            bundle,
        }
    }

    fn session(&self, inspector: Arc<Inspector>) -> GovernedSession {
        GovernedSession::from_value(
            &self.config,
            "tenant-a",
            "principal-a",
            "diagnosis",
            "grant-a",
        )
        .unwrap()
        .with_privacy_inspector(inspector, "local-detector")
        .unwrap()
    }

    fn engine(&self, session: GovernedSession) -> Engine {
        Engine::open_governed(&self.bundle, &self.path, "inspection-run", session).unwrap()
    }

    fn invocation(&self, engine: &Engine) -> crate::Step {
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
            .unwrap()
    }

    fn write_authority(&self, value: &Value) {
        fs::write(self.path.join("authority.json"), value.to_string()).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Inspector {
    pins: DetectorPins,
    mode: AtomicUsize,
    calls: AtomicUsize,
    binding: String,
    timeout_ms: u64,
    revoke: Option<(PathBuf, Value, usize)>,
}

impl Inspector {
    fn new() -> Self {
        Self {
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            mode: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            binding: "d".repeat(64),
            timeout_ms: 10_000,
            revoke: None,
        }
    }

    fn clean(&self, source: &Source, pins: &DetectorPins, limits: Limits) -> NeuralAssessment {
        let request = pins.request(source, limits).unwrap();
        let ranges = if source.content().is_empty() {
            json!([])
        } else {
            json!([{"start":0,"end":source.content().chars().count()}])
        };
        NeuralAssessment::from_value(&json!({"version":"mas-privacy-worker-response/1","source_id":source.id(),"source_token":request["source_token"],"input_bytes":source.content().len(),"detector":pins.value(),"offset_unit":"unicode-codepoints","completion":"complete","checked_ranges":ranges,"omissions":[],"findings":[]}),source,pins,limits).unwrap()
    }
}

impl PrivacyInspector for Inspector {
    fn detector(&self) -> &DetectorPins {
        &self.pins
    }
    fn configuration_binding(&self) -> String {
        self.binding.clone()
    }
    fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
    fn inspect(
        &self,
        source: &Source,
        limits: Limits,
        _deadline_ms: u64,
    ) -> Result<NeuralAssessment, String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((path, authority, at)) = &self.revoke {
            if call == *at {
                fs::write(path, authority.to_string()).unwrap();
            }
        }
        match self.mode.load(Ordering::SeqCst) {
            1 => {
                // Even identical bytes and ID receive a different invocation token.
                let stale =
                    Source::text(source.id(), source.content(), limits.max_input_bytes).unwrap();
                Ok(self.clean(&stale, &self.pins, limits))
            }
            2 => {
                let substituted =
                    DetectorPins::new(&"e".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap();
                Ok(self.clean(source, &substituted, limits))
            }
            3 => Err(PRIVATE.into()),
            4 => NeuralAssessment::failed(source, &self.pins, "worker-timeout", limits)
                .map_err(|_| "invalid-fixture".into()),
            5 => {
                std::thread::sleep(std::time::Duration::from_millis(1_100));
                Ok(self.clean(source, &self.pins, limits))
            }
            _ => Ok(self.clean(source, &self.pins, limits)),
        }
    }
}

#[test]
fn stale_source_tokens_and_substituted_detector_results_cannot_authorise_input() {
    for mode in [1, 2] {
        let fixture = Fixture::new();
        let inspector = Arc::new(Inspector::new());
        inspector.mode.store(mode, Ordering::SeqCst);
        let session = fixture.session(inspector.clone());
        assert_eq!(
            session
                .project_input("worker", &json!({"n":1}))
                .unwrap_err(),
            "governed-inspection-invalid"
        );
        assert_eq!(inspector.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn inspected_private_originals_keep_distinct_commitments_after_equal_projection() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new());
    let session = fixture.session(inspector.clone());
    let first = json!({"n":1,"private":PRIVATE});
    let changed = json!({"n":1,"private":"another-person@example.invalid"});
    assert_eq!(
        session.project_input("worker", &first).unwrap(),
        json!({"n":1})
    );
    assert_eq!(
        session.project_input("worker", &changed).unwrap(),
        json!({"n":1})
    );
    let engine = fixture.engine(session.clone());
    let step = engine
        .inner
        .prepare("", "root", "request", "client", "worker", &first)
        .unwrap();
    assert_eq!(step.input, json!({"n":1}));
    assert!(engine
        .inner
        .prepare("", "root", "request", "client", "worker", &changed)
        .unwrap_err()
        .contains("original-input-conflict"));
    let commitment: String = lock(&engine.inner.journal)
        .unwrap()
        .query_row(
            "SELECT original_commitment FROM governed_inputs WHERE id=?",
            [&step.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(commitment, session.commitment(&first).unwrap());
    assert_ne!(commitment, session.commitment(&changed).unwrap());
    let decision: String = lock(&engine.inner.journal)
        .unwrap()
        .query_row(
            "SELECT decision FROM governed_inputs WHERE id=?",
            [&step.id],
            |row| row.get(0),
        )
        .unwrap();
    let decision: Value = serde_json::from_str(&decision).unwrap();
    assert_eq!(decision["version"], "mas-governed-boundary-decision/1");
    assert!(
        decision["inspection"]["original_findings_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(decision["inspection"]["projected_findings_count"], 0);
    assert_eq!(decision["inspection"]["original_binding"], commitment);
    assert_eq!(
        decision["inspection"]["projection_binding"],
        session.commitment(&step.input).unwrap()
    );
    assert_eq!(
        decision["inspection"]["contract"]["detector"],
        inspector.pins.value()
    );
    assert_eq!(
        decision["inspection"]["contract"]["configuration_binding"],
        inspector.binding
    );
    assert!(!decision.to_string().contains(PRIVATE));
    assert!(inspector.calls.load(Ordering::SeqCst) > 0);
    drop(engine);
    for file in ["execution.sqlite", "protocol.sqlite"] {
        assert!(!fs::read(fixture.path.join("inspection-run").join(file))
            .unwrap()
            .windows(PRIVATE.len())
            .any(|bytes| bytes == PRIVATE.as_bytes()));
    }
}

#[test]
fn inspection_requires_processing_authority_before_the_callback() {
    let mut fixture = Fixture::new();
    fixture.authority["policies"][0]["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|rule| rule["operation"] != "inspect");
    fixture.write_authority(&fixture.authority);
    let inspector = Arc::new(Inspector::new());
    let session = fixture.session(inspector.clone());
    assert_eq!(
        session
            .project_input("worker", &json!({"n":1}))
            .unwrap_err(),
        "governed-policy-denied"
    );
    assert_eq!(inspector.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn incomplete_or_late_detector_results_cannot_release_the_projection() {
    for (mode, expected) in [
        (4, "governed-inspection-incomplete"),
        (5, "governed-inspection-timeout"),
    ] {
        let fixture = Fixture::new();
        let mut inspector = Inspector::new();
        inspector.mode.store(mode, Ordering::SeqCst);
        if mode == 5 {
            inspector.timeout_ms = 1_000;
        }
        let inspector = Arc::new(inspector);
        let session = fixture.session(inspector.clone());
        assert_eq!(
            session
                .project_input("worker", &json!({"n":1}))
                .unwrap_err(),
            expected
        );
        assert_eq!(inspector.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn revocation_during_final_inspection_callback_prevents_input_persistence() {
    let fixture = Fixture::new();
    let mut revoked = fixture.authority.clone();
    revoked["grants"] = json!([]);
    let mut inspector = Inspector::new();
    // The repeated key is one exact text component. Its original/projection
    // occurrences share the same validated assessment within this boundary.
    inspector.revoke = Some((fixture.path.join("authority.json"), revoked, 1));
    let inspector = Arc::new(inspector);
    let engine = fixture.engine(fixture.session(inspector.clone()));
    assert!(engine
        .inner
        .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
        .is_err());
    assert_eq!(inspector.calls.load(Ordering::SeqCst), 1);
    let count: i64 = lock(&engine.inner.journal)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM steps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn policy_byte_scope_includes_inspection_receipt_before_persistence() {
    let mut fixture = Fixture::new();
    fixture.authority["policies"][0]["rules"][0]["max_bytes"] = json!(256);
    fixture.write_authority(&fixture.authority);
    let inspector = Arc::new(Inspector::new());
    let engine = fixture.engine(fixture.session(inspector.clone()));
    let input = json!({"n":1});
    assert!(input.to_string().len() < 256);
    assert_eq!(
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &input)
            .unwrap_err(),
        "governed-policy-denied"
    );
    assert!(inspector.calls.load(Ordering::SeqCst) > 0);
    let count: i64 = lock(&engine.inner.journal)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM steps", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn detector_failure_after_external_completion_retains_known_withheld_fact() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new());
    let engine = fixture.engine(fixture.session(inspector.clone()));
    let invocation = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture/1","n":1});
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    inspector.mode.store(3, Ordering::SeqCst);
    engine
        .inner
        .complete_effect(
            &invocation,
            "effect",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let id = admission["invocation_id"].as_str().unwrap();
    let step = read_step(&lock(&engine.inner.journal).unwrap(), id)
        .unwrap()
        .unwrap();
    assert_eq!(step.state, "completed");
    let completion = engine.inner.governed_completion_ack(id).unwrap().unwrap();
    assert_eq!(completion["external_execution"], "completed");
    assert_eq!(completion["payload_disposition"], "withheld");
    assert!(!completion.to_string().contains(PRIVATE));
    inspector.mode.store(0, Ordering::SeqCst);
    drop(engine);
    let engine = fixture.engine(fixture.session(inspector));
    let invocation = fixture.invocation(&engine);
    let replay = engine
        .inner
        .begin_effect(&invocation, "effect", &descriptor)
        .unwrap();
    assert_eq!(replay["state"], "cached");
    assert_eq!(replay["output"], completion["output"]);
    assert_eq!(engine.status().unwrap()["protocol"]["usage"]["calls"], 1);
}

#[test]
fn same_policy_grant_epoch_refresh_replays_under_stable_inspected_namespace() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new());
    let old = fixture.session(inspector.clone());
    let engine = fixture.engine(old.clone());
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
    drop(engine);
    let mut rotated = fixture.authority.clone();
    rotated["grants"][0]["grant_epoch"] = json!(2);
    fixture.write_authority(&rotated);
    assert!(old.check_current().is_err());
    let fresh = fixture.session(inspector.clone());
    assert_eq!(old.fingerprint(), fresh.fingerprint());
    let engine = fixture.engine(fresh);
    let invocation = fixture.invocation(&engine);
    assert_eq!(
        engine
            .inner
            .begin_effect(&invocation, "effect", &descriptor)
            .unwrap()["state"],
        "cached"
    );
    assert_eq!(
        engine
            .inner
            .governed_completion_ack(admission["invocation_id"].as_str().unwrap())
            .unwrap()
            .unwrap()["external_execution"],
        "completed"
    );
    assert_eq!(engine.status().unwrap()["protocol"]["usage"]["calls"], 1);
    drop(engine);
    rotated["policies"][0]["rules"][0]["max_bytes"] = json!(32_768);
    fixture.write_authority(&rotated);
    assert!(Engine::open_governed(
        &fixture.bundle,
        &fixture.path,
        "inspection-run",
        fixture.session(inspector.clone())
    )
    .is_err());
    fixture.write_authority(&fixture.authority);
    fs::write(fixture.path.join("key"), [20u8; 32]).unwrap();
    assert!(Engine::open_governed(
        &fixture.bundle,
        &fixture.path,
        "inspection-run",
        fixture.session(inspector)
    )
    .is_err());
}

#[test]
fn inspector_configuration_and_destination_are_part_of_retained_namespace_identity() {
    let fixture = Fixture::new();
    let first = fixture.session(Arc::new(Inspector::new()));
    let engine = fixture.engine(first.clone());
    drop(engine);
    let mut changed = Inspector::new();
    changed.binding = "f".repeat(64);
    let second = fixture.session(Arc::new(changed));
    assert_ne!(first.fingerprint(), second.fingerprint());
    assert!(
        Engine::open_governed(&fixture.bundle, &fixture.path, "inspection-run", second).is_err()
    );
    let alternate = GovernedSession::from_value(
        &fixture.config,
        "tenant-a",
        "principal-a",
        "diagnosis",
        "grant-a",
    )
    .unwrap()
    .with_privacy_inspector(Arc::new(Inspector::new()), "other-detector")
    .unwrap();
    assert_ne!(first.fingerprint(), alternate.fingerprint());
    assert!(
        Engine::open_governed(&fixture.bundle, &fixture.path, "inspection-run", alternate).is_err()
    );
}
