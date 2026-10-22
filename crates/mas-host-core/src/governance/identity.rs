//! Distinct identity domains and the trusted ingress scope contract.
use super::{identifier, names, record, unsigned, Result};
use serde_json::Value;
use std::collections::BTreeSet;

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: &str) -> Result<Self> {
                identifier(&serde_json::json!({"id": value}), "id").map(Self)
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identity!(TenantId);
identity!(PrincipalId);
identity!(ActorId);
identity!(ProducerId);
identity!(DestinationId);
identity!(EvidenceId);

pub(super) fn evidence_names(value: &Value) -> Result<BTreeSet<EvidenceId>> {
    names(value)?
        .iter()
        .map(|name| EvidenceId::new(name))
        .collect()
}

/// Authenticated scope supplied by trusted ingress, never by worker assertions.
/// `evidence_refs` is the complete exposure set for this invocation. It is not
/// a worker's selection of dependencies or a set of merely available sources.
#[derive(Clone, Debug)]
pub struct InvocationScope {
    pub(super) tenant: TenantId,
    pub(super) principal: PrincipalId,
    pub(super) actor: ActorId,
    pub(super) producer: ProducerId,
    pub(super) grant_ref: String,
    pub(super) grant_epoch: u64,
    pub(super) purpose: String,
    pub(super) policy_version: String,
    pub(super) permitted_operations: BTreeSet<String>,
    pub(super) evidence_refs: BTreeSet<EvidenceId>,
}

impl InvocationScope {
    /// Validate a scope already derived by trusted authenticated ingress.
    /// Parsing does not establish the truth or current authority of its fields.
    pub fn from_value(value: &Value) -> Result<Self> {
        record(
            value,
            "mas-invocation-scope/1",
            &[
                "version",
                "tenant",
                "principal",
                "actor",
                "producer",
                "grant_ref",
                "grant_epoch",
                "purpose",
                "policy_version",
                "permitted_operations",
                "evidence_refs",
            ],
            &[],
        )?;
        Ok(Self {
            tenant: TenantId::new(&identifier(value, "tenant")?)?,
            principal: PrincipalId::new(&identifier(value, "principal")?)?,
            actor: ActorId::new(&identifier(value, "actor")?)?,
            producer: ProducerId::new(&identifier(value, "producer")?)?,
            grant_ref: identifier(value, "grant_ref")?,
            grant_epoch: unsigned(value, "grant_epoch")?,
            purpose: identifier(value, "purpose")?,
            policy_version: identifier(value, "policy_version")?,
            permitted_operations: names(&value["permitted_operations"])?,
            evidence_refs: evidence_names(&value["evidence_refs"])?,
        })
    }
}
