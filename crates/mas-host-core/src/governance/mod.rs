//! Versioned contracts for a future governed execution path.
//!
//! These checks neither authenticate callers nor intercept existing execution.
//! Trusted integration code supplies authenticated scope, current authority,
//! the complete host-maintained exposure set and fulfilled obligations. Managed
//! profiles remain unavailable until their enforcement paths are implemented.

mod evidence;
mod identity;
mod policy;
mod profile;
mod result;
pub(crate) mod runtime;

pub use evidence::GovernedEvidence;
pub use identity::{
    ActorId, DestinationId, EvidenceId, InvocationScope, PrincipalId, ProducerId, TenantId,
};
pub use policy::{decide_disclosure, CurrentAuthority, DataUsePolicy, DisclosureRequest};
pub use profile::check_profile;
pub use result::validate_result_view;
pub use runtime::GovernedSession;

use serde_json::Value;
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;

fn record(value: &Value, version: &str, required: &[&str], optional: &[&str]) -> Result<()> {
    super::checked_object(value, "governance record")?;
    if value.to_string().len() > 65_536 {
        return Err("governance record exceeds 65536 bytes".into());
    }
    super::fields(value, required, optional)?;
    if value["version"] != version {
        return Err("unsupported governance record version".into());
    }
    Ok(())
}

fn identifier(value: &Value, key: &str) -> Result<String> {
    let value = super::text(value, key)?;
    if value.is_empty()
        || value.len() > 128
        || value.contains('*')
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(
            "governance identifiers require 1..128 bytes without wildcards or controls".into(),
        );
    }
    Ok(value.to_owned())
}

fn names(value: &Value) -> Result<BTreeSet<String>> {
    let values = value.as_array().ok_or("governance list must be an array")?;
    if values.len() > 64 {
        return Err("governance list exceeds 64 entries".into());
    }
    let mut result = BTreeSet::new();
    for value in values {
        let name = identifier(&serde_json::json!({"id":value}), "id")?;
        if !result.insert(name) {
            return Err("governance list contains duplicate entries".into());
        }
    }
    Ok(result)
}

fn unsigned(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("{key} must be an unsigned integer"))
}

#[cfg(test)]
mod tests;
