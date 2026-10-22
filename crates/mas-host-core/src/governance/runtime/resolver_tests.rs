use super::resolver_record::validate_completion;
use serde_json::{json, Value};

fn available(certainty: &str) -> (Value, Value, Value) {
    let output = json!({"n":1});
    let evidence = json!({"version":"mas-governed-effect-evidence/1","external_execution":certainty,
        "payload_disposition":"available","reported_evidence":{"execution":certainty}});
    let completion = json!({"version":"mas-governed-effect-completion/1","effect_id":"eff-fixture",
        "host_settlement":"completed","external_execution":certainty,"payload_disposition":"available","output":output});
    (output, evidence, completion)
}

#[test]
fn completion_truth_requires_matching_retained_receipt_dimensions() {
    for certainty in ["completed", "unknown", "not-sent", "refused"] {
        let (output, evidence, completion) = available(certainty);
        validate_completion("eff-fixture", &output, &evidence, &completion).unwrap();
        assert_eq!(completion["external_execution"], certainty);
        for field in [
            "effect_id",
            "version",
            "host_settlement",
            "external_execution",
            "payload_disposition",
            "output",
        ] {
            let mut changed = completion.clone();
            changed[field] = json!("changed");
            assert!(
                validate_completion("eff-fixture", &output, &evidence, &changed).is_err(),
                "{field}"
            );
        }
    }
}

#[test]
fn withheld_completion_preserves_exact_host_marker_without_reported_payloads() {
    let output = json!({"version":"mas-governed-unavailable/1","external_execution":"completed","payload_disposition":"withheld"});
    let mut evidence = json!({"version":"mas-governed-effect-evidence/1","external_execution":"completed","payload_disposition":"withheld"});
    let completion = json!({"version":"mas-governed-effect-completion/1","effect_id":"eff-fixture",
        "host_settlement":"completed","external_execution":"completed","payload_disposition":"withheld","output":output});
    validate_completion("eff-fixture", &output, &evidence, &completion).unwrap();
    evidence["reported_evidence"] = json!({"private":"prohibited"});
    assert!(validate_completion("eff-fixture", &output, &evidence, &completion).is_err());
}

#[test]
fn completion_reader_rejects_missing_or_extra_untrusted_payload_fields() {
    let (output, mut evidence, completion) = available("completed");
    evidence["reported_evidence"] = json!({});
    assert!(validate_completion("eff-fixture", &output, &evidence, &completion).is_err());
    let (output, evidence, mut completion) = available("completed");
    completion["unapproved"] = json!({"private":"prohibited"});
    assert!(validate_completion("eff-fixture", &output, &evidence, &completion).is_err());
}
