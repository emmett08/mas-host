use mas_host_privacy::{
    decide, merge_findings, scan_synthetic, Assessment, ByteRange, Category, Completion,
    Disposition, Error, ExecutionCertainty, Finding, HandlingContext, HandlingProfile, Limits,
    NotificationRoute, Reason, Source,
};
use std::collections::BTreeSet;

fn context() -> HandlingContext {
    HandlingContext {
        profile: HandlingProfile::Protected,
        policy_available: true,
        authorised: true,
        findings_permitted: false,
        suspected_exposure: false,
        execution: ExecutionCertainty::NotSent,
    }
}

fn complete(source: &Source) -> Assessment {
    let checks = if source.content().is_empty() {
        vec![]
    } else {
        vec![ByteRange::new(source, 0, source.content().len()).unwrap()]
    };
    Assessment::new(
        source,
        Completion::Complete,
        vec![],
        checks,
        BTreeSet::new(),
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn fixture_taxonomy_roundtrips_its_categories_and_rejects_neural_only_categories() {
    let source = Source::text("source-taxonomy", "x", 100).unwrap();
    let limits = Limits::default();
    let range = ByteRange::new(&source, 0, 1).unwrap();
    for category in [
        Category::Person,
        Category::Location,
        Category::Organisation,
        Category::EmailAddress,
        Category::PhoneNumber,
        Category::AccountIdentifier,
        Category::Credential,
        Category::SensitiveContext,
    ] {
        let assessment = Assessment::new(
            &source,
            Completion::Complete,
            vec![Finding::new(range, category, "fixture/1").unwrap()],
            vec![range],
            BTreeSet::new(),
            limits,
        )
        .unwrap();
        assert!(Assessment::from_value(&assessment.metadata(), &source, limits).is_ok());
    }
    for category in [Category::PostalAddress, Category::Url, Category::Date] {
        assert_eq!(
            Assessment::new(
                &source,
                Completion::Complete,
                vec![Finding::new(range, category, "fixture/1").unwrap()],
                vec![range],
                BTreeSet::new(),
                limits,
            )
            .unwrap_err(),
            Error::InvalidRecord
        );
    }
}

#[test]
fn deterministic_full_range_still_has_unqualified_neural_coverage() {
    let source = Source::text("source-1", "A harmless synthetic fixture", 100).unwrap();
    let assessment = scan_synthetic(&source, Limits::default()).unwrap();
    assert_eq!(
        assessment.checked_ranges(),
        &[ByteRange {
            start: 0,
            end: source.content().len()
        }]
    );
    assert_eq!(assessment.completion(), Completion::Partial);
    assert!(assessment.omissions().contains("neural-unqualified"));
    let decision = decide(&assessment, &source, context(), Limits::default()).unwrap();
    assert_eq!(decision.disposition, Disposition::Hold);
    assert_eq!(decision.reason, Reason::IncompleteInspection);
}

#[test]
fn no_record_can_claim_qualified_neural_by_changing_detector_fields() {
    let source = Source::text("source-2", "fixture", 100).unwrap();
    let assessment = complete(&source);
    let mut metadata = assessment.metadata();
    metadata["detector"]["neural_qualified"] = true.into();
    assert_eq!(
        Assessment::from_value(&metadata, &source, Limits::default()).unwrap_err(),
        Error::DetectorMismatch
    );
    assert_eq!(
        decide(&assessment, &source, context(), Limits::default())
            .unwrap()
            .reason,
        Reason::UnqualifiedProtectedProfile
    );
}

#[test]
fn positive_findings_survive_partial_inspection_and_route_by_policy() {
    let source = Source::text(
        "source-3",
        "mail fixture@example.test or synthetic_secret_123",
        100,
    )
    .unwrap();
    let assessment = scan_synthetic(&source, Limits::default()).unwrap();
    assert_eq!(assessment.findings().len(), 2);
    assert_eq!(assessment.completion(), Completion::Partial);
    let decision = decide(&assessment, &source, context(), Limits::default()).unwrap();
    assert_eq!(decision.disposition, Disposition::Withhold);
    assert_eq!(decision.route, Some(NotificationRoute::SecurityResponse));
}

#[test]
fn utf8_byte_boundaries_include_combining_text() {
    let source = Source::text("source-4", "é e\u{301} fixture@example.test", 100).unwrap();
    assert_eq!(ByteRange::new(&source, 1, 2), Err(Error::InvalidRange));
    assert_eq!(ByteRange::new(&source, 3, 5), Err(Error::InvalidRange));
    let assessment = scan_synthetic(&source, Limits::default()).unwrap();
    let finding = &assessment.findings()[0];
    assert_eq!(
        &source.content()[finding.range.start..finding.range.end],
        "fixture@example.test"
    );
}

#[test]
fn byte_ranges_reject_reversed_out_of_bounds_and_empty() {
    let source = Source::text("source-5", "abc", 100).unwrap();
    for (start, end) in [(2, 1), (0, 4), (2, 2), (usize::MAX, usize::MAX)] {
        assert_eq!(
            ByteRange::new(&source, start, end),
            Err(Error::InvalidRange)
        );
    }
}

#[test]
fn complete_coverage_rejects_gaps_missing_final_chunk_and_reordering() {
    let source = Source::text("source-6", "0123456789", 100).unwrap();
    for ranges in [vec![(0, 4), (5, 10)], vec![(0, 9)], vec![(5, 10), (0, 5)]] {
        let ranges = ranges
            .into_iter()
            .map(|(start, end)| ByteRange::new(&source, start, end).unwrap())
            .collect();
        assert!(Assessment::new(
            &source,
            Completion::Complete,
            vec![],
            ranges,
            BTreeSet::new(),
            Limits::default()
        )
        .is_err());
    }
}

#[test]
fn positive_finding_must_be_inside_checked_coverage() {
    let source = Source::text("source-7", "0123456789", 100).unwrap();
    let finding = Finding::new(
        ByteRange::new(&source, 5, 9).unwrap(),
        Category::Credential,
        "fixture/1",
    )
    .unwrap();
    assert!(Assessment::new(
        &source,
        Completion::Partial,
        vec![finding],
        vec![ByteRange::new(&source, 0, 5).unwrap()],
        BTreeSet::new(),
        Limits::default()
    )
    .is_err());
}

#[test]
fn overlap_union_preserves_categories_and_provenance() {
    let source = Source::text("source-8", "0123456789", 100).unwrap();
    let findings = vec![
        Finding::new(
            ByteRange::new(&source, 2, 7).unwrap(),
            Category::Credential,
            "recogniser-b/1",
        )
        .unwrap(),
        Finding::new(
            ByteRange::new(&source, 0, 4).unwrap(),
            Category::AccountIdentifier,
            "recogniser-a/1",
        )
        .unwrap(),
        Finding::new(
            ByteRange::new(&source, 7, 10).unwrap(),
            Category::Person,
            "recogniser-c/1",
        )
        .unwrap(),
    ];
    let merged = merge_findings(&source, findings, Limits::default()).unwrap();
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].range, ByteRange { start: 0, end: 7 });
    assert_eq!(merged[0].categories.len(), 2);
    assert_eq!(merged[0].recognisers.len(), 2);
}

