//! Evaluate correlated policy predicates over an exact proposed disclosure.
use super::identity::evidence_names;
use super::{
    identifier, names, record, unsigned, DestinationId, EvidenceId, GovernedEvidence,
    InvocationScope, PrincipalId, Result,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
struct Grant {
    tenant: String,
    principal: String,
    reference: String,
    epoch: u64,
    policy_version: String,
    valid_from_ms: u64,
    expires_at_ms: u64,
}

/// Current facts supplied by the trusted authority. A parsed record is not an
/// authority lookup, authenticated attestation or revocation subscription.
#[derive(Clone, Debug)]
pub struct CurrentAuthority {
    grant: Option<Grant>,
    failure: &'static str,
}

impl CurrentAuthority {
    pub fn from_value(value: &Value) -> Result<Self> {
        match value["status"].as_str() {
            Some("unavailable" | "revoked") => {
                record(
                    value,
                    "mas-current-authority/1",
                    &["version", "status"],
                    &[],
                )?;
                Ok(Self {
                    grant: None,
                    failure: if value["status"] == "revoked" {
                        "grant-revoked"
                    } else {
                        "policy-unavailable"
                    },
                })
            }
            Some("active") => {
                record(
                    value,
                    "mas-current-authority/1",
                    &[
                        "version",
                        "status",
                        "tenant",
                        "principal",
                        "grant_ref",
                        "grant_epoch",
                        "policy_version",
                        "valid_from_ms",
                        "expires_at_ms",
                    ],
                    &[],
                )?;
                let valid_from_ms = unsigned(value, "valid_from_ms")?;
                let expires_at_ms = unsigned(value, "expires_at_ms")?;
                if expires_at_ms <= valid_from_ms {
                    return Err("authority validity interval must be nonempty".into());
                }
                Ok(Self {
                    grant: Some(Grant {
                        tenant: identifier(value, "tenant")?,
                        principal: identifier(value, "principal")?,
                        reference: identifier(value, "grant_ref")?,
                        epoch: unsigned(value, "grant_epoch")?,
                        policy_version: identifier(value, "policy_version")?,
                        valid_from_ms,
                        expires_at_ms,
                    }),
                    failure: "",
                })
            }
            _ => Err("unsupported current authority status".into()),
        }
    }
}

#[derive(Clone, Debug)]
struct Rule {
    operation: String,
    principal: String,
    actor: String,
    producer: String,
    recipient: String,
    purpose: String,
    destination: String,
    location: String,
    classifications: BTreeSet<String>,
    obligations: BTreeSet<String>,
    max_bytes: u64,
}

impl Rule {
    fn from_value(value: &Value) -> Result<Self> {
        super::super::fields(
            value,
            &[
                "operation",
                "principal",
                "actor",
                "producer",
                "recipient",
                "purpose",
                "destination",
                "location",
                "classifications",
                "obligations",
                "max_bytes",
            ],
            &[],
        )?;
        Ok(Self {
            operation: identifier(value, "operation")?,
            principal: identifier(value, "principal")?,
            actor: identifier(value, "actor")?,
            producer: identifier(value, "producer")?,
            recipient: identifier(value, "recipient")?,
            purpose: identifier(value, "purpose")?,
            destination: identifier(value, "destination")?,
            location: identifier(value, "location")?,
            classifications: names(&value["classifications"])?,
            obligations: names(&value["obligations"])?,
            max_bytes: unsigned(value, "max_bytes")?,
        })
    }

    fn matches(&self, scope: &InvocationScope, request: &DisclosureRequest) -> bool {
        self.operation == request.operation
            && self.principal == scope.principal.as_str()
            && self.actor == scope.actor.as_str()
            && self.producer == scope.producer.as_str()
            && self.recipient == request.recipient.as_str()
            && self.purpose == scope.purpose
            && self.destination == request.destination.as_str()
            && self.location == request.location
            && request.byte_count <= self.max_bytes
    }
}

/// A policy association preserves whole allowed tuples. Different purpose and
/// destination pairs are alternatives, never independent sets to cross-product.
#[derive(Clone, Debug)]
pub struct DataUsePolicy {
    id: String,
    tenant: String,
    policy_version: String,
    evidence_refs: BTreeSet<EvidenceId>,
    valid_from_ms: u64,
    expires_at_ms: u64,
    rules: Vec<Rule>,
}

impl DataUsePolicy {
    pub fn from_value(value: &Value) -> Result<Self> {
        record(
            value,
            "mas-data-use-policy/1",
            &[
                "version",
                "id",
                "tenant",
                "policy_version",
                "evidence_refs",
                "valid_from_ms",
                "expires_at_ms",
                "rules",
            ],
            &[],
        )?;
        let rules = value["rules"]
            .as_array()
            .filter(|rules| !rules.is_empty() && rules.len() <= 64)
            .ok_or("policy requires 1..64 correlated rules")?
            .iter()
            .map(Rule::from_value)
            .collect::<Result<Vec<_>>>()?;
        let valid_from_ms = unsigned(value, "valid_from_ms")?;
        let expires_at_ms = unsigned(value, "expires_at_ms")?;
        if expires_at_ms <= valid_from_ms {
            return Err("policy validity interval must be nonempty".into());
        }
        Ok(Self {
            id: identifier(value, "id")?,
            tenant: identifier(value, "tenant")?,
            policy_version: identifier(value, "policy_version")?,
            evidence_refs: evidence_names(&value["evidence_refs"])?,
            valid_from_ms,
            expires_at_ms,
            rules,
        })
    }
}

/// An exact proposed release assembled by trusted host code. Binding references
/// identify protected commitments to exact bytes, not public hashes of PII.
#[derive(Clone, Debug)]
pub struct DisclosureRequest {
    invocation_id: String,
    operation: String,
    recipient: PrincipalId,
    destination: DestinationId,
    location: String,
    input_binding_ref: String,
    output_binding_ref: String,
    evidence_refs: BTreeSet<EvidenceId>,
    fulfilled_obligations: BTreeSet<String>,
    byte_count: u64,
    now_ms: u64,
    expires_at_ms: u64,
}

impl DisclosureRequest {
    /// `fulfilled_obligations` contains trusted enforcement results, not an
    /// untrusted worker's declaration of intended handling.
    pub fn from_value(value: &Value) -> Result<Self> {
        record(
            value,
            "mas-disclosure-request/1",
            &[
                "version",
                "invocation_id",
                "operation",
                "recipient",
                "destination",
                "location",
                "input_binding_ref",
                "output_binding_ref",
                "evidence_refs",
                "fulfilled_obligations",
                "byte_count",
                "now_ms",
                "expires_at_ms",
            ],
            &[],
        )?;
        Ok(Self {
            invocation_id: identifier(value, "invocation_id")?,
            operation: identifier(value, "operation")?,
            recipient: PrincipalId::new(&identifier(value, "recipient")?)?,
            destination: DestinationId::new(&identifier(value, "destination")?)?,
            location: identifier(value, "location")?,
            input_binding_ref: identifier(value, "input_binding_ref")?,
            output_binding_ref: identifier(value, "output_binding_ref")?,
            evidence_refs: evidence_names(&value["evidence_refs"])?,
            fulfilled_obligations: names(&value["fulfilled_obligations"])?,
            byte_count: unsigned(value, "byte_count")?,
            now_ms: unsigned(value, "now_ms")?,
            expires_at_ms: unsigned(value, "expires_at_ms")?,
        })
    }
}

const MAX_DECISION_BYTES: usize = 65_536;

/// Retain only validated identity/reference fields, never source payloads,
/// credentials, worker diagnostics or policy rule text. These references can
/// themselves be sensitive: every decision remains governed metadata.
fn request_binding(scope: &InvocationScope, request: &DisclosureRequest) -> Value {
    json!({
        "version":"mas-data-use-decision/1", "disposition":"hold",
        "authority":"none", "expires_at_ms":0,
        "invocation_id":request.invocation_id,
        "tenant":scope.tenant.as_str(), "principal":scope.principal.as_str(),
        "actor":scope.actor.as_str(), "producer":scope.producer.as_str(),
        "operation":request.operation, "recipient":request.recipient.as_str(),
        "purpose":scope.purpose, "destination":request.destination.as_str(),
        "location":request.location, "input_binding_ref":request.input_binding_ref,
        "output_binding_ref":request.output_binding_ref,
        "policy_version":scope.policy_version,
        "grant_ref":scope.grant_ref, "grant_epoch":scope.grant_epoch,
        "max_bytes":request.byte_count, "evaluated_at_ms":request.now_ms,
        "requested_expires_at_ms":request.expires_at_ms,
    })
}

fn decision_binding(
    scope: &InvocationScope,
    request: &DisclosureRequest,
    authority: &CurrentAuthority,
    evidence: &[GovernedEvidence],
    policies: &[DataUsePolicy],
) -> Option<Value> {
    // Check cardinality before projecting caller-owned slices. An incomplete
    // projection must never appear to be an empty, completely checked exposure.
    if evidence.len() > 64 || policies.len() > 64 {
        return None;
    }
    let mut binding = request_binding(scope, request);
    binding["binding_complete"] = json!(true);
    binding["scope_permitted_operations"] = json!(scope.permitted_operations);
    binding["scope_evidence_refs"] = json!(scope
        .evidence_refs
        .iter()
        .map(EvidenceId::as_str)
        .collect::<Vec<_>>());
    binding["requested_evidence_refs"] = json!(request
        .evidence_refs
        .iter()
        .map(EvidenceId::as_str)
        .collect::<Vec<_>>());
    binding["request_fulfilled_obligations"] = json!(request.fulfilled_obligations);
    binding["evidence_bindings"] = json!(evidence
        .iter()
        .map(GovernedEvidence::metadata)
        .collect::<Vec<_>>());
    // Preserve duplicate/mismatched supplied references on rejected inputs too.
    // A policy reference/version is resolved by the trusted policy owner; this
    // projection does not establish authenticity or retain executable rules.
    binding["policy_refs"] = json!(policies.iter().map(|policy| &policy.id).collect::<Vec<_>>());
    binding["policy_bindings"] =
        json!(policies.iter().map(|policy| json!({
        "id":policy.id, "tenant":policy.tenant, "policy_version":policy.policy_version,
        "evidence_refs":policy.evidence_refs.iter().map(EvidenceId::as_str).collect::<Vec<_>>(),
        "valid_from_ms":policy.valid_from_ms, "expires_at_ms":policy.expires_at_ms,
    })).collect::<Vec<_>>());
    binding["current_authority"] = match &authority.grant {
        Some(grant) => json!({
            "status":"active", "tenant":grant.tenant, "principal":grant.principal,
            "grant_ref":grant.reference, "grant_epoch":grant.epoch,
            "policy_version":grant.policy_version, "valid_from_ms":grant.valid_from_ms,
            "expires_at_ms":grant.expires_at_ms,
        }),
        None => json!({"status":if authority.failure == "grant-revoked" {
            "revoked"
        } else {
            "unavailable"
        }}),
    };
    // Reserve space for the largest fixed outcome fields written below.
    binding["reason"] = json!("missing-or-excessive-governance");
    binding["fulfilled_obligations"] = json!(request.fulfilled_obligations);
    binding["authority"] = json!("requires-current-admission");
    binding["disposition"] = json!("permitted-by-contract");
    binding["expires_at_ms"] = json!(u64::MAX);
    if binding.to_string().len() > MAX_DECISION_BYTES {
        return None;
    }
    Some(binding)
}

fn held(mut binding: Value, reason: &str) -> Value {
    binding["disposition"] = json!("hold");
    binding["reason"] = json!(reason);
    binding["authority"] = json!("none");
    binding["expires_at_ms"] = json!(0);
    binding["fulfilled_obligations"] = json!([]);
    binding
}

/// Determine a bounded contract decision for the supplied current facts.
/// Every evidence policy is conjoined and all matched obligations accumulate.
/// The returned record grants no dispatch permission and must be revalidated
/// through an authoritative admission when an enforcement path is implemented.
/// Every record fits 65536 bytes. Excess references produce a non-permitting
/// limit decision with explicitly incomplete bindings, never truncated lineage.
pub fn decide_disclosure(
    scope: &InvocationScope,
    request: &DisclosureRequest,
    authority: &CurrentAuthority,
    evidence: &[GovernedEvidence],
    policies: &[DataUsePolicy],
) -> Value {
    let Some(mut binding) = decision_binding(scope, request, authority, evidence, policies) else {
        let mut binding = request_binding(scope, request);
        binding["binding_complete"] = json!(false);
        binding["binding_omission"] = json!("governance-reference-limit");
        return held(binding, "decision-binding-limit");
    };
    let Some(grant) = &authority.grant else {
        return held(binding, authority.failure);
    };
    if grant.tenant != scope.tenant.as_str()
        || grant.principal != scope.principal.as_str()
        || grant.reference != scope.grant_ref
        || grant.epoch != scope.grant_epoch
        || grant.policy_version != scope.policy_version
    {
        return held(binding, "authority-mismatch");
    }
    if request.now_ms < grant.valid_from_ms || request.now_ms >= grant.expires_at_ms {
        return held(binding, "expired-authority");
    }
    if !scope.permitted_operations.contains(&request.operation)
        || scope.evidence_refs != request.evidence_refs
        || request.now_ms >= request.expires_at_ms
    {
        return held(binding, "scope-mismatch");
    }
    if evidence.is_empty() || evidence.len() > 64 || policies.is_empty() || policies.len() > 64 {
        return held(binding, "missing-or-excessive-governance");
    }
    let evidence_ids: BTreeSet<_> = evidence.iter().map(|item| item.id.clone()).collect();
    let policy_ids: BTreeSet<_> = policies.iter().map(|policy| policy.id.clone()).collect();
    if evidence_ids.len() != evidence.len()
        || evidence_ids != scope.evidence_refs
        || policy_ids.len() != policies.len()
    {
        return held(binding, "exposure-mismatch");
    }
    let mut expires_at_ms = request.expires_at_ms.min(grant.expires_at_ms);
    let mut obligations = BTreeSet::new();
    for item in evidence {
        if item.coverage != "complete" || item.classification == "unchecked" {
            return held(binding, "incomplete-evidence");
        }
        if request.now_ms < item.acquired_at_ms || request.now_ms >= item.expires_at_ms {
            return held(binding, "expired-evidence");
        }
        if item.policy_refs.is_empty()
            || !item.policy_refs.is_subset(&policy_ids)
            || !item.parents.is_subset(&evidence_ids)
        {
            return held(binding, "missing-inherited-policy");
        }
        expires_at_ms = expires_at_ms.min(item.expires_at_ms);
        for policy in policies
            .iter()
            .filter(|policy| item.policy_refs.contains(&policy.id))
        {
            if policy.tenant != scope.tenant.as_str()
                || policy.policy_version != scope.policy_version
                || !policy.evidence_refs.contains(&item.id)
            {
                return held(binding, "policy-binding-mismatch");
            }
            if request.now_ms < policy.valid_from_ms || request.now_ms >= policy.expires_at_ms {
                return held(binding, "expired-policy");
            }
            let matches: Vec<_> = policy
                .rules
                .iter()
                .filter(|rule| {
                    rule.matches(scope, request)
                        && rule.classifications.contains(&item.classification)
                })
                .collect();
            if matches.is_empty() {
                return held(binding, "policy-denied");
            }
            for rule in matches {
                obligations.extend(rule.obligations.iter().cloned());
            }
            expires_at_ms = expires_at_ms.min(policy.expires_at_ms);
        }
    }
    if !obligations.is_subset(&request.fulfilled_obligations) {
        return held(binding, "obligations-unfulfilled");
    }
    binding.as_object_mut().unwrap().remove("reason");
    binding["policy_refs"] = json!(policy_ids);
    binding["fulfilled_obligations"] = json!(obligations);
    binding["expires_at_ms"] = json!(expires_at_ms);
    binding
}
