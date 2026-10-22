//! Challenge source restrictions through the existing invocation/effect owners.
use super::{exposure, read_governed_root_source, GovernedSession};
use crate::{digest, lock, read_step, Engine, Step};
use mas_host_privacy::{DetectorPins, Limits, NeuralAssessment, Source};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const PRIVATE: &str = "exposure-private-person@example.invalid";

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub(super) struct Fixture {
    pub(super) path: PathBuf,
    pub(super) config: Value,
    pub(super) authority: Value,
    pub(super) bundle: Value,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mas-exposure-adversarial-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("key"), [37u8; 32]).unwrap();
        fs::set_permissions(path.join("key"), fs::Permissions::from_mode(0o600)).unwrap();
        let schema = object(
            json!({"n":{"type":"integer","minimum":0,"maximum":100}}),
            &["n"],
        );
        let evidence = object(
            json!({"execution":{"type":"string","enum":["completed","unknown"]}}),
            &["execution"],
        );
        let descriptor = |format: &str| {
            object(
                json!({"format":{"type":"string","enum":[format]},"n":{"type":"integer","minimum":0,"maximum":100}}),
                &["format", "n"],
            )
        };
        let expires = crate::now_ms().unwrap() + 600_000;
        let mut grants = Vec::new();
        let mut policies = Vec::new();
        for (tenant, grant) in [("tenant-a", "grant-a"), ("tenant-b", "grant-b")] {
            let mut base_rules = Vec::new();
            for actor in ["worker", "other"] {
                for (operation, recipient, destination) in [
                    ("invoke", "worker:worker", "local-worker"),
                    ("invoke", "worker:other", "local-other"),
                    ("deliver", "principal-a", "client"),
                    ("resolve", "principal-a", "client"),
                    ("inspect", "privacy-inspector", "local-detector"),
                    ("effect", "operation:fixture-read/1", "fixture-source"),
                    ("effect", "operation:fixture-derive/1", "fixture-source"),
                    ("effect", "operation:fixture-action/1", "fixture-action"),
                ] {
                    base_rules.push(json!({"operation":operation,"principal":"principal-a","actor":actor,
                        "producer":"host","recipient":recipient,"purpose":"diagnosis","destination":destination,
                        "location":"uk","classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":65536}));
                }
            }
            let restricted_rules = base_rules
                .iter()
                .filter(|rule| rule["destination"] != "fixture-action")
                .cloned()
                .collect::<Vec<_>>();
            grants.push(json!({"version":"mas-current-authority/1","status":"active","tenant":tenant,
                "principal":"principal-a","grant_ref":grant,"grant_epoch":1,"policy_version":"policy-v1",
                "valid_from_ms":0,"expires_at_ms":expires}));
            for (id, rules) in [
                ("policy-base", base_rules),
                ("policy-source", restricted_rules),
            ] {
                policies.push(json!({"version":"mas-data-use-policy/1","id":format!("{id}-{tenant}"),
                    "tenant":tenant,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expires,"rules":rules}));
            }
        }
        let authority =
            json!({"version":"mas-governed-authority/1","grants":grants,"policies":policies});
        fs::write(path.join("authority.json"), authority.to_string()).unwrap();
        let agents = ["worker", "other"].map(|actor| {
            json!({"actor":actor,"destination":format!("local-{actor}"),"input_schema":schema,"output_schema":schema,
                "effects":[
                    {"format":"fixture-read/1","destination":"fixture-source","descriptor_schema":descriptor("fixture-read/1"),
                     "output_schema":schema,"evidence_schema":evidence,"source_policy_refs":["policy-source-tenant-a"]},
                    {"format":"fixture-action/1","destination":"fixture-action","descriptor_schema":descriptor("fixture-action/1"),
                     "output_schema":schema,"evidence_schema":evidence},
                    {"format":"fixture-derive/1","destination":"fixture-source","descriptor_schema":descriptor("fixture-derive/1"),
                     "output_schema":schema,"evidence_schema":evidence}]})
        });
        let config = json!({"version":"mas-governed-local/2","authority_file":path.join("authority.json"),
            "commitment_key_file":path.join("key"),"commitment_key_id":"key-v1","location":"uk",
            "baseline_policy_refs":["policy-base-tenant-a"],"agents":agents});
        let executable = fs::canonicalize("/usr/bin/true").unwrap();
        let hash = digest(&fs::read(&executable).unwrap());
        let agents = ["worker", "other"].map(|actor| {
            json!({"name":actor,"executable":executable,"sha256":hash,"handler":"Assess","operation":"Assess",
                "environment":[],"working_directory":path})
        });
        let bundle = json!({"version":"mas-bundle/3","agents":agents,
            "topology":{"agents":["client","worker","other"],
                "edges":[["client","worker"],["worker","client"],["worker","other"],["other","worker"]],
                "application":"agent-task/1","max_calls":200,"wall_seconds":600}});
        Self {
            path,
            config,
            authority,
            bundle,
        }
    }

    pub(super) fn session(&self) -> GovernedSession {
        GovernedSession::from_value(
            &self.config,
            "tenant-a",
            "principal-a",
            "diagnosis",
            "grant-a",
        )
        .unwrap()
    }

    pub(super) fn engine(&self, session: GovernedSession) -> Engine {
        Engine::open_governed(&self.bundle, &self.path, "exposure-run", session).unwrap()
    }

    pub(super) fn directory(&self) -> PathBuf {
        self.path.join("exposure-run")
    }

    pub(super) fn invocation(&self, engine: &Engine) -> Step {
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
            .unwrap()
    }

    pub(super) fn complete(
        &self,
        engine: &Engine,
        parent: &Step,
        key: &str,
        format: &str,
    ) -> String {
        let admission = engine
            .inner
            .begin_effect(parent, key, &json!({"format":format,"n":1}))
            .unwrap();
        assert_eq!(admission["state"], "execute");
        engine
            .inner
            .complete_effect(
                parent,
                key,
                &json!({"n":2}),
                &json!({"execution":"completed"}),
            )
            .unwrap();
        admission["invocation_id"].as_str().unwrap().to_owned()
    }

    pub(super) fn write_authority(&self, authority: &Value) {
        fs::write(self.path.join("authority.json"), authority.to_string()).unwrap();
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

#[test]
fn acquired_restriction_denies_empty_lineage_action_before_preparation() {
    let fixture = Fixture::new();
    let engine = fixture.engine(fixture.session());
    let parent = fixture.invocation(&engine);
    fixture.complete(
        &engine,
        &parent,
        "permitted-before-read",
        "fixture-action/1",
    );
    fixture.complete(&engine, &parent, "read", "fixture-read/1");
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "blocked-after-read",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
    let id = engine
        .inner
        .step_id(&parent.id, "blocked-after-read", "effect");
    assert!(read_step(&lock(&engine.inner.journal).unwrap(), &id)
        .unwrap()
        .is_none());
}

#[test]
fn source_restriction_survives_restart_and_does_not_refetch() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let id = fixture.complete(&engine, &parent, "read", "fixture-read/1");
    drop(engine);
    let engine = fixture.engine(session);
    let replay = engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .unwrap();
    assert_ne!(replay["state"], "execute");
    assert_eq!(replay["state"], "cached");
    assert_eq!(
        read_step(&lock(&engine.inner.journal).unwrap(), &id)
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "blocked-after-restart",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
}

#[test]
fn v1_configuration_rejects_v2_policy_selection_instead_of_ignoring_it() {
    let fixture = Fixture::new();
    let mut config = fixture.config.clone();
    config["version"] = json!("mas-governed-local/1");
    assert!(GovernedSession::from_value(
        &config,
        "tenant-a",
        "principal-a",
        "diagnosis",
        "grant-a"
    )
    .is_err());
    config
        .as_object_mut()
        .unwrap()
        .remove("baseline_policy_refs");
    assert!(GovernedSession::from_value(
        &config,
        "tenant-a",
        "principal-a",
        "diagnosis",
        "grant-a"
    )
    .is_err());
}

#[test]
fn identical_acquisition_with_baseline_source_policy_keeps_action_permitted() {
    let mut fixture = Fixture::new();
    for actor in fixture.config["agents"].as_array_mut().unwrap() {
        actor["effects"][0]["source_policy_refs"] = json!(["policy-base-tenant-a"]);
    }
    let engine = fixture.engine(fixture.session());
    let parent = fixture.invocation(&engine);
    fixture.complete(&engine, &parent, "read", "fixture-read/1");
    fixture.complete(&engine, &parent, "permitted-after-read", "fixture-action/1");
}

#[test]
fn acquisition_callback_cannot_act_before_reporting_its_source_completion() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let read = engine
        .inner
        .begin_effect(
            &parent,
            "pending-read",
            &json!({"format":"fixture-read/1","n":1}),
        )
        .unwrap();
    assert_eq!(read["state"], "execute");
    // A cooperative callback may use Context again after acquisition but
    // before effect_complete. Its registered restrictions must already apply.
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "premature-action",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
    let action_id = engine
        .inner
        .step_id(&parent.id, "premature-action", "effect");
    assert!(read_step(&lock(&engine.inner.journal).unwrap(), &action_id)
        .unwrap()
        .is_none());
    drop(engine);
    let engine = fixture.engine(session);
    assert_eq!(
        read_step(
            &lock(&engine.inner.journal).unwrap(),
            read["invocation_id"].as_str().unwrap()
        )
        .unwrap()
        .unwrap()
        .state,
        "unknown"
    );
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "action-after-unknown-read",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
}

