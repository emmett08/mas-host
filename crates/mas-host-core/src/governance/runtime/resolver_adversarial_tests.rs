//! Challenge retained-effect retrieval without acquiring execution authority.
use super::{read_governed_effect, read_governed_root_effect, GovernedSession, PrivacyInspector};
use crate::{digest, lock, read_step, Engine, Step};
use mas_host_privacy::{DetectorPins, Limits, NeuralAssessment, Source};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const PRIVATE: &str = "resolver-private-person@example.invalid";
const UNAVAILABLE: &str = "governed-evidence-unavailable";

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
            "mas-resolver-adversarial-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("key"), [29u8; 32]).unwrap();
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
            json!({"execution":{"type":"string","enum":["completed","unknown"]}}),
            &["execution"],
        );
        let expires = crate::now_ms().unwrap() + 600_000;
        let mut grants = Vec::new();
        let mut policies = Vec::new();
        for (tenant, grant) in [("tenant-a", "grant-a"), ("tenant-b", "grant-b")] {
            let mut rules = Vec::new();
            for actor in ["worker", "other"] {
                for (operation, recipient, destination) in [
                    ("invoke", format!("worker:{actor}"), "local-worker"),
                    ("deliver", "principal-a".into(), "client"),
                    ("effect", "operation:fixture/1".into(), "external-fixture"),
                    ("inspect", "privacy-inspector".into(), "local-detector"),
                    ("resolve", "principal-a".into(), "client"),
                ] {
                    rules.push(json!({"operation":operation,"principal":"principal-a","actor":actor,"producer":"host","recipient":recipient,"purpose":"diagnosis","destination":destination,"location":"uk","classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":65536}));
                }
            }
            grants.push(json!({"version":"mas-current-authority/1","status":"active","tenant":tenant,"principal":"principal-a","grant_ref":grant,"grant_epoch":1,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires}));
            policies.push(json!({"version":"mas-data-use-policy/1","id":format!("policy-{tenant}"),"tenant":tenant,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires,"rules":rules}));
        }
        let authority =
            json!({"version":"mas-governed-authority/1","grants":grants,"policies":policies});
        fs::write(path.join("authority.json"), authority.to_string()).unwrap();
        let agents = ["worker","other"].map(|actor|json!({"actor":actor,"destination":"local-worker","input_schema":schema,"output_schema":schema,"effects":[{"format":"fixture/1","destination":"external-fixture","descriptor_schema":descriptor,"output_schema":schema,"evidence_schema":evidence}]}));
        let config = json!({"version":"mas-governed-local/1","authority_file":path.join("authority.json"),"commitment_key_file":path.join("key"),"commitment_key_id":"key-v1","location":"uk","agents":agents});
        let executable = fs::canonicalize("/usr/bin/true").unwrap();
        let agents = ["worker","other"].map(|actor|json!({"name":actor,"executable":executable,"sha256":digest(&fs::read(&executable).unwrap()),"handler":"Assess","operation":"Assess","environment":[],"working_directory":path}));
        let bundle = json!({"version":"mas-bundle/3","agents":agents,"topology":{"agents":["client","worker","other"],"edges":[["client","worker"],["worker","client"],["worker","other"],["other","worker"]],"application":"agent-task/1","max_calls":5,"wall_seconds":600}});
        Self {
            path,
            config,
            authority,
            bundle,
        }
    }

    fn session(&self) -> GovernedSession {
        self.scoped_session("tenant-a", "grant-a")
    }

    fn scoped_session(&self, tenant: &str, grant: &str) -> GovernedSession {
        GovernedSession::from_value(&self.config, tenant, "principal-a", "diagnosis", grant)
            .unwrap()
    }

    fn inspected(&self, inspector: Arc<Inspector>) -> GovernedSession {
        self.session()
            .with_privacy_inspector(inspector, "local-detector")
            .unwrap()
    }

    fn engine(&self, session: GovernedSession) -> Engine {
        Engine::open_governed(&self.bundle, &self.path, "resolver-run", session).unwrap()
    }

    fn directory(&self) -> PathBuf {
        self.path.join("resolver-run")
    }

    fn invocation(&self, engine: &Engine) -> Step {
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
            .unwrap()
    }

    fn completed(&self, engine: &Engine, invocation: &Step, key: &str, certainty: &str) -> String {
        let admission = engine
            .inner
            .begin_effect(invocation, key, &json!({"format":"fixture/1","n":1}))
            .unwrap();
        engine
            .inner
            .complete_effect(
                invocation,
                key,
                &json!({"n":2}),
                &json!({"execution":certainty}),
            )
            .unwrap();
        admission["invocation_id"].as_str().unwrap().to_owned()
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

fn retained_files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_str().unwrap().to_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

struct Inspector {
    pins: DetectorPins,
    mode: AtomicUsize,
    triggered: AtomicBool,
    authority: PathBuf,
    revoked: Value,
    database: PathBuf,
    wrote_during_callback: AtomicBool,
    calls: AtomicUsize,
}

impl Inspector {
    fn new(fixture: &Fixture) -> Self {
        let mut revoked = fixture.authority.clone();
        revoked["grants"] = json!([]);
        Self {
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            mode: AtomicUsize::new(0),
            triggered: AtomicBool::new(false),
            authority: fixture.path.join("authority.json"),
            revoked,
            database: fixture.directory().join("execution.sqlite"),
            wrote_during_callback: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        }
    }
}

impl PrivacyInspector for Inspector {
    fn detector(&self) -> &DetectorPins {
        &self.pins
    }
    fn configuration_binding(&self) -> String {
        "d".repeat(64)
    }
    fn timeout_ms(&self) -> u64 {
        10_000
    }
    fn inspect(
        &self,
        source: &Source,
        limits: Limits,
        _deadline_ms: u64,
    ) -> Result<NeuralAssessment, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.mode.load(Ordering::SeqCst) == 4 {
            self.triggered.store(true, Ordering::SeqCst);
            return Err(PRIVATE.into());
        }
        if source.content() == "n" {
            match self.mode.load(Ordering::SeqCst) {
                1 => {
                    self.triggered.store(true, Ordering::SeqCst);
                    return Err(PRIVATE.into());
                }
                2 => {
                    self.triggered.store(true, Ordering::SeqCst);
                    fs::write(&self.authority, self.revoked.to_string()).unwrap();
                }
                3 => {
                    self.triggered.store(true, Ordering::SeqCst);
                    let db = rusqlite::Connection::open(&self.database).unwrap();
                    db.busy_timeout(std::time::Duration::ZERO).unwrap();
                    self.wrote_during_callback.store(
                        db.execute("UPDATE metadata SET cancelled=cancelled", [])
                            .is_ok(),
                        Ordering::SeqCst,
                    );
                }
                _ => {}
            }
        }
        let request = self.pins.request(source, limits).unwrap();
        let ranges = if source.content().is_empty() {
            json!([])
        } else {
            json!([{"start":0,"end":source.content().chars().count()}])
        };
        NeuralAssessment::from_value(&json!({"version":"mas-privacy-worker-response/1","source_id":source.id(),"source_token":request["source_token"],"input_bytes":source.content().len(),"detector":self.pins.value(),"offset_unit":"unicode-codepoints","completion":"complete","checked_ranges":ranges,"omissions":[],"findings":[]}),source,&self.pins,limits).map_err(|_|"invalid-fixture".into())
    }
}

