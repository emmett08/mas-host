use mas_host_privacy::{DetectorPins, Error, Limits, NeuralAssessment, Source};
use serde_json::{json, Value};

fn inputs() -> (Source, DetectorPins, Value) {
    let source = Source::text("fixture-omissions", "fixture", 100).unwrap();
    let pins = DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap();
    let request = pins.request(&source, Limits::default()).unwrap();
    let response = json!({"version":"mas-privacy-worker-response/1","source_id":request["source_id"],
        "source_token":request["source_token"],"input_bytes":request["input_bytes"],"detector":request["detector"],
        "offset_unit":"unicode-codepoints","completion":"failed","checked_ranges":[],"findings":[],"omissions":["model-unavailable"]});
    (source, pins, response)
}

#[test]
fn incomplete_statuses_require_a_fixed_content_free_worker_reason() {
    let (source, pins, mut value) = inputs();
    NeuralAssessment::from_value(&value, &source, &pins, Limits::default()).unwrap();
    for omissions in [
        json!([]),
        json!(["Alice-Example"]),
        json!(["worker-unavailable"]),
    ] {
        value["omissions"] = omissions;
        assert_eq!(
            NeuralAssessment::from_value(&value, &source, &pins, Limits::default()).unwrap_err(),
            Error::InvalidRecord
        );
    }
}

#[test]
fn host_failure_reasons_cannot_copy_arbitrary_exception_data() {
    let (source, pins, _) = inputs();
    NeuralAssessment::failed(&source, &pins, "invalid-worker-response", Limits::default()).unwrap();
    assert_eq!(
        NeuralAssessment::failed(&source, &pins, "Alice-Example", Limits::default()).unwrap_err(),
        Error::InvalidRecord
    );
}
