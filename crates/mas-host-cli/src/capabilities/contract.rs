//! Validate closed capability declarations independently of files or operator approval.
use crate::config::fields;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(super) fn digest(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 256 && !text.contains('\0'))
        .ok_or_else(|| format!("capability: invalid {key}"))
}

pub(super) fn hash(value: &Value, key: &str) -> Result<(), String> {
    let hash = text(value, key)?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("capability: invalid {key}"));
    }
    Ok(())
}

pub(super) fn names(value: &Value) -> Result<BTreeSet<String>, String> {
    let values = value
        .as_array()
        .filter(|v| v.len() <= 64)
        .ok_or("capability: expected bounded names")?;
    let mut names = BTreeSet::new();
    for value in values {
        let name = value.as_str().ok_or("capability: expected name")?;
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
            || !names.insert(name.into())
        {
            return Err("capability: invalid or duplicate name".into());
        }
    }
    Ok(names)
}

pub(super) fn file_records(value: &Value, allow_empty: bool) -> Result<(), String> {
    let files = value
        .as_array()
        .filter(|v| v.len() <= 64 && (allow_empty || !v.is_empty()))
        .ok_or("capability: expected bounded files")?;
    let mut paths = BTreeSet::new();
    for file in files {
        fields(file, &["path", "sha256"], &[], "capability file")?;
        hash(file, "sha256")?;
        if !paths.insert(text(file, "path")?) {
            return Err("capability: duplicate file".into());
        }
    }
    Ok(())
}

pub(super) fn declaration(value: &Value) -> Result<(), String> {
    fields(
        value,
        &[
            "version",
            "package_id",
            "agent",
            "operation",
            "working_directory",
            "implementation_sha256",
            "executable_sha256",
            "files",
            "runtime",
            "input_schema_sha256",
            "output_schema_sha256",
            "requires",
            "limits",
            "recovery",
        ],
        &[],
        "capability declaration",
    )?;
    if value["version"] != "mas-capability/1" {
        return Err("unsupported capability version".into());
    }
    for key in ["package_id", "agent", "operation"] {
        names(&serde_json::json!([text(value, key)?]))?;
    }
    text(value, "working_directory")?;
    for key in [
        "implementation_sha256",
        "executable_sha256",
        "input_schema_sha256",
        "output_schema_sha256",
    ] {
        hash(value, key)?;
    }
    file_records(&value["files"], false)?;
    fields(
        &value["runtime"],
        &["id", "files"],
        &[],
        "capability runtime",
    )?;
    names(&serde_json::json!([text(&value["runtime"], "id")?]))?;
    file_records(&value["runtime"]["files"], false)?;
    fields(
        &value["requires"],
        &["grants", "credential_references", "destinations"],
        &[],
        "capability requirements",
    )?;
    for key in ["grants", "credential_references", "destinations"] {
        names(&value["requires"][key])?;
    }
    fields(
        &value["limits"],
        &["timeout_seconds", "max_output_bytes"],
        &[],
        "capability limits",
    )?;
    for (key, maximum) in [("timeout_seconds", 3600), ("max_output_bytes", 49152)] {
        value["limits"][key]
            .as_u64()
            .filter(|n| *n > 0 && *n <= maximum)
            .ok_or_else(|| format!("capability: invalid {key}"))?;
    }
    fields(
        &value["recovery"],
        &["effect", "replay", "unknown"],
        &[],
        "capability recovery",
    )?;
    if value["recovery"]["effect"] != "read"
        || value["recovery"]["replay"] != "retained-observation"
        || value["recovery"]["unknown"] != "operator-reconciliation"
    {
        return Err("unsupported capability recovery contract".into());
    }
    Ok(())
}
