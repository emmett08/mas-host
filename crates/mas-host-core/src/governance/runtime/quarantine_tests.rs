//! Exercise exact source custody through the actual ingress/inspection owner.
use super::super::{exposure_adversarial_tests::Fixture, PrivacyInspector};
use super::*;
use mas_host_privacy::{DetectorPins, Limits, NeuralAssessment, Source as PrivacySource};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};
const PRIVATE: &str = "quarantine-person-8375@example.invalid";

struct Inspector {
    pins: DetectorPins,
    mode: AtomicUsize,
}
impl Inspector {
    fn new() -> Self {
        Self {
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            mode: AtomicUsize::new(0),
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
    fn contextual_json(&self) -> bool {
        true
    }
    fn retain_assessments(&self) -> bool {
        true
    }
    fn inspect(
        &self,
        source: &PrivacySource,
        limits: Limits,
        _deadline: u64,
    ) -> Result<NeuralAssessment> {
        match self.mode.load(Ordering::SeqCst) {
            1 => return Err(PRIVATE.into()),
            2 => {
                return NeuralAssessment::failed(source, &self.pins, "worker-timeout", limits)
                    .map_err(|_| "fixture".into())
            }
            _ => {}
        }
        let request = self.pins.request(source, limits).unwrap();
        let ranges = if source.content().is_empty() {
            json!([])
        } else {
            json!([{"start":0,"end":source.content().chars().count()}])
        };
        let findings = if let Some(start) = source.content().find("42") {
            json!([{"start":source.content()[..start].chars().count(),"end":source.content()[..start].chars().count()+2,"label":"account_number"}])
        } else {
            json!([])
        };
        NeuralAssessment::from_value(&json!({"version":"mas-privacy-worker-response/1","source_id":source.id(),"source_token":request["source_token"],"input_bytes":source.content().len(),"detector":self.pins.value(),"offset_unit":"unicode-codepoints","completion":"complete","checked_ranges":ranges,"omissions":[],"findings":findings}),source,&self.pins,limits).map_err(|_|"fixture".into())
    }
}
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for name in ["payload", "controls"] {
        fs::create_dir(f.path.join(name)).unwrap();
        fs::set_permissions(f.path.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    f.config["quarantine"] = json!({"version":VERSION,"payload_file":f.path.join("payload/quarantine.sqlite"),"control_file":f.path.join("controls/keys.sqlite"),"max_cases":32,"retention_ms":300_000});
    f.config["lifecycle_file"] = json!(f.path.join("lifecycle.sqlite"));
    for policy in f.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|p| p["tenant"] == "tenant-a")
    {
        let rules = policy["rules"].as_array_mut().unwrap();
        for (operation, recipient, destination) in [
            (
                "retain-assessment",
                "host-assessment-journal",
                "host-journal",
            ),
            (
                "retain-source",
                "host-source-quarantine",
                "quarantine-payload",
            ),
            (
                "retain-assessment",
                "host-source-quarantine",
                "quarantine-payload",
            ),
            (
                "quarantine-admin",
                "host-source-quarantine",
                "quarantine-control",
            ),
            (
                "hold-source",
                "host-source-quarantine",
                "quarantine-control",
            ),
            (
                "retire-source",
                "host-source-quarantine",
                "quarantine-control",
            ),
            (
                "resolve-source",
                "host-source-quarantine",
                "quarantine-control",
            ),
            (
                "adjudicate-source",
                "host-source-quarantine",
                "quarantine-control",
            ),
            (
                "list-source-quarantine",
                "host-source-quarantine",
                "quarantine-control",
            ),
            ("list-source-quarantine", "principal-a", "quarantine-review"),
            ("resolve-source", "principal-a", "quarantine-review"),
            ("resolve-assessment", "principal-a", "quarantine-review"),
            ("adjudicate-source", "principal-a", "quarantine-review"),
            ("adjudicate-assessment", "principal-a", "quarantine-review"),
        ] {
            rules.push(json!({"operation":operation,"principal":"principal-a","actor":"worker","producer":"host","recipient":recipient,"purpose":"diagnosis","destination":destination,"location":"uk","classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":65536}));
        }
    }
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
    f
}
fn session(f: &Fixture, inspector: Arc<Inspector>) -> GovernedSession {
    f.session()
        .with_privacy_inspector(inspector, "local-detector")
        .unwrap()
}
fn initialize(f: &Fixture, inspector: Arc<Inspector>) -> GovernedSession {
    let session = session(f, inspector);
    initialize_governed_source_quarantine(&session, "worker").unwrap();
    session
}
fn ids(session: &GovernedSession) -> Vec<String> {
    list_governed_source_quarantine(session, "worker").unwrap()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["reference"].as_str().unwrap().to_owned())
        .collect()
}
fn assert_no_plaintext(f: &Fixture) {
    for parent in ["payload", "controls"] {
        for entry in fs::read_dir(f.path.join(parent)).unwrap() {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            assert!(!bytes
                .windows(PRIVATE.len())
                .any(|p| p == PRIVATE.as_bytes()));
        }
    }
}
#[test]
fn rejected_projection_has_encrypted_exact_source_assessment_and_immutable_review() {
    let f = fixture();
    let inspector = Arc::new(Inspector::new());
    let session = initialize(&f, inspector.clone());
    let original = json!({"n":42,"private":PRIVATE});
    assert_eq!(
        session.project_input("worker", &original).unwrap_err(),
        "governed-inspection-prohibited-projection"
    );
    let references = ids(&session);
    assert_eq!(references.len(), 1);
    let view = read_governed_source_quarantine(&session, "worker", &references[0]).unwrap();
    assert_eq!(view["original"], original);
    assert_eq!(view["projection"], json!({"n":42}));
    assert!(
        view["inspection"]["projected_findings_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(!view["inspection"]["component_assessments"]
        .as_array()
        .unwrap()
        .is_empty());
    let review = json!({"review_key":PRIVATE,"verdict":"false-positive","reason":"domain-context"});
    let first =
        adjudicate_governed_source_quarantine(&session, "worker", &references[0], &review).unwrap();
    assert_eq!(
        first,
        adjudicate_governed_source_quarantine(&session, "worker", &references[0], &review).unwrap()
    );
    assert!(adjudicate_governed_source_quarantine(&session,"worker",&references[0],&json!({"review_key":PRIVATE,"verdict":"confirmed-finding","reason":"authorised-source-review"})).is_err());
    // Adjudication does not bypass the original projection gate.
    assert!(session.project_input("worker", &original).is_err());
    assert!(ids(&session).contains(&references[0]));
    let reopened = super::tests::session(&f, inspector);
    assert_eq!(
        view,
        read_governed_source_quarantine(&reopened, "worker", &references[0]).unwrap()
    );
    assert_no_plaintext(&f);
}
#[test]
fn failed_scanner_case_keeps_exact_bytes_and_bounded_failure_without_task_or_dispatch() {
    for mode in [1, 2] {
        let f = fixture();
        let inspector = Arc::new(Inspector::new());
        inspector.mode.store(mode, Ordering::SeqCst);
        let session = initialize(&f, inspector);
        let original = json!({"n":1,"private":PRIVATE});
        assert!(session.project_input("worker", &original).is_err());
        assert!(!f.directory().exists());
        let references = ids(&session);
        assert_eq!(references.len(), 1);
        let view = read_governed_source_quarantine(&session, "worker", &references[0]).unwrap();
        assert_eq!(view["original"], original);
        assert_ne!(view["inspection"]["completion"], "complete");
        assert!(!view["inspection"].to_string().contains(PRIVATE));
        if mode == 2 {
            assert!(!view["inspection"]["component_assessments"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        assert_no_plaintext(&f);
    }
}
#[test]
fn hold_expiry_retirement_and_old_ciphertext_restore_use_independent_current_keys() {
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":1,"private":PRIVATE}))
        .unwrap();
    let id = ids(&session).remove(0);
    let old = fs::read(f.path.join("payload/quarantine.sqlite")).unwrap();
    govern_source_quarantine(&session, "worker", &id, "hold", None).unwrap();
    assert!(govern_source_quarantine(&session, "worker", &id, "retire", None).is_err());
    let expiry = crate::now_ms().unwrap() + 30_000;
    govern_source_quarantine(&session, "worker", &id, "expire", Some(expiry)).unwrap();
    assert_eq!(
        read_governed_source_quarantine(&session, "worker", &id).unwrap()["expires_at_ms"],
        expiry
    );
    assert!(govern_source_quarantine(&session, "worker", &id, "expire", Some(expiry + 1)).is_err());
    govern_source_quarantine(&session, "worker", &id, "expire", Some(1)).unwrap();
    assert!(read_governed_source_quarantine(&session, "worker", &id).is_err());
    govern_source_quarantine(&session, "worker", &id, "release-hold", None).unwrap();
    govern_source_quarantine(&session, "worker", &id, "retire", None).unwrap();
    fs::write(f.path.join("payload/quarantine.sqlite"), old).unwrap();
    assert!(read_governed_source_quarantine(&session, "worker", &id).is_err());
    // A new caller-supplied input is a new assessed event. It cannot revive
    // the old case or read its retired key.
    assert!(read_governed_source_quarantine(&session, "worker", &id).is_err());
    fs::remove_file(f.path.join("controls/keys.sqlite")).unwrap();
    assert!(session.project_input("worker", &json!({"n":2})).is_err());
    assert!(!f.path.join("controls/keys.sqlite").exists());
    assert!(initialize_governed_source_quarantine(&session, "worker").is_err());
}
#[test]
fn inherited_source_policy_and_current_grant_are_required_for_raw_review() {
    let mut f = fixture();
    let source = f.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["id"] == "policy-source-tenant-a")
        .unwrap();
    source["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["operation"] != "resolve-source");
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
    let session = initialize(&f, Arc::new(Inspector::new()));
    let source = Source::create(
        &session,
        "source-test",
        "effect-output",
        "worker",
        &json!({"n":1}),
        &Default::default(),
        &[
            "policy-base-tenant-a".to_owned(),
            "policy-source-tenant-a".to_owned(),
        ]
        .into_iter()
        .collect(),
        u64::MAX,
        true,
    )
    .unwrap();
    let exposure = Exposure::default().with_source(source).unwrap();
    session
        .inspect_boundary_with_exposure(
            "worker",
            &json!({"n":1,"private":PRIVATE}),
            &json!({"n":1}),
            &exposure,
        )
        .unwrap();
    let reference = ids(&session).remove(0);
    assert!(read_governed_source_quarantine(&session, "worker", &reference).is_err());
    let mut revoked = f.authority.clone();
    revoked["grants"][0]["status"] = json!("revoked");
    fs::write(f.path.join("authority.json"), revoked.to_string()).unwrap();
    assert!(list_governed_source_quarantine(&session, "worker").is_err());
    assert!(read_governed_source_quarantine(&session, "worker", &reference).is_err());
}
#[test]
fn namespace_retirement_retires_bound_custody_and_preserves_unadmitted_cases() {
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":2,"private":PRIVATE}))
        .unwrap();
    let unadmitted = ids(&session).remove(0);
    let engine = f.engine(session.clone());
    f.invocation(&engine);
    drop(engine);
    let references = ids(&session);
    assert!(references.len() > 1);
    let bound = references.iter().find(|id| **id != unadmitted).unwrap();
    govern_source_quarantine(&session, "worker", bound, "hold", None).unwrap();
    assert!(
        super::super::govern_namespace_lifecycle(&f.directory(), &session, "retire", None).is_err()
    );
    govern_source_quarantine(&session, "worker", bound, "release-hold", None).unwrap();
    super::super::govern_namespace_lifecycle(&f.directory(), &session, "retire", None).unwrap();
    assert_eq!(ids(&session), vec![unadmitted.clone()]);
    assert!(read_governed_source_quarantine(&session, "worker", bound).is_err());
    assert!(read_governed_source_quarantine(&session, "worker", &unadmitted).is_ok());
    assert!(crate::Engine::open_governed(&f.bundle, &f.path, "exposure-run", session).is_err());
}
#[test]
fn authenticated_encryption_rejects_modified_ciphertext_even_with_rebound_envelope() {
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":1,"private":PRIVATE}))
        .unwrap();
    let id = ids(&session).remove(0);
    let payload = open(&f.path.join("payload/quarantine.sqlite"), true).unwrap();
    let mut body = case_body(&payload, &id).unwrap();
    let mut bytes = body["ciphertext"].as_str().unwrap().as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
    body["ciphertext"] = json!(String::from_utf8(bytes).unwrap());
    payload
        .execute(
            "UPDATE quarantine_cases SET body=? WHERE id=?",
            params![body.to_string(), id],
        )
        .unwrap();
    open(&f.path.join("controls/keys.sqlite"), true)
        .unwrap()
        .execute(
            "UPDATE quarantine_keys SET binding=? WHERE id=?",
            params![session.commitment(&body).unwrap(), id],
        )
        .unwrap();
    assert!(read_governed_source_quarantine(&session, "worker", &id).is_err());
}
#[test]
fn fresh_store_requires_explicit_private_independent_control_and_capacity_is_bounded() {
    let mut f = fixture();
    f.config["quarantine"]["max_cases"] = json!(1);
    let session = initialize(&f, Arc::new(Inspector::new()));
    session.project_input("worker", &json!({"n":1})).unwrap();
    assert!(session.project_input("worker", &json!({"n":2})).is_err());
    assert_eq!(ids(&session).len(), 1);
    fs::set_permissions(
        f.path.join("controls/keys.sqlite"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(list_governed_source_quarantine(&session, "worker").is_err());
}

#[test]
fn completed_effect_source_reads_accept_custody_receipts_and_case_expiry_blocks_derived_views() {
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":3,"private":PRIVATE}))
        .unwrap();
    let unadmitted = ids(&session).remove(0);
    let engine = f.engine(session.clone());
    let invocation = f.invocation(&engine);
    let effect = f.complete(&engine, &invocation, "source", "fixture-read/1");
    drop(engine);
    let source =
        super::super::read_governed_root_source(&f.directory(), &session, "worker", "source")
            .unwrap();
    assert_eq!(source["effect_id"], effect);
    assert_eq!(source["projection"], json!({"n":2}));
    drop(f.engine(session.clone()));
    let bound = ids(&session)
        .into_iter()
        .find(|id| *id != unadmitted)
        .unwrap();
    govern_source_quarantine(&session, "worker", &bound, "expire", Some(1)).unwrap();
    assert!(
        super::super::read_governed_root_source(&f.directory(), &session, "worker", "source")
            .is_err()
    );
    assert!(
        crate::Engine::open_governed(&f.bundle, &f.path, "exposure-run", session.clone()).is_err()
    );
    assert!(read_governed_source_quarantine(&session, "worker", &bound).is_err());
    assert_eq!(ids(&session), vec![unadmitted]);
    let db = Connection::open(f.directory().join("execution.sqlite")).unwrap();
    let state: String = db
        .query_row("SELECT state FROM steps WHERE id=?", [effect], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "completed");
}

#[test]
fn raw_read_admission_finishes_before_concurrent_retirement_commits() {
    use std::sync::{mpsc, Mutex};
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":1,"private":PRIVATE}))
        .unwrap();
    let id = ids(&session).remove(0);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let released = Arc::new(Mutex::new(release_rx));
    let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let path = f.path.join("controls/keys.sqlite");
    let reader = session.clone().with_authentication_check(Arc::new(move || {
        let mut db = open(&path, true)?;
        db.busy_timeout(Duration::ZERO).map_err(|_| UNAVAILABLE)?;
        let busy = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .is_err();
        if busy && !paused.swap(true, Ordering::SeqCst) {
            entered_tx.send(()).map_err(|_| UNAVAILABLE)?;
            released
                .lock()
                .map_err(|_| UNAVAILABLE)?
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| UNAVAILABLE)?;
        }
        Ok(())
    }));
    let read_id = id.clone();
    let read =
        std::thread::spawn(move || read_governed_source_quarantine(&reader, "worker", &read_id));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let retire_session = session.clone();
    let retire_id = id.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let retire = std::thread::spawn(move || {
        let result =
            govern_source_quarantine(&retire_session, "worker", &retire_id, "retire", None);
        done_tx.send(()).unwrap();
        result
    });
    assert!(done_rx.recv_timeout(Duration::from_millis(30)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(
        read.join().unwrap().unwrap()["original"]["private"],
        PRIVATE
    );
    retire.join().unwrap().unwrap();
    assert!(read_governed_source_quarantine(&session, "worker", &id).is_err());
}

#[test]
fn discovery_pages_reconstruct_more_than_sixty_four_immutable_cases() {
    let mut f = fixture();
    f.config["quarantine"]["max_cases"] = json!(65);
    let session = initialize(&f, Arc::new(Inspector::new()));
    for n in 0..65 {
        session
            .retain_quarantine(
                "worker",
                &json!({"n":n}),
                &json!({"n":n}),
                &json!({"version":"fixture-assessment/1"}),
                &Exposure::default(),
            )
            .unwrap();
    }
    let first = list_governed_source_quarantine(&session, "worker").unwrap();
    assert_eq!(first["cases"].as_array().unwrap().len(), 64);
    assert_eq!(first["complete"], false);
    let next = first["next_after"].as_str().unwrap();
    let second = list_governed_source_quarantine_page(&session, "worker", Some(next)).unwrap();
    assert_eq!(second["cases"].as_array().unwrap().len(), 1);
    assert_eq!(second["complete"], true);
    assert!(first["cases"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["reference"] != second["cases"][0]["reference"]));
}

#[test]
fn review_ciphertext_cannot_outlive_newly_shortened_retention_authority() {
    let f = fixture();
    let session = initialize(&f, Arc::new(Inspector::new()));
    session
        .project_input("worker", &json!({"n":1,"private":PRIVATE}))
        .unwrap();
    let id = ids(&session).remove(0);
    let mut shortened = f.authority.clone();
    shortened["grants"][0]["expires_at_ms"] = json!(crate::now_ms().unwrap() + 30_000);
    fs::write(f.path.join("authority.json"), shortened.to_string()).unwrap();
    assert!(adjudicate_governed_source_quarantine(&session,"worker",&id,&json!({"review_key":"shorter-authority","verdict":"inconclusive","reason":"insufficient-evidence"})).is_err());
    let db = open(&f.path.join("controls/keys.sqlite"), false).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM quarantine_review_keys", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let db = open(&f.path.join("payload/quarantine.sqlite"), false).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM quarantine_reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
