//! Reject deployment profiles whose mandatory enforcement is unimplemented.
use super::{names, record, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const LOCAL: &[&str] = &["trusted-local-execution", "local-effect-replay"];
const MANAGED: &[&str] = &[
    "authenticated-scope",
    "pre-persistence-handling",
    "current-disclosure-admission",
    "governed-payload-lifecycle",
    "effect-output-handling",
];
const SHARED: &[&str] = &["shared-authority", "fenced-scheduling", "shared-quotas"];

/// Check requirements against this implementation, not a caller's declaration
/// of available capabilities. Success describes trusted-local compatibility.
/// This function performs no activation, execution or persistence.
pub fn check_profile(value: &Value) -> Result<Value> {
    record(
        value,
        "mas-host-profile/1",
        &["version", "profile", "required_capabilities"],
        &["worker_isolation"],
    )?;
    let profile = value["profile"]
        .as_str()
        .ok_or("profile must be a string")?;
    let mut required = names(&value["required_capabilities"])?;
    for name in &required {
        if !LOCAL.contains(&name.as_str())
            && !MANAGED.contains(&name.as_str())
            && !SHARED.contains(&name.as_str())
            && name != "worker-isolation"
        {
            return Err("unknown required governance capability".into());
        }
    }
    let isolation = match value.get("worker_isolation") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or("worker_isolation must be a boolean")?,
    };
    if isolation {
        required.insert("worker-isolation".to_owned());
    }
    match profile {
        "trusted-local" => {}
        "managed-cluster" => required.extend(MANAGED.iter().map(|name| (*name).to_owned())),
        "shared-enterprise" => {
            required.extend(MANAGED.iter().chain(SHARED).map(|name| (*name).to_owned()));
        }
        _ => return Err("unsupported deployment profile".into()),
    }
    let supported: BTreeSet<_> = LOCAL.iter().map(|name| (*name).to_owned()).collect();
    let missing: Vec<_> = required.difference(&supported).cloned().collect();
    if !missing.is_empty() {
        return Err(format!(
            "unimplemented required capabilities: {}",
            missing.join(", ")
        ));
    }
    Ok(json!({
        "version":"mas-host-profile-check/1", "profile":profile,
        "accepted":true, "implemented_capabilities":supported,
        "required_capabilities":required,
        "execution_boundary":"trusted-local",
        "activation":"none"
    }))
}