#[test]
fn input_and_finding_limits_fail_explicitly() {
    assert_eq!(
        Source::text("source-9", "12345", 4).unwrap_err(),
        Error::InputTooLarge
    );
    let source = Source::text("source-10", "a@example.test b@example.test", 100).unwrap();
    let limits = Limits {
        max_findings: 1,
        ..Limits::default()
    };
    assert_eq!(
        scan_synthetic(&source, limits).unwrap_err(),
        Error::LimitExceeded
    );
}

#[test]
fn assessment_rejects_same_id_equal_length_replacement_before_and_after_wire_roundtrip() {
    let original = Source::text("source-11", "aaaa", 100).unwrap();
    let changed = Source::text("source-11", "bbbb", 100).unwrap();
    let assessment = complete(&original);
    assert_eq!(
        assessment.validate(&changed, Limits::default()),
        Err(Error::SourceMismatch)
    );
    assert_eq!(
        Assessment::from_value(&assessment.metadata(), &changed, Limits::default()).unwrap_err(),
        Error::SourceMismatch
    );
    Assessment::from_value(&assessment.metadata(), &original.clone(), Limits::default()).unwrap();
}

#[test]
fn strict_records_reject_fields_versions_offsets_and_missing_coverage() {
    let source = Source::text("source-12", "éxxx", 100).unwrap();
    let assessment = complete(&source);
    let mut unknown = assessment.metadata();
    unknown["raw_excerpt"] = "unsafe".into();
    assert_eq!(
        Assessment::from_value(&unknown, &source, Limits::default()).unwrap_err(),
        Error::InvalidRecord
    );
    let mut wrong_version = assessment.metadata();
    wrong_version["version"] = "mas-detection-assessment/2".into();
    assert_eq!(
        Assessment::from_value(&wrong_version, &source, Limits::default()).unwrap_err(),
        Error::UnsupportedVersion
    );
    let mut wrong_offset = assessment.metadata();
    wrong_offset["checked_ranges"][0]["start"] = 1.into();
    assert_eq!(
        Assessment::from_value(&wrong_offset, &source, Limits::default()).unwrap_err(),
        Error::InvalidRange
    );
    let mut missing_tail = assessment.metadata();
    missing_tail["checked_ranges"][0]["end"] = 4.into();
    assert_eq!(
        Assessment::from_value(&missing_tail, &source, Limits::default()).unwrap_err(),
        Error::IncompleteCoverage
    );
}

