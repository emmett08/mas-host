//! Validate independently reported execution, payload and evidence dimensions.
use super::{identifier, record, Result};
use serde_json::Value;

fn choice(value: &Value, key: &str, choices: &[&str]) -> Result<()> {
    if !value[key].as_str().is_some_and(|v| choices.contains(&v)) {
        return Err(format!("unsupported protected result {key}"));
    }
    Ok(())
}

/// Validate a protected observation contract without deriving permission or
/// execution certainty from task state, payload availability or assessment.
/// Result metadata itself requires a governed disclosure path.
pub fn validate_result_view(value: &Value) -> Result<()> {
    record(
        value,
        "mas-protected-result/1",
        &[
            "version",
            "invocation_id",
            "task_state",
            "host_settlement",
            "external_execution",
            "payload",
            "evidence",
            "application_assessment",
        ],
        &["cso"],
    )?;
    identifier(value, "invocation_id")?;
    choice(
        value,
        "task_state",
        &[
            "pending",
            "running",
            "completed",
            "failed",
            "unknown",
            "cancelled",
        ],
    )?;
    choice(
        value,
        "host_settlement",
        &["pending", "completed", "failed", "unknown"],
    )?;
    choice(
        value,
        "external_execution",
        &[
            "not-applicable",
            "not-sent",
            "refused",
            "completed",
            "unknown",
        ],
    )?;
    let payload = &value["payload"];
    super::super::fields(payload, &["disposition"], &["reference"])?;
    choice(
        payload,
        "disposition",
        &[
            "available",
            "withheld",
            "expired",
            "deleted",
            "temporarily-inaccessible",
        ],
    )?;
    if payload["disposition"] == "available" {
        identifier(payload, "reference")?;
    } else if payload.get("reference").is_some() {
        return Err("unavailable payload cannot carry a deliverable reference".into());
    }
    let evidence = &value["evidence"];
    super::super::fields(evidence, &["coverage", "freshness"], &[])?;
    choice(
        evidence,
        "coverage",
        &["complete", "partial", "unchecked", "unsupported", "failed"],
    )?;
    choice(
        evidence,
        "freshness",
        &["fresh", "stale", "unknown", "not-applicable"],
    )?;
    choice(
        value,
        "application_assessment",
        &[
            "pass",
            "fail",
            "unknown",
            "error",
            "not-run",
            "not-applicable",
        ],
    )?;
    if let Some(cso) = value.get("cso") {
        super::super::fields(
            cso,
            &["native", "eal_correspondence", "publication", "authority"],
            &[],
        )?;
        choice(
            cso,
            "native",
            &["PASS", "FAIL", "UNKNOWN", "ERROR", "NOT_RUN"],
        )?;
        choice(
            cso,
            "eal_correspondence",
            &["confirmed", "divergent", "unknown", "not-run"],
        )?;
        choice(
            cso,
            "publication",
            &["unpublished", "published", "withdrawn"],
        )?;
        choice(cso, "authority", &["permitted", "denied", "unknown"])?;
    }
    Ok(())
}
