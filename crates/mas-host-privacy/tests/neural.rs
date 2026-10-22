use mas_host_privacy::{
    Category, CombinedAssessment, Completion, DetectorPins, Disposition, Error, ExecutionCertainty,
    HandlingContext, HandlingProfile, Limits, NeuralAssessment, OpenAiLabel, Reason, Source,
};
use serde_json::{json, Value};

fn pins() -> DetectorPins {
    DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap()
}

fn response(source: &Source) -> Value {
    let request = pins().request(source, Limits::default()).unwrap();
    let checked = if source.content().is_empty() {
        vec![]
    } else {
        vec![json!({"start":0,"end":source.content().chars().count()})]
    };
    json!({"version":"mas-privacy-worker-response/1","source_id":request["source_id"],
        "source_token":request["source_token"],"input_bytes":request["input_bytes"],"detector":request["detector"],
        "offset_unit":"unicode-codepoints","completion":"complete","checked_ranges":checked,
        "omissions":[],"findings":[]})
}

fn context() -> HandlingContext {
    HandlingContext {
        profile: HandlingProfile::SyntheticFixture,
        policy_available: true,
        authorised: true,
        findings_permitted: false,
        suspected_exposure: false,
        execution: ExecutionCertainty::KnownCompleted,
    }
}

#[test]
fn request_binds_exact_private_source_and_pins_without_qualification_fields() {
    let source = Source::text("source-r1", "é", 100).unwrap();
    let request = pins().request(&source, Limits::default()).unwrap();
    assert_eq!(request["version"], "mas-privacy-worker-request/1");
    assert_eq!(request["input_bytes"], 2);
    assert_eq!(request["text"], "é");
    assert_eq!(request["source_token"].as_str().unwrap().len(), 64);
    assert_eq!(request["detector"], pins().value());
    assert!(request.get("release_qualified").is_none());
}

#[test]
fn all_eight_labels_retain_original_meaning_and_distinct_categories() {
    let source = Source::text("source-r2", "a b c d e f g h", 100).unwrap();
    let mappings = [
        (
            "account_number",
            OpenAiLabel::AccountNumber,
            Category::AccountIdentifier,
        ),
        (
            "private_address",
            OpenAiLabel::PrivateAddress,
            Category::PostalAddress,
        ),
        (
            "private_email",
            OpenAiLabel::PrivateEmail,
            Category::EmailAddress,
        ),
        (
            "private_person",
            OpenAiLabel::PrivatePerson,
            Category::Person,
        ),
        (
            "private_phone",
            OpenAiLabel::PrivatePhone,
            Category::PhoneNumber,
        ),
        ("private_url", OpenAiLabel::PrivateUrl, Category::Url),
        ("private_date", OpenAiLabel::PrivateDate, Category::Date),
        ("secret", OpenAiLabel::Secret, Category::Credential),
    ];
    let mut value = response(&source);
    value["findings"] = mappings
        .iter()
        .enumerate()
        .map(|(i, (label, _, _))| json!({"start":i*2,"end":i*2+1,"label":label}))
        .collect();
    let neural = NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap();
    for (finding, (_, label, category)) in neural.findings().iter().zip(mappings) {
        assert_eq!(finding.label, label);
        assert_eq!(finding.label.category(), category);
    }
    let combined = CombinedAssessment::new(&source, neural, Limits::default()).unwrap();
    assert_eq!(combined.findings().len(), 8);
    assert_eq!(combined.metadata()["taxonomy"], "mas-privacy-taxonomy/2");
}

#[test]
fn unicode_codepoints_convert_to_original_bytes_without_normalisation() {
    let source = Source::text("source-r3", "é e\u{301}😀x", 100).unwrap();
    let mut value = response(&source);
    value["findings"] = json!([{"start":2,"end":5,"label":"private_person"}]);
    let neural = NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap();
    assert_eq!(neural.findings()[0].range.start, 3);
    assert_eq!(neural.findings()[0].range.end, 10);
    assert_eq!(neural.checked_ranges()[0].end, 11);
    assert_eq!(&source.content()[3..10], "e\u{301}😀");
    assert_eq!(neural.metadata()["offset_unit"], "utf8-bytes");
}

#[test]
fn declared_wrong_units_and_impossible_codepoint_offsets_are_rejected() {
    let source = Source::text("source-r4", "é😀", 100).unwrap();
    let mut value = response(&source);
    value["offset_unit"] = "utf8-bytes".into();
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::InvalidRange
    );
    value["offset_unit"] = "unicode-codepoints".into();
    value["findings"] = json!([{"start":0,"end":6,"label":"private_person"}]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::InvalidRange
    );
}

