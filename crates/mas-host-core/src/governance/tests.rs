use super::*;
use serde_json::{json, Value};

fn fixtures() -> Value {
    serde_json::from_str(include_str!("fixtures/decisions.json")).unwrap()
}

fn apply(value: &mut Value, patches: &Value) {
    for patch in patches.as_array().unwrap() {
        *value.pointer_mut(patch["path"].as_str().unwrap()).unwrap() = patch["value"].clone();
    }
}

fn decide(value: &Value) -> Value {
    decide_disclosure(
        &InvocationScope::from_value(&value["scope"]).unwrap(),
        &DisclosureRequest::from_value(&value["request"]).unwrap(),
        &CurrentAuthority::from_value(&value["authority"]).unwrap(),
        &value["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| GovernedEvidence::from_value(item).unwrap())
            .collect::<Vec<_>>(),
        &value["policies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| DataUsePolicy::from_value(item).unwrap())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn published_policy_counterexamples() {
    let fixtures = fixtures();
    for case in fixtures["cases"].as_array().unwrap() {
        let mut input = fixtures.clone();
        apply(&mut input, &case["patches"]);
        let decision = decide(&input);
        let observed = if decision["disposition"] == "hold" {
            &decision["reason"]
        } else {
            &decision["disposition"]
        };
        assert_eq!(observed, &case["expected"], "case {}", case["name"]);
    }
}

#[test]
fn decisions_bind_exact_operation_payload_and_recipient() {
    let input = fixtures();
    let decision = decide(&input);
    assert_eq!(decision["authority"], "requires-current-admission");
    assert_eq!(decision["binding_complete"], true);
    assert!(decision.to_string().len() <= 65_536);
    assert!(decision["policy_bindings"][0].get("rules").is_none());
    for field in [
        "invocation_id",
        "operation",
        "recipient",
        "destination",
        "location",
        "input_binding_ref",
        "output_binding_ref",
    ] {
        assert_eq!(decision[field], input["request"][field]);
    }
    assert_eq!(
        decision["evidence_bindings"][0]["binding_ref"],
        input["evidence"][0]["binding_ref"]
    );
    assert_eq!(decision["grant_epoch"], 3);
    assert_eq!(decision["max_bytes"], 128);
    assert_eq!(decision["expires_at_ms"], 800);
    let mut changed = input;
    changed["request"]["output_binding_ref"] = json!("different-protected-output");
    assert_ne!(decision, decide(&changed));
    changed = fixtures();
    changed["evidence"][0]["source_ref"] = json!("different-source");
    assert_ne!(decision, decide(&changed));
    assert_eq!(
        GovernedEvidence::from_value(&fixtures()["evidence"][0])
            .unwrap()
            .metadata(),
        fixtures()["evidence"][0]
    );
}

#[test]
fn held_decisions_preserve_distinct_operation_output_and_recipient_bindings() {
    let mut input = fixtures();
    input["authority"] = json!({"version":"mas-current-authority/1", "status":"unavailable"});
    let original = decide(&input);
    assert_eq!(original["disposition"], "hold");
    assert_eq!(original["reason"], "policy-unavailable");
    assert_eq!(original["authority"], "none");
    assert_eq!(original["expires_at_ms"], 0);
    assert_eq!(original["binding_complete"], true);
    assert_eq!(
        original["request_fulfilled_obligations"],
        input["request"]["fulfilled_obligations"]
    );
    for field in ["operation", "output_binding_ref", "recipient"] {
        let mut changed = input.clone();
        changed["request"][field] = json!(format!("different-{field}"));
        let held = decide(&changed);
        assert_eq!(held["disposition"], "hold");
        assert_eq!(held["reason"], "policy-unavailable");
        assert_eq!(held[field], changed["request"][field]);
        assert_ne!(
            held, original,
            "distinct rejected {field} must stay distinct"
        );
    }
    assert_eq!(original["evidence_bindings"], input["evidence"]);
    assert_eq!(
        original["policy_bindings"][0]["id"],
        input["policies"][0]["id"]
    );
    assert_eq!(original["current_authority"]["status"], "unavailable");
}

#[test]
fn mismatched_and_expired_decisions_retain_requested_and_observed_facts() {
    let mut input = fixtures();
    input["request"]["evidence_refs"] = json!([]);
    let decision = decide(&input);
    assert_eq!(decision["reason"], "scope-mismatch");
    assert_eq!(decision["scope_evidence_refs"], json!(["evidence-1"]));
    assert_eq!(decision["requested_evidence_refs"], json!([]));
    assert_eq!(decision["evidence_bindings"][0]["id"], "evidence-1");

    input = fixtures();
    input["request"]["now_ms"] = json!(1000);
    let decision = decide(&input);
    assert_eq!(decision["reason"], "expired-authority");
    assert_eq!(decision["evaluated_at_ms"], 1000);
    assert_eq!(decision["current_authority"]["expires_at_ms"], 1000);
    assert_eq!(decision["requested_expires_at_ms"], 900);
    assert_eq!(decision["expires_at_ms"], 0);
    assert_eq!(decision["authority"], "none");

    input = fixtures();
    input["request"]["output_binding_ref"] = json!({"unexpected":"private diagnostic"});
    assert!(DisclosureRequest::from_value(&input["request"]).is_err());
}

#[test]
fn aggregate_decision_limit_never_permits_or_implies_complete_exposure() {
    let mut input = fixtures();
    let ids = (0..64)
        .map(|index| format!("evidence-{index}"))
        .collect::<Vec<_>>();
    let policy_ids = (0..64)
        .map(|index| format!("policy-{index:02}-{}", "p".repeat(118)))
        .collect::<Vec<_>>();
    input["scope"]["evidence_refs"] = json!(ids);
    input["request"]["evidence_refs"] = json!(ids);
    let evidence = ids
        .iter()
        .map(|id| {
            let mut item = input["evidence"][0].clone();
            item["id"] = json!(id);
            item["policy_refs"] = json!(policy_ids);
            // Each accepted metadata record fits its own limit. Their assembled
            // reference closure exceeds the existing 65536-byte decision envelope.
            assert!(GovernedEvidence::from_value(&item).is_ok());
            item
        })
        .collect::<Vec<_>>();
    let policies = policy_ids
        .iter()
        .map(|id| {
            let mut policy = input["policies"][0].clone();
            policy["id"] = json!(id);
            policy["evidence_refs"] = json!(ids);
            assert!(DataUsePolicy::from_value(&policy).is_ok());
            policy
        })
        .collect::<Vec<_>>();
    input["evidence"] = json!(evidence);
    input["policies"] = json!(policies);
    let decision = decide(&input);
    assert_eq!(decision["disposition"], "hold");
    assert_eq!(decision["reason"], "decision-binding-limit");
    assert_eq!(decision["binding_complete"], false);
    assert_eq!(decision["binding_omission"], "governance-reference-limit");
    assert_eq!(decision["authority"], "none");
    assert_eq!(
        decision["output_binding_ref"],
        input["request"]["output_binding_ref"]
    );
    assert_eq!(decision["recipient"], input["request"]["recipient"]);
    assert!(decision.get("evidence_bindings").is_none());
    assert!(decision.get("policy_refs").is_none());
    assert!(decision.to_string().len() <= 65_536);

    // The same bound applies before an early hold, not only to candidates
    // which otherwise satisfy their policy predicates.
    input["authority"] = json!({"version":"mas-current-authority/1", "status":"revoked"});
    assert_eq!(decide(&input)["reason"], "decision-binding-limit");
    let duplicate = input["evidence"][0].clone();
    input["evidence"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(decide(&input)["binding_complete"], false);
}

#[test]
fn inherited_policy_permissions_intersect_and_obligations_accumulate() {
    let mut input = fixtures();
    let mut second = input["policies"][0].clone();
    second["id"] = json!("parent-policy");
    second["rules"][0]["obligations"] = json!(["extra-restriction"]);
    input["policies"].as_array_mut().unwrap().push(second);
    input["evidence"][0]["policy_refs"] = json!(["source-policy", "parent-policy"]);
    assert_eq!(decide(&input)["reason"], "obligations-unfulfilled");
    input["request"]["fulfilled_obligations"] = json!(["bounded-retention", "extra-restriction"]);
    assert_eq!(decide(&input)["disposition"], "permitted-by-contract");
    input["policies"][1]["rules"][0]["destination"] = json!("different-destination");
    assert_eq!(decide(&input)["reason"], "policy-denied");
}

#[test]
fn protected_result_dimensions_remain_independent() {
    let fixtures = fixtures();
    for case in fixtures["result_cases"].as_array().unwrap() {
        let mut result = fixtures["result"].clone();
        apply(&mut result, &case["patches"]);
        validate_result_view(&result).unwrap();
        assert_eq!(result["host_settlement"], "completed");
        assert_eq!(result["application_assessment"], "not-run");
        assert_eq!(result["cso"]["native"], "NOT_RUN");
        assert_eq!(result["cso"]["authority"], "denied");
    }
    let mut invalid = fixtures["result"].clone();
    invalid["payload"]["reference"] = json!("must-not-deliver");
    assert!(validate_result_view(&invalid).is_err());
}

#[test]
fn deployment_declarations_cannot_enable_unimplemented_enforcement() {
    let mut profile = json!({"version":"mas-host-profile/1", "profile":"trusted-local", "required_capabilities":[]});
    assert_eq!(check_profile(&profile).unwrap()["accepted"], true);
    for name in ["managed-cluster", "shared-enterprise"] {
        profile["profile"] = json!(name);
        assert!(check_profile(&profile)
            .unwrap_err()
            .contains("pre-persistence-handling"));
    }
    profile["required_capabilities"] = json!([
        "authenticated-scope",
        "pre-persistence-handling",
        "current-disclosure-admission",
        "governed-payload-lifecycle",
        "effect-output-handling",
        "shared-authority",
        "fenced-scheduling",
        "shared-quotas"
    ]);
    assert!(check_profile(&profile).is_err());
    profile["profile"] = json!("trusted-local");
    profile["required_capabilities"] = json!([]);
    profile["worker_isolation"] = json!(true);
    assert!(check_profile(&profile)
        .unwrap_err()
        .contains("worker-isolation"));
}

#[test]
fn strict_contracts_reject_unknown_versions_fields_and_identity_wildcards() {
    let input = fixtures();
    let mut scope = input["scope"].clone();
    scope["tenant"] = json!("*");
    assert!(InvocationScope::from_value(&scope).is_err());
    scope = input["scope"].clone();
    scope["version"] = json!("mas-invocation-scope/2");
    assert!(InvocationScope::from_value(&scope).is_err());
    scope = input["scope"].clone();
    scope["untrusted_override"] = json!(true);
    assert!(InvocationScope::from_value(&scope).is_err());
    let mut policy = input["policies"][0].clone();
    policy["rules"][0]["fallback"] = json!(true);
    assert!(DataUsePolicy::from_value(&policy).is_err());
    let mut evidence = input["evidence"][0].clone();
    evidence["parents"] = json!(["evidence-1"]);
    assert!(GovernedEvidence::from_value(&evidence).is_err());
    let profile = json!({"version":"mas-host-profile/1", "profile":"trusted-local", "required_capabilities":["unrecognised"]});
    assert!(check_profile(&profile).is_err());
    assert!(TenantId::new("same-name").is_ok());
    assert!(ActorId::new("same-name").is_ok());
    assert!(DestinationId::new("*").is_err());
}
