//! Apply the structured gate at existing request/effect completion boundaries.
use super::{
    schema,
    session::{is_unavailable, unavailable},
    storage,
};
use crate::{lock, Inner, Result, Step};
use serde_json::{json, Value};

fn certainty(output: &Value, evidence: &Value) -> &'static str {
    let reported = output["certainty"]
        .as_str()
        .or_else(|| evidence["execution"].as_str());
    match reported {
        Some("completed") => "completed",
        Some("not_sent" | "not_started" | "not-sent") => "not-sent",
        Some("rejected" | "refused") => "refused",
        _ if output["status"] == "not_started" => "not-sent",
        _ => "unknown",
    }
}

fn completion_values(
    effect_id: &str,
    certainty: &str,
    output: Value,
    reported_evidence: Option<Value>,
) -> (Value, Value, Value) {
    let disposition = if reported_evidence.is_some() {
        "available"
    } else {
        "withheld"
    };
    let mut evidence = json!({"version":"mas-governed-effect-evidence/1",
        "external_execution":certainty,"payload_disposition":disposition});
    if let Some(reported) = reported_evidence {
        evidence["reported_evidence"] = reported;
    }
    let completion = json!({"version":"mas-governed-effect-completion/1","effect_id":effect_id,
        "host_settlement":"completed","external_execution":certainty,"payload_disposition":disposition,"output":output});
    (output, evidence, completion)
}

fn available_completion(
    effect_id: &str,
    certainty: &str,
    output: Value,
    evidence: Value,
) -> Result<(Value, Value, Value)> {
    let values = completion_values(effect_id, certainty, output, Some(evidence));
    if !valid_completion_ack(&values.2) {
        return Err("governed-completion-limit".into());
    }
    Ok(values)
}

// The receipt is nested in a native response. Check that exact envelope before
// retaining an available payload, so framing failure cannot erase completion.
pub(super) fn valid_completion_ack(completion: &Value) -> bool {
    let frame = json!({"type":"response","ok":true,"completion":completion});
    super::encoding::canonical(&frame, 65_536)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|body| mas_protocol_codec::parse_json(&body).is_ok())
}

impl Inner {
    pub(crate) fn validate_worker_output(&self, actor: &str, output: &Value) -> Result<()> {
        if self.governance.is_some() && is_unavailable(output) {
            return Ok(());
        }
        if let Some(schema) = &self
            .agents
            .get(actor)
            .ok_or("unknown application agent")?
            .output_schema
        {
            mas_host_schema::validate(schema, output)?;
        }
        Ok(())
    }

    pub(crate) fn governed_input(
        &self,
        kind: &str,
        actor: &str,
        target: &str,
        input: &Value,
    ) -> Result<Option<(Value, String, Value)>> {
        let Some(session) = &self.governance else {
            return Ok(None);
        };
        let original = session.commitment(input)?;
        let (projection, decision) = match kind {
            "request" => session.prepare_request(target, input)?,
            "effect" => session.prepare_effect(actor, input)?,
            _ => return Err("governed-step-kind-unsupported".into()),
        };
        Ok(Some((projection, original, decision)))
    }

    pub(crate) fn governed_output(&self, step: &Step, output: &Value) -> Value {
        match &self.governance {
            None => output.clone(),
            Some(session) => {
                if is_unavailable(output) {
                    let recorded=lock(&self.journal).and_then(|db|db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM steps WHERE kind='effect' AND state IN ('completed','resolved') AND parent=? AND actor=? AND output=?)",
                        rusqlite::params![step.id,step.target,output.to_string()],|row|row.get::<_,bool>(0)).map_err(|_|"governed-storage-unavailable".into()))
                        .unwrap_or(false);
                    // This copies an already retained host fact; current user
                    // authority is checked separately before any delivery.
                    if recorded {
                        return output.clone();
                    }
                    return unavailable("unknown");
                }
                session
                    .project_output(&step.target, output)
                    .unwrap_or_else(|_| unavailable("unknown"))
            }
        }
    }

    pub(crate) fn governed_request_delivery(&self, step: &Step, output: &Value) -> Result<()> {
        if let Some(session) = &self.governance {
            session.request_delivery(&step.target, &step.actor, output)?;
        }
        Ok(())
    }

    pub(crate) fn governed_error(&self, error: &str) -> String {
        if self.governance.is_some() {
            "governed-worker-or-operation-failed".into()
        } else {
            crate::bounded_error(error)
        }
    }

    pub(crate) fn governed_effect_completion(
        &self,
        step: &Step,
        output: &Value,
        evidence: &Value,
    ) -> Result<Option<(Value, Value, Value)>> {
        let Some(session) = &self.governance else {
            return Ok(None);
        };
        let certainty = certainty(output, evidence);
        let project = || -> Result<(Value, Value, Value)> {
            session.check_current()?;
            let policy = session.effect(
                &step.actor,
                step.input["format"]
                    .as_str()
                    .ok_or("governed-effect-format")?,
            )?;
            let projected_output = schema::project(&policy.output_schema, output)?;
            if is_unavailable(&projected_output) {
                return Err("governed-reserved-result".into());
            }
            let projected_evidence = schema::project(&policy.evidence_schema, evidence)?;
            if projected_evidence.as_object().is_none_or(|o| o.is_empty()) {
                return Err("governed-evidence-empty".into());
            }
            let (safe_output, safe_evidence, completion) =
                available_completion(&step.id, certainty, projected_output, projected_evidence)?;
            // Bind every retained/disclosed value, including supporting
            // evidence and host receipt metadata, to the current policy.
            session.authorize(
                &step.actor,
                "invoke",
                &format!("worker:{}", step.actor),
                &session.agent(&step.actor)?.destination,
                &json!([output, evidence]),
                &json!({"output":safe_output,"evidence":safe_evidence,"completion":completion}),
            )?;
            Ok((safe_output, safe_evidence, completion))
        };
        let (output, evidence, completion) = project().unwrap_or_else(|_| {
            completion_values(&step.id, certainty, unavailable(certainty), None)
        });
        Ok(Some((output, evidence, completion)))
    }

    pub(crate) fn governed_completion_ack(&self, id: &str) -> Result<Option<Value>> {
        if self.governance.is_none() {
            return Ok(None);
        }
        storage::completion(&*lock(&self.journal)?, id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_native_ack_envelope_limits_preserve_small_completion_receipt() {
        let id = format!("eff-{}", "0".repeat(64));
        let output = json!({"payload":"x".repeat(65_000)});
        let (_, _, base) = completion_values(
            &id,
            "completed",
            output,
            Some(json!({"execution":"completed"})),
        );
        let overhead = base.to_string().len() - 65_000;
        let output = json!({"payload":"x".repeat(65_536 - overhead)});
        let (_, _, body) = completion_values(
            &id,
            "completed",
            output.clone(),
            Some(json!({"execution":"completed"})),
        );
        assert_eq!(body.to_string().len(), 65_536);
        assert!(!valid_completion_ack(&body));
        let values =
            available_completion(&id, "completed", output, json!({"execution":"completed"}))
                .unwrap_or_else(|_| {
                    completion_values(&id, "completed", unavailable("completed"), None)
                });
        assert_eq!(values.2["host_settlement"], "completed");
        assert_eq!(values.2["external_execution"], "completed");
        assert_eq!(values.2["payload_disposition"], "withheld");
        assert!(valid_completion_ack(&values.2));

        let mut deep = json!({});
        for _ in 0..31 {
            deep = json!({"nested":deep});
        }
        assert!(
            available_completion(&id, "completed", deep, json!({"execution":"completed"})).is_err()
        );
    }
}