#[test]
fn live_owner_reads_are_exact_read_only_and_do_not_recover_executing_effects() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let admission = engine
        .inner
        .begin_effect(&parent, "pending", &json!({"format":"fixture/1","n":1}))
        .unwrap();
    let before = retained_files(&fixture.directory());
    let view = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "pending",
    )
    .unwrap();
    assert_eq!(view["state"], "executing");
    assert_eq!(view["external_execution"], "unknown");
    assert_eq!(view["effect_id"], admission["invocation_id"]);
    assert_eq!(retained_files(&fixture.directory()), before);
    assert_eq!(
        read_step(
            &lock(&engine.inner.journal).unwrap(),
            admission["invocation_id"].as_str().unwrap()
        )
        .unwrap()
        .unwrap()
        .state,
        "executing"
    );
}

#[test]
fn authorised_parent_absence_does_not_certify_external_non_execution() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "present", "completed");
    let view = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "different-key",
    )
    .unwrap();
    assert_eq!(view["state"], "not-recorded");
    assert_eq!(view["external_execution"], "unknown");
    for name in ["effect_id", "descriptor", "output", "evidence"] {
        assert!(view[name].is_null());
    }
    assert_ne!(view["external_execution"], "not-sent");
}

#[test]
fn foreign_namespace_parent_actor_and_denied_queries_share_one_public_failure() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "present", "completed");
    let foreign = fixture.scoped_session("tenant-b", "grant-b");
    for result in [
        read_governed_effect(
            &fixture.directory(),
            &foreign,
            "worker",
            &parent.id,
            "present",
        ),
        read_governed_effect(
            &fixture.directory(),
            &session,
            "other",
            &parent.id,
            "present",
        ),
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            "task-absent",
            "present",
        ),
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "invalid\nprivate-key",
        ),
    ] {
        assert_eq!(result.unwrap_err(), UNAVAILABLE);
    }
    let denied = session.with_authentication_check(Arc::new(|| Err(PRIVATE.into())));
    assert_eq!(
        read_governed_effect(
            &fixture.directory(),
            &denied,
            "worker",
            &parent.id,
            "present"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
    assert_eq!(
        read_governed_effect(
            &fixture.path.join("missing-run"),
            &denied,
            "worker",
            &parent.id,
            "present"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
}

#[test]
fn completed_host_settlement_never_promotes_unknown_external_execution() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "unknown");
    let view = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "observed",
    )
    .unwrap();
    assert_eq!(view["state"], "completed");
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "unknown");
    assert_eq!(view["output"], json!({"n":2}));
}