#[test]
fn known_completion_and_uncertainty_are_preserved_when_output_is_withheld() {
    let source = Source::text("source-13", "synthetic_secret_123", 100).unwrap();
    let assessment = scan_synthetic(&source, Limits::default()).unwrap();
    for execution in [
        ExecutionCertainty::KnownCompleted,
        ExecutionCertainty::Unknown,
    ] {
        let decision = decide(
            &assessment,
            &source,
            HandlingContext {
                execution,
                ..context()
            },
            Limits::default(),
        )
        .unwrap();
        assert_eq!(decision.execution, execution);
        assert_eq!(decision.disposition, Disposition::Withhold);
    }
}

#[test]
fn authority_and_policy_failures_never_permit_synthetic_data() {
    let source = Source::text("source-14", "fixture", 100).unwrap();
    let assessment = complete(&source);
    for context in [
        HandlingContext {
            authorised: false,
            ..context()
        },
        HandlingContext {
            policy_available: false,
            ..context()
        },
    ] {
        assert_ne!(
            decide(&assessment, &source, context, Limits::default())
                .unwrap()
                .disposition,
            Disposition::PermitSynthetic
        );
    }
}

#[test]
fn only_explicit_synthetic_fixture_profile_can_permit_complete_fixture() {
    let source = Source::text("source-15", "fixture", 100).unwrap();
    let assessment = complete(&source);
    let decision = decide(
        &assessment,
        &source,
        HandlingContext {
            profile: HandlingProfile::SyntheticFixture,
            ..context()
        },
        Limits::default(),
    )
    .unwrap();
    assert_eq!(decision.disposition, Disposition::PermitSynthetic);
    assert_eq!(decision.route, None);
}

#[test]
fn unsupported_json_and_attachments_have_no_guessed_mapping() {
    for format in ["json-fields/1", "pdf/1", "email/1"] {
        assert_eq!(
            Source::from_format("source-16", r#"{"text":"\u00e9"}"#, format, 100).unwrap_err(),
            Error::UnsupportedFormat
        );
    }
}

#[test]
fn safe_metadata_and_debug_contain_no_detected_values_or_fingerprints() {
    let marker = "synthetic_secret_do_not_emit";
    let source = Source::text("source-17", marker, 100).unwrap();
    let assessment = scan_synthetic(&source, Limits::default()).unwrap();
    let diagnostics = format!("{source:?} {assessment:?} {}", assessment.metadata());
    assert!(!diagnostics.contains(marker));
    assert!(!diagnostics.contains("fingerprint"));
}

#[test]
fn empty_source_can_be_completely_checked_without_a_fake_range() {
    let source = Source::text("source-18", "", 100).unwrap();
    let assessment = complete(&source);
    assert!(assessment.checked_ranges().is_empty());
    assert_eq!(assessment.completion(), Completion::Complete);
}

#[test]
fn complete_records_cannot_hide_omissions_or_overlap_rules() {
    let source = Source::text("source-19", "0123456789", 100).unwrap();
    let checked = vec![ByteRange::new(&source, 0, 10).unwrap()];
    assert_eq!(
        Assessment::new(
            &source,
            Completion::Complete,
            vec![],
            checked.clone(),
            BTreeSet::from(["missing-chunk".into()]),
            Limits::default(),
        )
        .unwrap_err(),
        Error::IncompleteCoverage,
    );
    let findings = vec![
        Finding::new(
            ByteRange::new(&source, 0, 5).unwrap(),
            Category::Person,
            "fixture/1",
        )
        .unwrap(),
        Finding::new(
            ByteRange::new(&source, 4, 9).unwrap(),
            Category::Person,
            "fixture/1",
        )
        .unwrap(),
    ];
    assert_eq!(
        Assessment::new(
            &source,
            Completion::Partial,
            findings,
            checked,
            BTreeSet::new(),
            Limits::default()
        )
        .unwrap_err(),
        Error::InvalidRecord,
    );
}

#[test]
fn public_merger_validates_directly_constructed_recogniser_sets() {
    let source = Source::text("source-20", "fixture", 100).unwrap();
    let mut finding = Finding::new(
        ByteRange::new(&source, 0, 3).unwrap(),
        Category::Person,
        "fixture/1",
    )
    .unwrap();
    finding.recognisers.insert("raw value!".into());
    assert_eq!(
        merge_findings(&source, vec![finding], Limits::default()),
        Err(Error::InvalidIdentifier)
    );
}
