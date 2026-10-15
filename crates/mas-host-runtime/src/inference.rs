//! Durable inference turns and explicitly permitted peer delegation.
use mas_host_providers::{Outcome, Request};
use mas_host_sdk::{json, Context, Json, Result};

pub fn execute(
    ctx: &mut Context,
    input: Json,
    configuration: &Json,
    provider: &dyn mas_host_providers::Provider,
) -> Result<Json> {
    if let Some(schema) = configuration.get("input_schema") {
        super::schema::validate(schema, &input)?;
    }
    let delegates: Vec<_> = configuration["delegates"]
        .as_array()
        .map(|a| a.iter().filter_map(Json::as_str).collect())
        .unwrap_or_default();
    let final_schema = &configuration["output_schema"];
    let schema = if delegates.is_empty() {
        final_schema.clone()
    } else {
        action_schema(final_schema, &delegates)
    };
    let max_turns = configuration["max_turns"].as_u64().unwrap_or(1);
    let mut history = Vec::new();
    for turn in 0..max_turns {
        let supplied = if delegates.is_empty() {
            input.clone()
        } else {
            json!({"task":input,"history":history})
        };
        let instructions = if delegates.is_empty() {
            configuration["instructions"]
                .as_str()
                .unwrap_or("")
                .to_string()
        } else {
            format!("{}\nReturn a decision object. A request decision asks one permitted peer for work. A complete decision supplies the final output. Peer replies appear in history. Permitted peers: {}.", configuration["instructions"].as_str().unwrap_or(""), delegates.join(", "))
        };
        let descriptor = json!({"format":"mas-inference/1","configuration":configuration,"input":supplied,"instructions":instructions,"output_schema":schema});
        // Every known provider outcome is checkpointed, including refusal and
        // invalid output. Only lost/unknown observations return an effect error.
        let deadline_ms = ctx.deadline_ms;
        let outcome = ctx.effect(&format!("inference-{turn}"), descriptor, |request_id| {
            let request = Request {
                model: configuration["model"]
                    .as_str()
                    .ok_or("missing model")?
                    .into(),
                instructions,
                input: supplied,
                output_schema: schema.clone(),
                max_output_tokens: configuration["max_output_tokens"].as_u64().unwrap_or(4096),
                request_id: request_id.to_owned(),
                deadline_ms,
            };
            let (output, evidence) = classify(provider.infer(&request), &schema)?;
            Ok((output, evidence))
        })?;
        if outcome["certainty"] != "completed" || outcome["output_status"] != "valid" {
            return Err(format!(
                "provider outcome: {} / {}{}",
                outcome["certainty"].as_str().unwrap_or("invalid"),
                outcome["output_status"].as_str().unwrap_or("unavailable"),
                outcome["reason"]
                    .as_str()
                    .map(|s| format!(": {s}"))
                    .unwrap_or_default()
            ));
        }
        if outcome["cleanup"] == "incomplete" {
            return Err("provider completed; cleanup remains incomplete".into());
        }
        let output = outcome["output"].clone();
        if delegates.is_empty() {
            return Ok(output);
        }
        let decision = &output["decision"];
        match decision["kind"].as_str() {
            Some("complete") => {
                super::schema::validate(final_schema, &decision["output"])?;
                return Ok(decision["output"].clone());
            }
            Some("request") => {
                if turn + 1 >= max_turns {
                    return Err("LLM turn budget cannot accommodate a peer response".into());
                }
                let target = decision["target"].as_str().ok_or("peer target missing")?;
                if !delegates.contains(&target) {
                    return Err("model selected an unpermitted peer".into());
                }
                let input = mas_protocol_codec::parse_json(
                    decision["input_json"]
                        .as_str()
                        .ok_or("peer input_json must be a string")?,
                )
                .map_err(|e| format!("peer input: {}", e.message))?;
                if !input.is_object() {
                    return Err("peer input must be an object".into());
                }
                let result = ctx.request(&format!("peer-{turn}"), target, input)?;
                history.push(json!({"request":decision,"result":result}));
            }
            _ => return Err("model returned an invalid decision".into()),
        }
    }
    Err("LLM turn budget exhausted before completion".into())
}

fn classify(outcome: Outcome, schema: &Json) -> Result<(Json, Json)> {
    match outcome {
        Outcome::Completed {
            output,
            mut evidence,
        } => {
            if super::schema::validate(schema, &output).is_err()
                || !output.is_object()
                || mas_protocol_codec::parse_json(&output.to_string()).is_err()
            {
                evidence["output_status"] = json!("invalid");
                return Ok((
                    json!({"certainty":"completed","output_status":"invalid"}),
                    evidence,
                ));
            }
            if evidence["cleanup"] == "incomplete" {
                return Ok((
                    json!({"certainty":"completed","output_status":"valid","cleanup":"incomplete","output":output}),
                    evidence,
                ));
            }
            Ok((
                json!({"certainty":"completed","output_status":"valid","output":output}),
                evidence,
            ))
        }
        Outcome::NotSent { error, evidence } => Ok((
            json!({"certainty":"not_sent","output_status":"unavailable","reason":error}),
            evidence,
        )),
        Outcome::Unusable { evidence, .. } => {
            let certainty = if evidence["execution"] == "rejected" {
                "rejected"
            } else {
                "completed"
            };
            let status = evidence["output_status"].as_str().unwrap_or("invalid");
            Ok((
                json!({"certainty":certainty,"output_status":status}),
                evidence,
            ))
        }
        Outcome::Unknown { .. } => Err("provider outcome unknown; reconciliation required".into()),
    }
}

fn action_schema(final_schema: &Json, delegates: &[&str]) -> Json {
    json!({"type":"object","properties":{"decision":{"anyOf":[
        {"type":"object","properties":{"kind":{"type":"string","enum":["complete"]},"output":final_schema},"required":["kind","output"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"type":"string","enum":["request"]},"target":{"type":"string","enum":delegates},"input_json":{"type":"string"}},"required":["kind","target","input_json"],"additionalProperties":false}
    ]}},"required":["decision"],"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_invalid_completion_does_not_become_an_unknown_effect() {
        let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        let (result, _) = classify(
            Outcome::Completed {
                output: json!({"ok":"wrong"}),
                evidence: json!({"provider":"mock"}),
            },
            &schema,
        )
        .unwrap();
        assert_eq!(result["certainty"], "completed");
        assert_eq!(result["output_status"], "invalid");
        assert!(classify(
            Outcome::Unknown {
                error: "lost".into(),
                evidence: json!({})
            },
            &schema
        )
        .is_err());
        super::super::schema::check(&action_schema(&schema, &["reviewer"])).unwrap();
    }
}