#[test]
fn conflicting_or_missing_completion_receipts_fail_without_repairing_storage() {
    for corrupt in ["missing", "changed-output", "changed-id", "changed-version"] {
        let fixture = Fixture::new();
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        let id = fixture.completed(&engine, &parent, "observed", "completed");
        {
            let db = lock(&engine.inner.journal).unwrap();
            if corrupt == "missing" {
                db.execute("DELETE FROM governed_completions WHERE id=?", [&id])
                    .unwrap();
            } else {
                let body: String = db
                    .query_row(
                        "SELECT body FROM governed_completions WHERE id=?",
                        [&id],
                        |row| row.get(0),
                    )
                    .unwrap();
                let mut body: Value = serde_json::from_str(&body).unwrap();
                match corrupt {
                    "changed-output" => body["output"] = json!({"n":3}),
                    "changed-id" => body["effect_id"] = json!("eff-other"),
                    _ => body["version"] = json!("unapproved/99"),
                }
                db.execute(
                    "UPDATE governed_completions SET body=? WHERE id=?",
                    rusqlite::params![body.to_string(), id],
                )
                .unwrap();
            }
        }
        let before = retained_files(&fixture.directory());
        assert_eq!(
            read_governed_effect(
                &fixture.directory(),
                &session,
                "worker",
                &parent.id,
                "observed"
            )
            .unwrap_err(),
            UNAVAILABLE
        );
        assert_eq!(retained_files(&fixture.directory()), before);
    }
}

#[test]
fn completed_payload_inspection_outage_preserves_authorised_host_facts() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new(&fixture));
    let session = fixture.inspected(inspector.clone());
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    inspector.mode.store(1, Ordering::SeqCst);
    let view = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "observed",
    )
    .unwrap();
    assert!(inspector.triggered.load(Ordering::SeqCst));
    assert_eq!(view["state"], "completed");
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "withheld");
    assert!(view["output"].is_null() && view["descriptor"].is_null() && view["evidence"].is_null());
    assert!(!view.to_string().contains(PRIVATE));
}

#[test]
fn revocation_during_payload_callback_denies_the_entire_view() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new(&fixture));
    let session = fixture.inspected(inspector.clone());
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    inspector.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
    assert!(inspector.triggered.load(Ordering::SeqCst));
}