#[test]
fn source_projection_cannot_reconstruct_discarded_original_bytes() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = engine
        .inner
        .prepare(
            "",
            "root",
            "request",
            "client",
            "worker",
            &json!({"n":1,"discarded":PRIVATE}),
        )
        .unwrap();
    let admission = engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .unwrap();
    engine
        .inner
        .complete_effect(
            &parent,
            "read",
            &json!({"n":2,"discarded":PRIVATE}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let before = retained_files(&fixture.directory());
    let view = read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
    assert_eq!(view["effect_id"], admission["invocation_id"]);
    assert_eq!(view["projection"], json!({"n":2}));
    assert_eq!(view["raw_source_disposition"], "not-retained");
    assert!(!view.to_string().contains(PRIVATE));
    assert_eq!(retained_files(&fixture.directory()), before);
    assert!(!before.values().any(|bytes| bytes
        .windows(PRIVATE.len())
        .any(|part| part == PRIVATE.as_bytes())));
}

#[test]
fn every_source_metadata_component_is_bound_and_missing_ancestors_hold() {
    for corruption in [
        "policy",
        "parents",
        "cycle",
        "expiry",
        "payload",
        "missing-parent",
        "mapping",
    ] {
        let fixture = Fixture::new();
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        let effect = fixture.complete(&engine, &parent, "read", "fixture-read/1");
        let view =
            read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
        let source_id = view["source_id"].as_str().unwrap();
        let mut metadata = view["source_metadata"].clone();
        let db = lock(&engine.inner.journal).unwrap();
        match corruption {
            "payload" => {
                db.execute(
                    "UPDATE steps SET output=? WHERE id=?",
                    rusqlite::params![json!({"n":3}).to_string(), effect],
                )
                .unwrap();
            }
            "missing-parent" => {
                let ancestor = metadata["parents"]
                    .as_array()
                    .unwrap()
                    .first()
                    .unwrap()
                    .as_str()
                    .unwrap();
                db.execute("DELETE FROM governed_sources WHERE id=?", [ancestor])
                    .unwrap();
            }
            "mapping" => {
                db.execute(
                    "DELETE FROM governed_source_steps WHERE source_id=?",
                    [source_id],
                )
                .unwrap();
            }
            _ => {
                match corruption {
                    "policy" => metadata["policy_refs"] = json!(["policy-base-tenant-a"]),
                    "parents" => metadata["parents"] = json!([]),
                    "cycle" => metadata["parents"] = json!([source_id]),
                    _ => {
                        metadata["expires_at_ms"] =
                            json!(metadata["expires_at_ms"].as_u64().unwrap() + 600_000)
                    }
                }
                db.execute(
                    "UPDATE governed_sources SET body=? WHERE id=?",
                    rusqlite::params![metadata.to_string(), source_id],
                )
                .unwrap();
            }
        }
        drop(db);
        let before = retained_files(&fixture.directory());
        assert_eq!(
            read_governed_root_source(&fixture.directory(), &session, "worker", "read")
                .unwrap_err(),
            "governed-evidence-unavailable"
        );
        assert_eq!(retained_files(&fixture.directory()), before);
    }
}

#[test]
fn missing_or_edited_exposure_manifest_cannot_become_an_empty_scope() {
    for missing in [false, true] {
        let mut fixture = Fixture::new();
        for actor in fixture.config["agents"].as_array_mut().unwrap() {
            actor["effects"][0]["source_policy_refs"] = json!(["policy-base-tenant-a"]);
        }
        let engine = fixture.engine(fixture.session());
        let parent = fixture.invocation(&engine);
        fixture.complete(&engine, &parent, "read", "fixture-read/1");
        fixture.complete(&engine, &parent, "positive-action", "fixture-action/1");
        let db = lock(&engine.inner.journal).unwrap();
        if missing {
            db.execute(
                "DELETE FROM governed_exposures WHERE invocation_id=?",
                [&parent.id],
            )
            .unwrap();
        } else {
            let body: String = db
                .query_row(
                    "SELECT body FROM governed_exposures WHERE invocation_id=?",
                    [&parent.id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut body: Value = serde_json::from_str(&body).unwrap();
            body["roots"] = json!([]);
            body["source_count"] = json!(0);
            db.execute(
                "UPDATE governed_exposures SET body=? WHERE invocation_id=?",
                rusqlite::params![body.to_string(), parent.id],
            )
            .unwrap();
        }
        drop(db);
        assert!(engine
            .inner
            .begin_effect(
                &parent,
                "must-not-reset-scope",
                &json!({"format":"fixture-action/1","n":1})
            )
            .is_err());
        let id = engine
            .inner
            .step_id(&parent.id, "must-not-reset-scope", "effect");
        assert!(read_step(&lock(&engine.inner.journal).unwrap(), &id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn ancestor_beyond_thirty_two_still_constrains_the_current_action() {
    for restricted in [false, true] {
        let mut fixture = Fixture::new();
        if !restricted {
            for actor in fixture.config["agents"].as_array_mut().unwrap() {
                actor["effects"][0]["source_policy_refs"] = json!(["policy-base-tenant-a"]);
            }
        }
        let session = fixture.session();
        let engine = fixture.engine(session.clone());
        let parent = fixture.invocation(&engine);
        fixture.complete(&engine, &parent, "original-read", "fixture-read/1");
        for n in 0..33 {
            fixture.complete(&engine, &parent, &format!("derive-{n}"), "fixture-derive/1");
        }
        let closure =
            exposure::load_invocation(&lock(&engine.inner.journal).unwrap(), &session, &parent.id)
                .unwrap();
        assert!(closure.sources.len() > 33);
        let read = read_governed_root_source(&fixture.directory(), &session, "worker", "derive-32")
            .unwrap();
        assert_eq!(
            read["lineage"].as_array().unwrap().len(),
            closure.sources.len()
        );
        let outcome = engine.inner.begin_effect(
            &parent,
            "action-after-33-ancestors",
            &json!({"format":"fixture-action/1","n":1}),
        );
        if restricted {
            assert!(outcome.is_err());
        } else {
            assert_eq!(outcome.unwrap()["state"], "execute");
        }
    }
}

#[test]
fn source_reader_rejects_foreign_namespace_actor_and_current_revocation() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    fixture.complete(&engine, &parent, "read", "fixture-read/1");
    let mut foreign = fixture.config.clone();
    foreign["baseline_policy_refs"] = json!(["policy-base-tenant-b"]);
    for actor in foreign["agents"].as_array_mut().unwrap() {
        actor["effects"][0]["source_policy_refs"] = json!(["policy-source-tenant-b"]);
    }
    let foreign =
        GovernedSession::from_value(&foreign, "tenant-b", "principal-a", "diagnosis", "grant-b")
            .unwrap();
    for result in [
        read_governed_root_source(&fixture.directory(), &foreign, "worker", "read"),
        read_governed_root_source(&fixture.directory(), &session, "other", "read"),
    ] {
        assert_eq!(result.unwrap_err(), "governed-evidence-unavailable");
    }
    let mut revoked = fixture.authority.clone();
    revoked["grants"] = json!([]);
    fixture.write_authority(&revoked);
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap_err(),
        "governed-evidence-unavailable"
    );
}

#[test]
fn aborting_execute_transition_cannot_strand_a_pending_prepared_acquisition() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let deadline = engine.inner.deadline_ms;
    let before: String = lock(&engine.inner.journal)
        .unwrap()
        .query_row(
            "SELECT body FROM governed_exposures WHERE invocation_id=?",
            [&parent.id],
            |r| r.get(0),
        )
        .unwrap();
    lock(&engine.inner.journal).unwrap().execute_batch("CREATE TRIGGER abort_exposure_execute BEFORE UPDATE OF state ON steps WHEN NEW.kind='effect' AND NEW.state='executing' BEGIN SELECT RAISE(ABORT,'fixture_execute_abort'); END;").unwrap();
    assert!(engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .is_err());
    let after: String = lock(&engine.inner.journal)
        .unwrap()
        .query_row(
            "SELECT body FROM governed_exposures WHERE invocation_id=?",
            [&parent.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, before);
    let effect = engine.inner.step_id(&parent.id, "read", "effect");
    assert_eq!(
        read_step(&lock(&engine.inner.journal).unwrap(), &effect)
            .unwrap()
            .unwrap()
            .state,
        "prepared"
    );
    lock(&engine.inner.journal)
        .unwrap()
        .execute_batch("DROP TRIGGER abort_exposure_execute;")
        .unwrap();
    drop(engine);
    let engine = fixture.engine(session);
    assert_eq!(engine.inner.deadline_ms, deadline);
    let admission = engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .unwrap();
    assert_eq!(admission["state"], "execute");
}

struct Inspector {
    pins: DetectorPins,
    calls: AtomicUsize,
}

impl Inspector {
    fn new() -> Self {
        Self {
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            calls: AtomicUsize::new(0),
        }
    }
}

impl super::PrivacyInspector for Inspector {
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
fn inherited_inspection_denial_stops_the_detector_before_child_persistence() {
    let fixture = Fixture::new();
    let mut authority = fixture.authority.clone();
    let rules = authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["id"] == "policy-source-tenant-a")
        .unwrap()["rules"]
        .as_array_mut()
        .unwrap();
    rules.retain(|rule| !(rule["actor"] == "other" && rule["operation"] == "inspect"));
    fixture.write_authority(&authority);
    let inspector = Arc::new(Inspector::new());
    let session = fixture
        .session()
        .with_privacy_inspector(inspector.clone(), "local-detector")
        .unwrap();
    let engine = fixture.engine(session);
    let parent = fixture.invocation(&engine);
    engine
        .inner
        .prepare(
            &parent.id,
            "baseline-child",
            "request",
            "worker",
            "other",
            &json!({"n":2}),
        )
        .unwrap();
    assert!(inspector.calls.load(Ordering::SeqCst) > 0);
    fixture.complete(&engine, &parent, "read", "fixture-read/1");
    let before = inspector.calls.load(Ordering::SeqCst);
    assert!(engine
        .inner
        .prepare(
            &parent.id,
            "restricted-child",
            "request",
            "worker",
            "other",
            &json!({"n":2})
        )
        .is_err());
    assert_eq!(inspector.calls.load(Ordering::SeqCst), before);
    let child = engine
        .inner
        .step_id(&parent.id, "restricted-child", "request");
    assert!(read_step(&lock(&engine.inner.journal).unwrap(), &child)
        .unwrap()
        .is_none());
}

#[test]
fn registered_source_permissions_apply_before_acquisition_execute() {
    for denied in ["effect", "inspect"] {
        let fixture = Fixture::new();
        let mut authority = fixture.authority.clone();
        let rules = authority["policies"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == "policy-source-tenant-a")
            .unwrap()["rules"]
            .as_array_mut()
            .unwrap();
        rules.retain(|rule| !(rule["actor"] == "worker" && rule["operation"] == denied));
        fixture.write_authority(&authority);
        let inspector = Arc::new(Inspector::new());
        let session = fixture
            .session()
            .with_privacy_inspector(inspector.clone(), "local-detector")
            .unwrap();
        let engine = fixture.engine(session);
        let parent = fixture.invocation(&engine);
        let before = inspector.calls.load(Ordering::SeqCst);
        assert!(before > 0);
        assert!(engine
            .inner
            .begin_effect(
                &parent,
                "denied-acquisition",
                &json!({"format":"fixture-read/1","n":1})
            )
            .is_err());
        if denied == "inspect" {
            assert_eq!(inspector.calls.load(Ordering::SeqCst), before);
        }
        let id = engine
            .inner
            .step_id(&parent.id, "denied-acquisition", "effect");
        assert!(read_step(&lock(&engine.inner.journal).unwrap(), &id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn invalid_stored_inspection_receipt_cannot_hide_behind_source_projection() {
    let fixture = Fixture::new();
    let inspector = Arc::new(Inspector::new());
    let session = fixture
        .session()
        .with_privacy_inspector(inspector, "local-detector")
        .unwrap();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let effect = fixture.complete(&engine, &parent, "read", "fixture-read/1");
    let clean =
        read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
    assert_eq!(clean["payload_disposition"], "available");
    assert_eq!(
        engine
            .inner
            .governed_completion_ack(&effect)
            .unwrap()
            .unwrap()["output"],
        json!({"n":2})
    );
    let db = lock(&engine.inner.journal).unwrap();
    let evidence: String = db
        .query_row("SELECT evidence FROM steps WHERE id=?", [&effect], |r| {
            r.get(0)
        })
        .unwrap();
    let clean_evidence = evidence.clone();
    let mut evidence: Value = serde_json::from_str(&evidence).unwrap();
    assert_eq!(
        evidence["inspection"]["version"],
        "mas-governed-inspection-receipt/1"
    );
    evidence["inspection"]["projection_binding"] = json!("e".repeat(64));
    db.execute(
        "UPDATE steps SET evidence=? WHERE id=?",
        rusqlite::params![evidence.to_string(), effect],
    )
    .unwrap();
    drop(db);
    let view = read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "withheld");
    assert!(view["projection"].is_null());

    let db = lock(&engine.inner.journal).unwrap();
    db.execute(
        "UPDATE steps SET evidence=? WHERE id=?",
        rusqlite::params![clean_evidence, effect],
    )
    .unwrap();
    let mut completion = super::storage::completion(&db, &effect).unwrap();
    completion["output"] = json!({"n":99});
    db.execute(
        "UPDATE governed_completions SET body=? WHERE id=?",
        rusqlite::params![completion.to_string(), effect],
    )
    .unwrap();
    drop(db);
    assert!(engine.inner.governed_completion_ack(&effect).is_err());
    let retained = read_step(&lock(&engine.inner.journal).unwrap(), &effect)
        .unwrap()
        .unwrap();
    assert_eq!(retained.state, "completed");
    assert_eq!(retained.output, Some(json!({"n":2})));
}

#[test]
fn parallel_children_keep_frozen_input_but_parent_accumulates_returned_exposure() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let frozen =
        exposure::load_invocation(&lock(&engine.inner.journal).unwrap(), &session, &parent.id)
            .unwrap();
    let first = engine
        .inner
        .prepare_with_exposure(
            &parent.id,
            "parallel-first",
            "request",
            "worker",
            "other",
            &json!({"n":1}),
            Some(&frozen),
        )
        .unwrap();
    fixture.complete(&engine, &first, "read", "fixture-read/1");
    engine
        .inner
        .commit_completed(&first.id, &json!({"n":2}))
        .unwrap();
    engine
        .inner
        .governed_request_delivery(&first, &json!({"n":2}))
        .unwrap();
    let second = engine
        .inner
        .prepare_with_exposure(
            &parent.id,
            "parallel-second",
            "request",
            "worker",
            "other",
            &json!({"n":1}),
            Some(&frozen),
        )
        .unwrap();
    fixture.complete(
        &engine,
        &second,
        "permitted-from-frozen-context",
        "fixture-action/1",
    );
    engine
        .inner
        .commit_completed(&second.id, &json!({"n":2}))
        .unwrap();
    engine
        .inner
        .governed_request_delivery(&second, &json!({"n":2}))
        .unwrap();
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "parent-action-after-both",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
    let later = engine
        .inner
        .prepare(
            &parent.id,
            "future-child",
            "request",
            "worker",
            "other",
            &json!({"n":1}),
        )
        .unwrap();
    assert!(engine
        .inner
        .begin_effect(
            &later,
            "must-inherit-later-restriction",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
}

#[test]
fn renewing_current_grant_cannot_extend_original_source_expiry() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let mut short = fixture.authority.clone();
    let admitted_expiry = crate::now_ms().unwrap() + 8_000;
    short["grants"][0]["expires_at_ms"] = json!(admitted_expiry);
    fixture.write_authority(&short);
    let admission = engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .unwrap();
    assert_eq!(admission["state"], "execute");
    let effect = admission["invocation_id"].as_str().unwrap().to_owned();
    // Parent input has a longer expiry. Renew current grant during the
    // acquisition callback, before reporting its completion, to isolate the
    // pending admission's immutable minimum from both surrounding boundaries.
    fixture.write_authority(&fixture.authority);
    engine
        .inner
        .complete_effect(
            &parent,
            "read",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let original =
        read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
    let expiry = original["source_metadata"]["expires_at_ms"]
        .as_u64()
        .unwrap();
    assert_eq!(expiry, admitted_expiry);
    session.check_current().unwrap();
    let replay = engine
        .inner
        .begin_effect(&parent, "read", &json!({"format":"fixture-read/1","n":1}))
        .unwrap();
    assert_eq!(replay["state"], "cached");
    let retained =
        read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap();
    assert_eq!(retained["source_id"], original["source_id"]);
    assert_eq!(retained["source_metadata"], original["source_metadata"]);
    while crate::now_ms().unwrap() <= expiry {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    session.check_current().unwrap();
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "read").unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert_eq!(
        read_step(&lock(&engine.inner.journal).unwrap(), &effect)
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
}

#[test]
fn closure_beyond_sixty_four_holds_without_reexecuting_known_completion() {
    let mut fixture = Fixture::new();
    for actor in fixture.config["agents"].as_array_mut().unwrap() {
        actor["effects"][0]["source_policy_refs"] = json!(["policy-base-tenant-a"]);
    }
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let mut held = None;
    for n in 0..70 {
        let key = format!("derive-limit-{n}");
        let descriptor = json!({"format":"fixture-derive/1","n":1});
        let admission = engine.inner.begin_effect(&parent, &key, &descriptor);
        let Ok(admission) = admission else {
            assert!(n >= 33);
            held = Some((key, None));
            break;
        };
        assert_eq!(admission["state"], "execute");
        let result = engine.inner.complete_effect(
            &parent,
            &key,
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        );
        let id = admission["invocation_id"].as_str().unwrap().to_owned();
        let source = read_governed_root_source(&fixture.directory(), &session, "worker", &key);
        if result.is_err() || source.is_err() {
            assert!(n >= 33);
            held = Some((key, Some(id)));
            break;
        }
    }
    let (key, completed) = held.expect("65+ source closure must not be silently truncated");
    if let Some(id) = &completed {
        assert_eq!(
            read_step(&lock(&engine.inner.journal).unwrap(), id)
                .unwrap()
                .unwrap()
                .state,
            "completed"
        );
    }
    drop(engine);
    let engine = fixture.engine(session);
    let replay =
        engine
            .inner
            .begin_effect(&parent, &key, &json!({"format":"fixture-derive/1","n":1}));
    assert!(replay.is_err() || replay.unwrap()["state"] != "execute");
    if let Some(id) = completed {
        assert_eq!(
            read_step(&lock(&engine.inner.journal).unwrap(), &id)
                .unwrap()
                .unwrap()
                .state,
            "completed"
        );
    }
}

#[test]
fn cached_effect_payload_obeys_recipient_sources_even_when_descriptor_is_permitted() {
    let mut fixture = Fixture::new();
    let baseline = fixture.authority["policies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|policy| policy["id"] == "policy-base-tenant-a")
        .unwrap()["rules"]
        .clone();
    let restricted = fixture.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|policy| policy["id"] == "policy-source-tenant-a")
        .unwrap();
    restricted["rules"] = baseline;
    for rule in restricted["rules"].as_array_mut().unwrap() {
        if rule["operation"] == "invoke" {
            rule["max_bytes"] = json!(8192);
        }
    }
    fixture.write_authority(&fixture.authority);
    let fixed_value = "x".repeat(120);
    fixture.config["agents"][0]["effects"][1]["output_schema"] = object(
        json!({"n":{"type":"integer","minimum":0,"maximum":100},
            "blob":{"type":"array","maxItems":32,
                "items":{"type":"array","maxItems":3,
                    "items":{"type":"string","enum":[fixed_value]}}}}),
        &["n", "blob"],
    );
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let descriptor = json!({"format":"fixture-action/1","n":1});
    let large = json!({"n":2,"blob":vec![vec![fixed_value;3];32]});
    assert!(large.to_string().len() > 8192);
    let admission = engine
        .inner
        .begin_effect(&parent, "large-before-source", &descriptor)
        .unwrap();
    assert_eq!(admission["state"], "execute");
    engine
        .inner
        .complete_effect(
            &parent,
            "large-before-source",
            &large,
            &json!({"execution":"completed"}),
        )
        .unwrap();
    assert_eq!(
        engine
            .inner
            .begin_effect(&parent, "large-before-source", &descriptor)
            .unwrap()["output"],
        large
    );

    fixture.complete(&engine, &parent, "new-restriction", "fixture-read/1");
    let recipient =
        exposure::load_invocation(&lock(&engine.inner.journal).unwrap(), &session, &parent.id)
            .unwrap();
    session
        .prepare_effect_with_exposure("worker", &descriptor, &recipient)
        .unwrap();
    assert!(engine
        .inner
        .begin_effect(&parent, "large-before-source", &descriptor)
        .is_err());
    let retained = read_step(
        &lock(&engine.inner.journal).unwrap(),
        admission["invocation_id"].as_str().unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(retained.state, "completed");
    assert_eq!(retained.output.as_ref(), Some(&large));
}

#[test]
fn frozen_child_reply_obeys_recipient_sources_acquired_after_input_freeze() {
    let mut fixture = Fixture::new();
    let restricted = fixture.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|policy| policy["id"] == "policy-source-tenant-a")
        .unwrap();
    restricted["rules"].as_array_mut().unwrap().retain(|rule| {
        !(rule["operation"] == "invoke"
            && rule["actor"] == "other"
            && rule["recipient"] == "worker:worker")
    });
    fixture.write_authority(&fixture.authority);
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let frozen =
        exposure::load_invocation(&lock(&engine.inner.journal).unwrap(), &session, &parent.id)
            .unwrap();
    let mut children = Vec::new();
    for key in ["reply-before-source", "reply-after-source"] {
        children.push(
            engine
                .inner
                .prepare_with_exposure(
                    &parent.id,
                    key,
                    "request",
                    "worker",
                    "other",
                    &json!({"n":1}),
                    Some(&frozen),
                )
                .unwrap(),
        );
    }
    engine
        .inner
        .commit_completed(&children[0].id, &json!({"n":2}))
        .unwrap();
    engine
        .inner
        .governed_request_delivery(&children[0], &json!({"n":2}))
        .unwrap();
    fixture.complete(&engine, &parent, "new-restriction", "fixture-read/1");

    let second_context = exposure::load_invocation(
        &lock(&engine.inner.journal).unwrap(),
        &session,
        &children[1].id,
    )
    .unwrap();
    session
        .authorize_request_admission("other", &json!({"n":1}), &second_context)
        .unwrap();
    engine
        .inner
        .commit_completed(&children[1].id, &json!({"n":2}))
        .unwrap();
    assert!(engine
        .inner
        .governed_request_delivery(&children[1], &json!({"n":2}))
        .is_err());
    let recipient =
        exposure::load_invocation(&lock(&engine.inner.journal).unwrap(), &session, &parent.id)
            .unwrap();
    assert!(recipient.sources.values().any(|source| {
        source
            .refs("policy_refs")
            .contains("policy-source-tenant-a")
    }));
}

#[test]
fn earlier_retained_effect_and_source_cannot_bypass_later_invocation_resolve_denial() {
    let mut fixture = Fixture::new();
    let restricted = fixture.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|policy| policy["id"] == "policy-source-tenant-a")
        .unwrap();
    restricted["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|rule| rule["operation"] != "resolve");
    fixture.write_authority(&fixture.authority);
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let earlier = fixture.complete(&engine, &parent, "earlier-baseline", "fixture-derive/1");
    let effect = super::read_governed_root_effect(
        &fixture.directory(),
        &session,
        "worker",
        "earlier-baseline",
    )
    .unwrap();
    assert_eq!(effect["payload_disposition"], "available");
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "earlier-baseline")
            .unwrap()["payload_disposition"],
        "available"
    );
    let acquisition = engine
        .inner
        .begin_effect(
            &parent,
            "later-restriction",
            &json!({"format":"fixture-read/1","n":1}),
        )
        .unwrap();
    assert_eq!(acquisition["state"], "execute");
    let pending_files = retained_files(&fixture.directory());
    assert_eq!(
        super::read_governed_root_effect(
            &fixture.directory(),
            &session,
            "worker",
            "earlier-baseline"
        )
        .unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "earlier-baseline")
            .unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert_eq!(pending_files, retained_files(&fixture.directory()));
    engine
        .inner
        .complete_effect(
            &parent,
            "later-restriction",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let before = retained_files(&fixture.directory());
    assert_eq!(
        super::read_governed_root_effect(
            &fixture.directory(),
            &session,
            "worker",
            "earlier-baseline"
        )
        .unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "earlier-baseline")
            .unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert_eq!(before, retained_files(&fixture.directory()));
    assert_eq!(
        read_step(&lock(&engine.inner.journal).unwrap(), &earlier)
            .unwrap()
            .unwrap()
            .state,
        "completed"
    );
}
