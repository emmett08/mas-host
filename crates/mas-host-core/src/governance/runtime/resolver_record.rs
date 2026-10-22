//! Validate retained effect truth separately from permission to disclose payloads.
use super::{schema, session::is_unavailable, GovernedSession};
use serde_json::{json, Value};
type Result<T> = std::result::Result<T, String>;

pub(super) fn certainty(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|s| ["completed", "not-sent", "refused", "unknown"].contains(s))
        .ok_or_else(|| "governed-evidence-invalid".into())
}

pub(super) fn validate_completion(
    effect_id: &str,
    output: &Value,
    evidence: &Value,
    completion: &Value,
) -> Result<()> {
    crate::fields(
        completion,
        &[
            "version",
            "effect_id",
            "host_settlement",
            "external_execution",
            "payload_disposition",
            "output",
        ],
        &[],
    )?;
    crate::fields(
        evidence,
        &["version", "external_execution", "payload_disposition"],
        &["reported_evidence", "inspection"],
    )?;
    if completion["version"] != "mas-governed-effect-completion/1"
        || completion["effect_id"] != effect_id
        || completion["host_settlement"] != "completed"
        || completion["output"] != *output
        || evidence["version"] != "mas-governed-effect-evidence/1"
        || completion["external_execution"] != evidence["external_execution"]
        || completion["payload_disposition"] != evidence["payload_disposition"]
        || !output.is_object()
    {
        return Err("governed-evidence-invalid".into());
    }
    certainty(&completion["external_execution"])?;
    match completion["payload_disposition"].as_str() {
        Some("available")
            if !is_unavailable(output)
                && evidence["reported_evidence"]
                    .as_object()
                    .is_some_and(|v| !v.is_empty()) =>
        {
            Ok(())
        }
        Some("withheld")
            if is_unavailable(output)
                && output["external_execution"] == completion["external_execution"]
                && evidence.get("reported_evidence").is_none()
                && evidence.get("inspection").is_none() =>
        {
            Ok(())
        }
        _ => Err("governed-evidence-invalid".into()),
    }
}

pub(super) fn validate_payload(session: &GovernedSession, actor: &str, view: &Value) -> Result<()> {
    let descriptor = &view["descriptor"];
    let format = descriptor["format"]
        .as_str()
        .ok_or("governed-evidence-invalid")?;
    let policy = session.effect(actor, format)?;
    if schema::project(&policy.descriptor_schema, descriptor)? != *descriptor {
        return Err("governed-evidence-schema-changed".into());
    }
    if view["host_settlement"] != "completed" {
        return Ok(());
    }
    if view["payload_disposition"] == "available" {
        if schema::project(&policy.output_schema, &view["output"])? != view["output"]
            || schema::project(
                &policy.evidence_schema,
                &view["evidence"]["reported_evidence"],
            )? != view["evidence"]["reported_evidence"]
        {
            return Err("governed-evidence-schema-changed".into());
        }
        if let Some(receipt) = view["evidence"].get("inspection") {
            validate_inspection(
                session,
                receipt,
                &view["output"],
                &view["evidence"]["reported_evidence"],
            )?;
        }
    }
    Ok(())
}

fn validate_inspection(
    session: &GovernedSession,
    receipt: &Value,
    output: &Value,
    evidence: &Value,
) -> Result<()> {
    crate::fields(
        receipt,
        &[
            "version",
            "contract",
            "original_binding",
            "projection_binding",
            "completion",
            "coverage",
            "original_findings_count",
            "projected_findings_count",
            "text_units",
            "inspected_sources",
            "boundary_bytes",
        ],
        &["component_assessments", "quarantine_ref"],
    )?;
    if let Some(reference) = receipt.get("quarantine_ref") {
        if session.0.quarantine.is_none()
            || !reference.as_str().is_some_and(|s| {
                s.strip_prefix("quarantine-").is_some_and(|id| {
                    id.len() == 64
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            })
        {
            return Err("governed-evidence-inspection-contract".into());
        }
    }
    let gate = session
        .2
        .as_ref()
        .ok_or("governed-evidence-inspection-contract")?;
    let is_binding = |value: &Value| {
        value.as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    };
    if receipt["version"] != "mas-governed-inspection-receipt/1"
        || receipt["contract"] != gate.contract()
        || receipt["completion"] != "complete"
        || receipt["coverage"] != gate.coverage()
        || !is_binding(&receipt["original_binding"])
        || !is_binding(&receipt["projection_binding"])
        || receipt["projection_binding"] != session.commitment(&json!([output, evidence]))?
        || receipt["projected_findings_count"] != 0
        || receipt["original_findings_count"]
            .as_u64()
            .is_none_or(|n| n > 16_384)
        || receipt["text_units"].as_u64().is_none_or(|n| n > 128)
        || receipt["inspected_sources"]
            .as_u64()
            .is_none_or(|n| n > receipt["text_units"].as_u64().unwrap_or(0))
        || receipt["boundary_bytes"]
            .as_u64()
            .is_none_or(|n| n > 16_384)
    {
        return Err("governed-evidence-inspection-contract".into());
    }
    if let Some(components) = receipt.get("component_assessments") {
        if receipt["contract"]["version"] != "mas-governed-inspection/3" {
            return Err("governed-assessment-contract".into());
        }
        let components = components
            .as_array()
            .filter(|items| items.len() <= 128)
            .ok_or("governed-assessment-contract")?;
        for component in components {
            crate::fields(component, &["binding", "record"], &[])?;
            let record = &component["record"];
            crate::fields(
                record,
                &["index", "scope", "source_binding", "assessment"],
                &[],
            )?;
            if component["binding"] != session.commitment(record)?
                || !is_binding(&record["source_binding"])
                || record["index"].as_u64().is_none_or(|n| n >= 128)
                || ![json!("original"), json!("projection")].contains(&record["scope"])
                || record["assessment"]["version"] != "mas-combined-privacy-assessment/1"
            {
                return Err("governed-assessment-contract".into());
            }
        }
    } else if receipt["contract"]["version"] == "mas-governed-inspection/3" {
        return Err("governed-assessment-contract".into());
    }
    Ok(())
}