#[test]
fn read_snapshot_is_released_before_privacy_callbacks() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new(&fixture));
    let session = fixture.inspected(inspector.clone());
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    inspector.mode.store(3, Ordering::SeqCst);
    let view = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "observed",
    )
    .unwrap();
    assert!(inspector.triggered.load(Ordering::SeqCst));
    assert!(inspector.wrote_during_callback.load(Ordering::SeqCst));
    assert_eq!(view["payload_disposition"], "available");
    assert_eq!(view["output"], json!({"n":2}));
}

#[test]
fn root_helper_and_explicit_parent_select_the_same_bound_effect() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    let explicit = read_governed_effect(
        &fixture.directory(),
        &session,
        "worker",
        &parent.id,
        "observed",
    )
    .unwrap();
    let root =
        read_governed_root_effect(&fixture.directory(), &session, "worker", "observed").unwrap();
    assert_eq!(root, explicit);
    assert_eq!(root["actor"], "worker");
    assert_eq!(root["parent_invocation"], parent.id);
    assert_eq!(root["key"], "observed");
}

#[test]
fn execution_authority_does_not_grant_resolution_or_query_processing() {
    let fixture = Fixture::new();
    let mut authority = fixture.authority.clone();
    for policy in authority["policies"].as_array_mut().unwrap() {
        policy["rules"]
            .as_array_mut()
            .unwrap()
            .retain(|rule| rule["operation"] != "resolve");
    }
    fixture.write_authority(&authority);
    let inspector = Arc::new(Inspector::new(&fixture));
    let session = fixture.inspected(inspector.clone());
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    let calls = inspector.calls.load(Ordering::SeqCst);
    assert!(calls > 0);
    assert_eq!(
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
    assert_eq!(inspector.calls.load(Ordering::SeqCst), calls);
}

#[test]
fn total_query_inspection_outage_denies_even_known_completion_metadata() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new(&fixture));
    let session = fixture.inspected(inspector.clone());
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    let before = retained_files(&fixture.directory());
    inspector.mode.store(4, Ordering::SeqCst);
    assert_eq!(
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
    assert!(inspector.triggered.load(Ordering::SeqCst));
    assert_eq!(retained_files(&fixture.directory()), before);
}

#[test]
fn resolution_byte_limit_covers_the_entire_envelope_and_metadata_fallback() {
    let baseline = Fixture::new();
    let session = baseline.session();
    let engine = baseline.engine(session.clone());
    let parent = baseline.invocation(&engine);
    baseline.completed(&engine, &parent, "observed", "completed");
    let full = read_governed_effect(
        &baseline.directory(),
        &session,
        "worker",
        &parent.id,
        "observed",
    )
    .unwrap();
    assert_eq!(full["payload_disposition"], "available");
    let mut minimal = full.clone();
    for field in ["descriptor", "output", "evidence"] {
        minimal[field] = Value::Null;
    }
    minimal["payload_disposition"] = json!("withheld");
    let metadata_bytes = minimal.to_string().len();
    assert!(full.to_string().len() > metadata_bytes);
    assert!(metadata_bytes > full["output"].to_string().len());
    for (limit, metadata_allowed) in [(metadata_bytes, true), (metadata_bytes - 1, false)] {
        let fixture = Fixture::new();
        let mut authority = fixture.authority.clone();
        for policy in authority["policies"].as_array_mut().unwrap() {
            for rule in policy["rules"].as_array_mut().unwrap() {
                if rule["operation"] == "resolve" {
                    rule["max_bytes"] = json!(limit);
                }
            }
        }
        fixture.write_authority(&authority);
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        fixture.completed(&engine, &parent, "observed", "completed");
        let result = read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed",
        );
        if metadata_allowed {
            let view = result.unwrap();
            assert_eq!(view.to_string().len(), metadata_bytes);
            assert_eq!(view["host_settlement"], "completed");
            assert_eq!(view["external_execution"], "completed");
            assert_eq!(view["payload_disposition"], "withheld");
            assert!(
                view["descriptor"].is_null()
                    && view["output"].is_null()
                    && view["evidence"].is_null()
            );
        } else {
            assert_eq!(result.unwrap_err(), UNAVAILABLE);
        }
    }
}

