//! Validate governed evidence references and their retained restrictions.
use super::identity::evidence_names;
use super::{identifier, names, record, unsigned, EvidenceId, Result};
use serde_json::Value;
use std::collections::BTreeSet;

/// Metadata for a bounded governed reference. Resolving the referenced bytes,
/// protected commitment and complete ancestry remains a trusted host duty.
#[derive(Clone, Debug)]
pub struct GovernedEvidence {
    pub(super) id: EvidenceId,
    payload_ref: String,
    pub(super) binding_ref: String,
    source_ref: String,
    acquisition_ref: String,
    transformation_ref: String,
    pub(super) coverage: String,
    pub(super) classification: String,
    pub(super) policy_refs: BTreeSet<String>,
    pub(super) parents: BTreeSet<EvidenceId>,
    pub(super) acquired_at_ms: u64,
    pub(super) expires_at_ms: u64,
}

impl GovernedEvidence {
    pub fn from_value(value: &Value) -> Result<Self> {
        record(
            value,
            "mas-governed-evidence/1",
            &[
                "version",
                "id",
                "payload_ref",
                "binding_ref",
                "source_ref",
                "acquisition_ref",
                "coverage",
                "classification",
                "policy_refs",
                "parents",
                "transformation_ref",
                "acquired_at_ms",
                "expires_at_ms",
            ],
            &[],
        )?;
        let coverage = identifier(value, "coverage")?;
        if !["complete", "partial", "unchecked", "unsupported", "failed"]
            .contains(&coverage.as_str())
        {
            return Err("unsupported governed evidence coverage".into());
        }
        let classification = identifier(value, "classification")?;
        if !["public", "internal", "restricted", "secret", "unchecked"]
            .contains(&classification.as_str())
        {
            return Err("unsupported governed evidence classification".into());
        }
        let acquired_at_ms = unsigned(value, "acquired_at_ms")?;
        let expires_at_ms = unsigned(value, "expires_at_ms")?;
        if expires_at_ms <= acquired_at_ms {
            return Err("evidence validity interval must be nonempty".into());
        }
        let evidence = Self {
            id: EvidenceId::new(&identifier(value, "id")?)?,
            binding_ref: identifier(value, "binding_ref")?,
            payload_ref: identifier(value, "payload_ref")?,
            source_ref: identifier(value, "source_ref")?,
            acquisition_ref: identifier(value, "acquisition_ref")?,
            transformation_ref: identifier(value, "transformation_ref")?,
            coverage,
            classification,
            policy_refs: names(&value["policy_refs"])?,
            parents: evidence_names(&value["parents"])?,
            acquired_at_ms,
            expires_at_ms,
        };
        if evidence.parents.contains(&evidence.id) {
            return Err("evidence cannot be its own parent".into());
        }
        Ok(evidence)
    }

    /// Retain the accepted reference contract without dropping provenance or
    /// inherited restrictions. The caller must govern disclosure of metadata.
    pub fn metadata(&self) -> Value {
        serde_json::json!({
            "version":"mas-governed-evidence/1", "id":self.id.as_str(),
            "payload_ref":self.payload_ref, "binding_ref":self.binding_ref,
            "source_ref":self.source_ref, "acquisition_ref":self.acquisition_ref,
            "transformation_ref":self.transformation_ref, "coverage":self.coverage,
            "classification":self.classification, "policy_refs":self.policy_refs,
            "parents":self.parents.iter().map(EvidenceId::as_str).collect::<Vec<_>>(), "acquired_at_ms":self.acquired_at_ms, "expires_at_ms":self.expires_at_ms,
        })
    }
}