#[test]
fn source_replacement_and_detector_changes_invalidate_responses() {
    let source = Source::text("source-r5", "aaaa", 100).unwrap();
    let changed = Source::text("source-r5", "bbbb", 100).unwrap();
    let value = response(&source);
    assert_eq!(
        NeuralAssessment::from_value(&value, &changed, &pins(), Limits::default()).unwrap_err(),
        Error::SourceMismatch
    );
    let mut value = response(&source);
    value["detector"]["model_revision"] = "d".repeat(40).into();
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::DetectorMismatch
    );
}

#[test]
fn exact_schema_and_all_output_limits_are_enforced() {
    let source = Source::text("source-r6", "fixture", 100).unwrap();
    for field in ["text", "score", "release_qualified"] {
        let mut value = response(&source);
        value[field] = "not allowed".into();
        assert_eq!(
            NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
            Error::InvalidRecord
        );
    }
    let mut value = response(&source);
    value["findings"] = json!([{"start":0,"end":1,"label":"private_person"}]);
    assert_eq!(
        NeuralAssessment::from_value(
            &value,
            &source,
            &pins(),
            Limits {
                max_findings: 0,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        Error::LimitExceeded
    );
    assert_eq!(
        NeuralAssessment::from_value(
            &response(&source),
            &source,
            &pins(),
            Limits {
                max_record_bytes: 10,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        Error::LimitExceeded
    );
    assert_eq!(
        pins()
            .request(
                &source,
                Limits {
                    max_input_bytes: 1,
                    ..Limits::default()
                }
            )
            .unwrap_err(),
        Error::InputTooLarge
    );
}

#[test]
fn unknown_model_labels_and_model_scores_are_rejected() {
    let source = Source::text("source-r7", "fixture", 100).unwrap();
    let mut value = response(&source);
    value["findings"] = json!([{"start":0,"end":1,"label":"organisation"}]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::InvalidRecord
    );
    value["findings"] = json!([{"start":0,"end":1,"label":"private_person","score":1}]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::InvalidRecord
    );
}

#[test]
fn complete_neural_responses_require_every_final_codepoint_and_no_gaps() {
    let source = Source::text("source-r8", "0123456789", 100).unwrap();
    for ranges in [
        json!([{ "start":0,"end":9 }]),
        json!([{ "start":0,"end":4 },{ "start":5,"end":10 }]),
        json!([]),
    ] {
        let mut value = response(&source);
        value["checked_ranges"] = ranges;
        assert_eq!(
            NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
            Error::IncompleteCoverage
        );
    }
    let mut value = response(&source);
    value["omissions"] = json!(["incomplete-token-coverage"]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::IncompleteCoverage
    );
}

#[test]
fn partial_unsupported_and_failed_coverage_remain_distinct_with_valid_findings() {
    let source = Source::text("source-r9", "0123456789", 100).unwrap();
    for (status, expected) in [
        ("partial", Completion::Partial),
        ("unsupported", Completion::Unsupported),
        ("failed", Completion::Failed),
    ] {
        let mut value = response(&source);
        value["completion"] = status.into();
        value["omissions"] = json!(["incomplete-token-coverage"]);
        value["checked_ranges"] = json!([{"start":0,"end":5}]);
        value["findings"] = json!([{"start":0,"end":2,"label":"secret"}]);
        let neural =
            NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap();
        assert_eq!(neural.completion(), expected);
        assert_eq!(neural.findings().len(), 1);
        let combined = CombinedAssessment::new(&source, neural, Limits::default()).unwrap();
        assert_eq!(combined.completion(), expected);
        let decision = combined
            .decide(&source, context(), Limits::default())
            .unwrap();
        assert_eq!(decision.disposition, Disposition::Withhold);
        assert_eq!(decision.execution, ExecutionCertainty::KnownCompleted);
    }
}

#[test]
fn finding_outside_observed_ranges_and_reordered_ranges_are_rejected() {
    let source = Source::text("source-r10", "0123456789", 100).unwrap();
    let mut value = response(&source);
    value["completion"] = "partial".into();
    value["omissions"] = json!(["incomplete-token-coverage"]);
    value["checked_ranges"] = json!([{"start":0,"end":5}]);
    value["findings"] = json!([{"start":6,"end":8,"label":"secret"}]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::IncompleteCoverage
    );
    value["findings"] = json!([]);
    value["checked_ranges"] = json!([{"start":5,"end":10},{"start":0,"end":5}]);
    assert_eq!(
        NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap_err(),
        Error::InvalidRange
    );
}

#[test]
fn worker_failure_preserves_deterministic_positive_findings_and_safe_metadata() {
    let marker = "synthetic_secret_retained";
    let source = Source::text("source-r11", marker, 100).unwrap();
    let failure =
        NeuralAssessment::failed(&source, &pins(), "worker-timeout", Limits::default()).unwrap();
    let combined = CombinedAssessment::new(&source, failure, Limits::default()).unwrap();
    assert_eq!(combined.completion(), Completion::Failed);
    assert_eq!(combined.findings().len(), 1);
    assert!(combined.findings()[0]
        .categories
        .contains(&Category::Credential));
    assert_eq!(
        combined
            .decide(&source, context(), Limits::default())
            .unwrap()
            .disposition,
        Disposition::Withhold
    );
    assert!(!combined.metadata().to_string().contains(marker));
    assert!(!format!("{combined:?}").contains(marker));
}

#[test]
fn complete_neural_candidate_cannot_unlock_fixture_or_protected_release() {
    let source = Source::text("source-r12", "fixture", 100).unwrap();
    let neural =
        NeuralAssessment::from_value(&response(&source), &source, &pins(), Limits::default())
            .unwrap();
    let combined = CombinedAssessment::new(&source, neural, Limits::default()).unwrap();
    assert_eq!(combined.completion(), Completion::Complete);
    assert_eq!(combined.metadata()["release_qualified"], false);
    let decision = combined
        .decide(&source, context(), Limits::default())
        .unwrap();
    assert_eq!(decision.disposition, Disposition::Hold);
    assert_eq!(decision.reason, Reason::UnqualifiedProtectedProfile);
}

#[test]
fn overlap_merger_keeps_pattern_and_neural_provenance() {
    let source = Source::text("source-r13", "synthetic_secret_123", 100).unwrap();
    let mut value = response(&source);
    value["findings"] = json!([{"start":0,"end":19,"label":"account_number"}]);
    let neural = NeuralAssessment::from_value(&value, &source, &pins(), Limits::default()).unwrap();
    let combined = CombinedAssessment::new(&source, neural, Limits::default()).unwrap();
    assert_eq!(combined.findings().len(), 1);
    assert_eq!(combined.findings()[0].categories.len(), 2);
    assert_eq!(combined.findings()[0].recognisers.len(), 2);
    assert_eq!(
        combined.neural().findings()[0].label,
        OpenAiLabel::AccountNumber
    );
}

#[test]
fn duplicate_detector_contributions_use_the_merged_finding_limit() {
    let limits = Limits::default();
    for count in [64, 65, limits.max_findings] {
        let email = "a@b.invalid";
        let stride = email.len() + 1;
        let text = format!("{email} ").repeat(count);
        let source = Source::text("source-merge-limit", &text, limits.max_input_bytes).unwrap();
        let mut value = response(&source);
        value["findings"] = (0..count)
            .map(|i| json!({"start":i*stride,"end":i*stride+email.len(),"label":"private_email"}))
            .collect();
        let neural = NeuralAssessment::from_value(&value, &source, &pins(), limits).unwrap();
        let combined = CombinedAssessment::new(&source, neural, limits).unwrap();
        assert_eq!(combined.findings().len(), count);
        assert_eq!(combined.completion(), Completion::Complete);
        assert!(combined
            .findings()
            .iter()
            .all(|finding| finding.recognisers.len() == 2));
    }
}

#[test]
fn combined_output_still_rejects_more_than_the_finding_limit() {
    let limits = Limits::default();
    let prefix = "a@b.invalid ";
    let text = prefix.to_owned() + &"x ".repeat(limits.max_findings);
    let source = Source::text("source-output-limit", &text, limits.max_input_bytes).unwrap();
    let mut value = response(&source);
    value["findings"] = (0..limits.max_findings)
        .map(
            |i| json!({"start":prefix.len()+i*2,"end":prefix.len()+i*2+1,"label":"private_person"}),
        )
        .collect();
    let neural = NeuralAssessment::from_value(&value, &source, &pins(), limits).unwrap();
    assert_eq!(
        CombinedAssessment::new(&source, neural, limits).unwrap_err(),
        Error::LimitExceeded
    );
}

#[test]
fn pin_identity_is_closed_and_supports_only_the_implemented_backend() {
    for value in ["A".repeat(64), "a".repeat(63), "a".repeat(65)] {
        assert_eq!(
            DetectorPins::new(&value, &"b".repeat(40), &"c".repeat(40)),
            Err(Error::InvalidIdentifier)
        );
    }
    let mut value = pins().value();
    value["backend"] = "opf-gpu/1".into();
    assert_eq!(
        DetectorPins::from_value(&value),
        Err(Error::DetectorMismatch)
    );
    let mut value = pins().value();
    value["dtype"] = "bfloat16".into();
    assert_eq!(DetectorPins::from_value(&value), Err(Error::InvalidRecord));
}