#[test]
fn nonobject_retained_descriptor_cannot_bypass_current_payload_schemas() {
    for descriptor in [json!(null), json!([]), json!(PRIVATE)] {
        let fixture = Fixture::new();
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        let id = fixture.completed(&engine, &parent, "observed", "completed");
        lock(&engine.inner.journal)
            .unwrap()
            .execute(
                "UPDATE steps SET input=? WHERE id=?",
                rusqlite::params![descriptor.to_string(), id],
            )
            .unwrap();
        let before = retained_files(&fixture.directory());
        assert_eq!(
            read_governed_effect(
                &fixture.directory(),
                &session,
                "worker",
                &parent.id,
                "observed"
            )
            .unwrap_err(),
            UNAVAILABLE
        );
        assert_eq!(retained_files(&fixture.directory()), before);
        // Supply a deliberately inconsistent historical schema fixture with a
        // matching host commitment to challenge the separate schema gate.
        lock(&engine.inner.journal)
            .unwrap()
            .execute(
                "UPDATE governed_inputs SET original_commitment=? WHERE id=?",
                rusqlite::params![session.commitment(&descriptor).unwrap(), id],
            )
            .unwrap();
        let before = retained_files(&fixture.directory());
        let view = read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed",
        )
        .unwrap();
        assert_eq!(view["host_settlement"], "completed");
        assert_eq!(view["external_execution"], "completed");
        assert_eq!(view["payload_disposition"], "withheld");
        assert!(
            view["descriptor"].is_null() && view["output"].is_null() && view["evidence"].is_null()
        );
        assert!(!view.to_string().contains(PRIVATE));
        assert_eq!(retained_files(&fixture.directory()), before);
    }
}

#[test]
fn oversized_multibyte_retained_text_is_rejected_by_bytes_before_decoding() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let id = fixture.completed(&engine, &parent, "observed", "completed");
    let descriptor = json!({"format":"fixture/1","n":1,"private":"é".repeat(40_000)}).to_string();
    assert!(descriptor.len() > 65_536);
    assert!(descriptor.chars().count() < 65_536);
    lock(&engine.inner.journal)
        .unwrap()
        .execute(
            "UPDATE steps SET input=? WHERE id=?",
            rusqlite::params![descriptor, id],
        )
        .unwrap();
    let before = retained_files(&fixture.directory());
    assert_eq!(
        read_governed_effect(
            &fixture.directory(),
            &session,
            "worker",
            &parent.id,
            "observed"
        )
        .unwrap_err(),
        UNAVAILABLE
    );
    assert_eq!(retained_files(&fixture.directory()), before);
}

#[test]
fn substituted_valid_descriptor_and_missing_admission_commitment_are_unavailable() {
    for missing in [false, true] {
        let fixture = Fixture::new();
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        let id = fixture.completed(&engine, &parent, "observed", "completed");
        let db = lock(&engine.inner.journal).unwrap();
        if missing {
            db.execute("DELETE FROM governed_inputs WHERE id=?", [&id])
                .unwrap();
        } else {
            db.execute(
                "UPDATE steps SET input=? WHERE id=?",
                rusqlite::params![json!({"format":"fixture/1","n":2}).to_string(), id],
            )
            .unwrap();
        }
        drop(db);
        let before = retained_files(&fixture.directory());
        assert_eq!(
            read_governed_effect(
                &fixture.directory(),
                &session,
                "worker",
                &parent.id,
                "observed"
            )
            .unwrap_err(),
            UNAVAILABLE
        );
        assert_eq!(retained_files(&fixture.directory()), before);
    }
}

#[test]
fn copied_journal_under_another_run_name_cannot_rebind_the_namespace() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.completed(&engine, &parent, "observed", "completed");
    let before = retained_files(&fixture.directory());
    let copied = fixture.path.join("different-run");
    fs::create_dir(&copied).unwrap();
    for (name, content) in &before {
        fs::write(copied.join(name), content).unwrap();
    }
    let copied_before = retained_files(&copied);
    assert_eq!(
        read_governed_effect(&copied, &session, "worker", &parent.id, "observed").unwrap_err(),
        UNAVAILABLE
    );
    assert_eq!(retained_files(&fixture.directory()), before);
    assert_eq!(retained_files(&copied), copied_before);
}
