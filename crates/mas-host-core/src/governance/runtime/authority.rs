//! Re-read a bounded operator-owned current grant and policy snapshot.
use super::super::{CurrentAuthority, DataUsePolicy};
use serde_json::{json, Value};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
type Result<T> = std::result::Result<T, String>;

pub(super) struct Snapshot {
    pub grant: Value,
    pub policies: Vec<Value>,
    pub policy_digest: String,
}

pub(super) fn read(
    path: &Path,
    tenant: &str,
    principal: &str,
    grant_ref: &str,
) -> Result<Snapshot> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "governed-authority-unavailable")?;
    if !file
        .metadata()
        .map_err(|_| "governed-authority-unavailable")?
        .is_file()
    {
        return Err("governed-authority-unavailable".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "governed-authority-unavailable")?;
    if bytes.len() > 65_536 {
        return Err("governed-authority-limit".into());
    }
    let value = mas_protocol_codec::parse_json(
        std::str::from_utf8(&bytes).map_err(|_| "governed-authority-invalid")?,
    )
    .map_err(|_| "governed-authority-invalid")?;
    super::super::record(
        &value,
        "mas-governed-authority/1",
        &["version", "grants", "policies"],
        &[],
    )
    .map_err(|_| "governed-authority-invalid")?;
    let grants = value["grants"]
        .as_array()
        .filter(|v| v.len() <= 128)
        .ok_or("governed-authority-invalid")?;
    let selected: Vec<_> = grants
        .iter()
        .filter(|g| {
            g["tenant"] == tenant && g["principal"] == principal && g["grant_ref"] == grant_ref
        })
        .collect();
    if selected.len() != 1 {
        return Err("governed-grant-unavailable".into());
    }
    let grant = selected[0].clone();
    CurrentAuthority::from_value(&grant).map_err(|_| "governed-grant-invalid")?;
    if grant["status"] != "active" {
        return Err("governed-grant-revoked".into());
    }
    let now = crate::now_ms().map_err(|_| "governed-clock-unavailable")?;
    if grant["valid_from_ms"].as_u64().is_none_or(|v| now < v)
        || grant["expires_at_ms"].as_u64().is_none_or(|v| now >= v)
    {
        return Err("governed-grant-expired".into());
    }
    let all = value["policies"]
        .as_array()
        .filter(|v| v.len() <= 64)
        .ok_or("governed-policy-invalid")?;
    let mut policies = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for template in all {
        if template["tenant"] != tenant || template["policy_version"] != grant["policy_version"] {
            continue;
        }
        if template.get("evidence_refs").is_some() {
            return Err("governed-policy-association-must-be-host-bound".into());
        }
        let mut probe = template.clone();
        probe["evidence_refs"] = json!(["validation-source"]);
        DataUsePolicy::from_value(&probe).map_err(|_| "governed-policy-invalid")?;
        if !ids.insert(template["id"].as_str().unwrap().to_owned()) {
            return Err("governed-policy-duplicate".into());
        }
        if template["rules"].as_array().unwrap().iter().any(|rule| {
            !rule["obligations"]
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v == "inline-retention-approved"))
        }) {
            return Err("governed-inline-retention-not-approved".into());
        }
        policies.push(template.clone());
    }
    if policies.is_empty() {
        return Err("governed-policy-unavailable".into());
    }
    policies.sort_by_key(|policy| policy["id"].as_str().unwrap().to_owned());
    let policy_digest = crate::digest(json!(policies).to_string().as_bytes());
    Ok(Snapshot {
        grant,
        policies,
        policy_digest,
    })
}
