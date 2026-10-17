//! Project audit history without widening its inherited reader restrictions.
use super::super::{error, Result};
use super::store;
use serde_json::{json, Value};

pub(crate) fn audit(db: &rusqlite::Connection) -> Result<Value> {
    store::audit(db)
}

/// Payload opt-in is necessary but never sufficient to disclose restricted data.
pub(crate) fn public_audit(mut audit: Value, include_payloads: bool) -> Value {
    if audit.is_null() {
        return audit;
    }
    for record in audit["records"].as_array_mut().unwrap() {
        if record["kind"] == "candidate" {
            record["output_sha256"] = record["record"]["output_sha256"].clone();
        }
        if !include_payloads || record["readers"] != json!(["*"]) {
            record.as_object_mut().unwrap().remove("record");
            record["redacted"] = json!(true);
        }
    }
    for context in audit["contexts"].as_array_mut().unwrap() {
        if !include_payloads || context["readers"] != json!(["*"]) {
            context.as_object_mut().unwrap().remove("input");
            context.as_object_mut().unwrap().remove("policy");
            context["redacted"] = json!(true);
        }
    }
    audit["candidates"] = json!(audit["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "candidate")
        .filter_map(|r| r.get("record").cloned())
        .collect::<Vec<_>>());
    audit["assessments"] = json!(audit["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "assessment")
        .filter_map(|r| r.get("record").cloned())
        .collect::<Vec<_>>());
    audit
}

pub(crate) fn step_public(db: &rusqlite::Connection, id: &str) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='revision_contexts')",
            [],
            |r| r.get(0),
        )
        .map_err(error)?;
    if !exists {
        return Ok(true);
    }
    let active: bool = db
        .query_row("SELECT EXISTS(SELECT 1 FROM revision_plan)", [], |r| {
            r.get(0)
        })
        .map_err(error)?;
    if !active {
        return Ok(true);
    }
    let value:Option<String>=db.query_row("SELECT body FROM revision_contexts WHERE id=? OR id=(SELECT parent FROM steps WHERE id=?) LIMIT 1",[id,id],|r|r.get(0)).optional().map_err(error)?;
    value
        .map(|s| {
            serde_json::from_str::<Value>(&s)
                .map(|v| v["readers"] == json!(["*"]))
                .map_err(error)
        })
        .unwrap_or(Ok(false))
}
