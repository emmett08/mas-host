//! Exercise source views through real retained effect records and read-only snapshots.
use super::{exposure_adversarial_tests::Fixture, read_governed_root_source};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

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
fn exact_source_projection_has_closed_shape_and_read_only_restart_identity() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let effect_id = fixture.complete(&engine, &parent, "acquire", "fixture-read/1");
    let view =
        read_governed_root_source(&fixture.directory(), &session, "worker", "acquire").unwrap();
    assert_eq!(view.as_object().unwrap().len(), 15);
    assert_eq!(view["version"], "mas-governed-source-view/1");
    assert_eq!(view["effect_id"], effect_id);
    assert_eq!(view["parent_invocation"], parent.id);
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "available");
    assert_eq!(view["projection"], json!({"n":2}));
    assert_eq!(view["raw_source_disposition"], "not-retained");
    assert_eq!(view["source_metadata"]["id"], view["source_id"]);
    assert_eq!(view["source_metadata"]["step_id"], effect_id);
    assert_eq!(view["source_metadata"]["kind"], "effect-output");
    assert!(view["lineage"]
        .as_array()
        .unwrap()
        .contains(&view["source_metadata"]));
    drop(engine);
    let before = retained_files(&fixture.directory());
    assert_eq!(
        read_governed_root_source(
            &fixture.directory(),
            &fixture.session(),
            "worker",
            "acquire"
        )
        .unwrap(),
        view
    );
    assert_eq!(retained_files(&fixture.directory()), before);
}

#[test]
fn absent_and_unfinished_source_views_do_not_infer_non_execution() {
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    let absent =
        read_governed_root_source(&fixture.directory(), &session, "worker", "missing").unwrap();
    assert_eq!(absent["state"], "not-recorded");
    assert_eq!(absent["external_execution"], "unknown");
    assert_eq!(absent["payload_disposition"], "unavailable");
    for field in ["effect_id", "source_id", "source_metadata", "projection"] {
        assert!(absent[field].is_null());
    }
    engine
        .inner
        .begin_effect(
            &parent,
            "acquire",
            &json!({"format":"fixture-read/1","n":1}),
        )
        .unwrap();
    assert_eq!(
        read_governed_root_source(&fixture.directory(), &session, "worker", "acquire").unwrap_err(),
        "governed-evidence-unavailable"
    );
}

#[test]
fn completed_withheld_source_preserves_certainty_without_returning_original() {
    const PRIVATE: &str = "source-private-person@example.invalid";
    let fixture = Fixture::new();
    let session = fixture.session();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    engine
        .inner
        .begin_effect(
            &parent,
            "acquire",
            &json!({"format":"fixture-read/1","n":1}),
        )
        .unwrap();
    engine
        .inner
        .complete_effect(
            &parent,
            "acquire",
            &json!({"n":PRIVATE}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    let view =
        read_governed_root_source(&fixture.directory(), &session, "worker", "acquire").unwrap();
    assert_eq!(view["host_settlement"], "completed");
    assert_eq!(view["external_execution"], "completed");
    assert_eq!(view["payload_disposition"], "withheld");
    assert_eq!(view["source_metadata"]["payload_available"], false);
    assert_eq!(view["raw_source_disposition"], "not-retained");
    assert!(view["projection"].is_null());
    assert!(!view.to_string().contains(PRIVATE));
    drop(engine);
    for bytes in retained_files(&fixture.directory()).values() {
        assert!(!bytes
            .windows(PRIVATE.len())
            .any(|window| window == PRIVATE.as_bytes()));
    }
}

struct GrowingInspector {
    pins: mas_host_privacy::DetectorPins,
    armed: std::sync::atomic::AtomicBool,
    acquired: std::sync::atomic::AtomicBool,
    context: std::sync::Mutex<Option<(std::sync::Weak<crate::Inner>, crate::Step)>>,
}

impl super::PrivacyInspector for GrowingInspector {
    fn detector(&self) -> &mas_host_privacy::DetectorPins {
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
        source: &mas_host_privacy::Source,
        limits: mas_host_privacy::Limits,
        _deadline: u64,
    ) -> Result<mas_host_privacy::NeuralAssessment, String> {
        use std::sync::atomic::Ordering;
        if self.armed.swap(false, Ordering::SeqCst) {
            let (inner, parent) = self.context.lock().unwrap().as_ref().cloned().unwrap();
            let inner = inner.upgrade().unwrap();
            inner.begin_effect(
                &parent,
                "acquired-during-delivery",
                &json!({"format":"fixture-read/1","n":1}),
            )?;
            inner.complete_effect(
                &parent,
                "acquired-during-delivery",
                &json!({"n":2}),
                &json!({"execution":"completed"}),
            )?;
            self.acquired.store(true, Ordering::SeqCst);
        }
        let request = self.pins.request(source, limits).unwrap();
        let ranges = if source.content().is_empty() {
            json!([])
        } else {
            json!([{"start":0,"end":source.content().chars().count()}])
        };
        mas_host_privacy::NeuralAssessment::from_value(
            &json!({
            "version":"mas-privacy-worker-response/1","source_id":source.id(),
            "source_token":request["source_token"],"input_bytes":source.content().len(),
            "detector":self.pins.value(),"offset_unit":"unicode-codepoints","completion":"complete",
            "checked_ranges":ranges,"omissions":[],"findings":[]}),
            source,
            &self.pins,
            limits,
        )
        .map_err(|_| "fixture-inspection-invalid".into())
    }
}

#[test]
fn root_delivery_reloads_exposure_and_rebinds_payload_beyond_small_status_marker() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    let fixture = Fixture::new();
    let mut authority = fixture.authority.clone();
    for policy in authority["policies"].as_array_mut().unwrap() {
        if policy["id"] == "policy-source-tenant-a" {
            for rule in policy["rules"].as_array_mut().unwrap() {
                if rule["operation"] == "deliver" && rule["destination"] == "client" {
                    rule["max_bytes"] = json!(4);
                }
            }
        }
    }
    fixture.write_authority(&authority);
    let inspector = Arc::new(GrowingInspector {
        pins: mas_host_privacy::DetectorPins::new(
            &"a".repeat(64),
            &"b".repeat(40),
            &"c".repeat(40),
        )
        .unwrap(),
        armed: AtomicBool::new(false),
        acquired: AtomicBool::new(false),
        context: Mutex::new(None),
    });
    let session = fixture
        .session()
        .with_privacy_inspector(inspector.clone(), "local-detector")
        .unwrap();
    let engine = fixture.engine(session.clone());
    let parent = fixture.invocation(&engine);
    *inspector.context.lock().unwrap() = Some((Arc::downgrade(&engine.inner), parent));
    inspector.armed.store(true, Ordering::SeqCst);
    assert_eq!(
        super::read_governed_root_delivery(
            &fixture.directory(),
            &session,
            "worker",
            Some(&json!({"n":1})),
            &json!({})
        )
        .unwrap_err(),
        "governed-evidence-unavailable"
    );
    assert!(inspector.acquired.load(Ordering::SeqCst));
}
