//! Durable inference turns and explicitly permitted peer delegation.
use mas_host_providers::{Outcome, Request};
use mas_host_sdk::{json, Context, Json, Result};
use sha2::{Digest, Sha256};

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
    let max_turns = configuration["max_turns"].as_u64().unwrap_or(1);
    let min_delegate_calls = configuration["min_delegate_calls"].as_u64().unwrap_or(0);
    let required_delegates: Vec<_> = configuration["required_delegates"]
        .as_array()
        .map(|a| a.iter().filter_map(Json::as_str).collect())
        .unwrap_or_default();
    let mut history: Vec<Json> = Vec::new();
    for turn in 0..max_turns {
        // History only gains an entry after ctx.request returns successfully.
        // Replayed replies rebuild the same coverage without another dispatch.
        let outstanding: Vec<_> = required_delegates
            .iter()
            .copied()
            .filter(|target| {
                !history
                    .iter()
                    .any(|entry| entry["request"]["target"].as_str() == Some(*target))
            })
            .collect();
        let turn_delegates = if outstanding.is_empty() {
            delegates.as_slice()
        } else {
            outstanding.as_slice()
        };
        let can_complete = history.len() as u64 >= min_delegate_calls && outstanding.is_empty();
        // Preserve previous schemas when distinct coverage was not requested.
        let can_request = required_delegates.is_empty() || turn + 1 < max_turns;
        let schema = if delegates.is_empty() {
            final_schema.clone()
        } else {
            action_schema(final_schema, turn_delegates, can_complete, can_request)
        };
        let supplied = if delegates.is_empty() {
            input.clone()
        } else {
            json!({"task":input,"history":history})
        };
        let mut instructions = if delegates.is_empty() {
            configuration["instructions"]
                .as_str()
                .unwrap_or("")
                .to_string()
        } else {
            format!("{}\nReturn a decision object. A request decision asks one permitted peer for work. A complete decision supplies the final output. Peer replies appear in history. Permitted peers: {}. Operation contracts: {}.", configuration["instructions"].as_str().unwrap_or(""), delegates.join(", "), configuration["delegate_contracts"])
        };
        if (history.len() as u64) < min_delegate_calls {
            instructions.push_str(&format!("\nCompletion requires at least {min_delegate_calls} completed peer requests; {} are recorded. Return a request decision. The response schema excludes completion until this requirement is met.", history.len()));
        }
        if !outstanding.is_empty() {
            instructions.push_str(&format!("\nCompletion requires a returned observation from every required peer. Still required: {}. Request one of these peers next; the response schema excludes other targets and completion until all required peers have returned.", outstanding.join(", ")));
        }
        if !can_request {
            instructions.push_str("\nThis is the final inference turn. Return a complete decision using the recorded peer observations; no further peer request is permitted.");
        }
        let descriptor =
            super::identity::effect_descriptor("mas-inference/2", configuration, &supplied);
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
                if (history.len() as u64) < min_delegate_calls {
                    return Err("LLM completion requires more completed delegate calls".into());
                }
                if !outstanding.is_empty() {
                    return Err("LLM completion requires all required peer observations".into());
                }
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
                if !turn_delegates.contains(&target) {
                    return Err("model must select an outstanding required peer".into());
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
            let encoded = output.to_string();
            let invalid = if !output.is_object() {
                Some("expected-object")
            } else if mas_protocol_codec::parse_json(&encoded).is_err() {
                Some("integer-json")
            } else if super::schema::validate(schema, &output).is_err() {
                Some("schema-mismatch")
            } else {
                None
            };
            if let Some(code) = invalid {
                evidence["output_status"] = json!("invalid");
                // Preserve the rejection boundary, not model text or schema
                // errors that could contain untrusted property names/values.
                let digest = Sha256::digest(encoded.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                let diagnostics = &mut evidence["output_diagnostics"];
                if !diagnostics.is_object() {
                    *diagnostics = json!({});
                }
                diagnostics["stage"] = json!("output-schema");
                diagnostics["code"] = json!(code);
                diagnostics["output_bytes"] = json!(encoded.len());
                diagnostics["output_sha256"] = json!(digest);
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

fn action_schema(
    final_schema: &Json,
    delegates: &[&str],
    can_complete: bool,
    can_request: bool,
) -> Json {
    let mut decisions = Vec::new();
    if can_complete {
        decisions.push(json!({"type":"object","properties":{"kind":{"type":"string","enum":["complete"]},"output":final_schema},"required":["kind","output"],"additionalProperties":false}));
    }
    if can_request {
        decisions.push(json!({"type":"object","properties":{"kind":{"type":"string","enum":["request"]},"target":{"type":"string","enum":delegates},"input_json":{"type":"string"}},"required":["kind","target","input_json"],"additionalProperties":false}));
    }
    json!({"type":"object","properties":{"decision":{"anyOf":decisions}},"required":["decision"],"additionalProperties":false})
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
        super::super::schema::check(&action_schema(&schema, &["reviewer"], true, true)).unwrap();
    }

    #[test]
    fn rejection_diagnostics_distinguish_local_validation_without_retaining_output() {
        let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        for (output, code) in [
            (json!({"private-model-text":"secret"}), "schema-mismatch"),
            (json!(["private-model-text"]), "expected-object"),
            (json!({"n":1.5}), "integer-json"),
        ] {
            let encoded = output.to_string();
            let (result, evidence) = classify(
                Outcome::Completed {
                    output,
                    evidence: json!({"output_diagnostics":{"eligible_messages":1}}),
                },
                &schema,
            )
            .unwrap();
            assert_eq!(
                result,
                json!({"certainty":"completed","output_status":"invalid"})
            );
            let diagnostic = &evidence["output_diagnostics"];
            assert_eq!(diagnostic["stage"], "output-schema");
            assert_eq!(diagnostic["code"], code);
            assert_eq!(diagnostic["eligible_messages"], 1);
            assert_eq!(diagnostic["output_bytes"], encoded.len());
            assert_eq!(
                diagnostic["output_sha256"],
                Sha256::digest(encoded.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            assert!(!evidence.to_string().contains("private-model-text"));
            assert!(!evidence.to_string().contains("secret"));
        }
    }

    #[test]
    fn required_delegation_schema_excludes_early_completion() {
        let final_schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        let completion = json!({"decision":{"kind":"complete","output":{"ok":true}}});
        let request = json!({"decision":{"kind":"request","target":"reviewer","input_json":"{}"}});
        for can_complete in [false, true] {
            let schema = action_schema(&final_schema, &["reviewer"], can_complete, true);
            super::super::schema::check(&schema).unwrap();
            assert!(super::super::schema::validate(&schema, &request).is_ok());
            assert_eq!(
                super::super::schema::validate(&schema, &completion).is_ok(),
                can_complete
            );
            let (result, _) = classify(
                Outcome::Completed {
                    output: completion.clone(),
                    evidence: json!({}),
                },
                &schema,
            )
            .unwrap();
            assert_eq!(result["certainty"], "completed");
            assert_eq!(
                result["output_status"],
                if can_complete { "valid" } else { "invalid" }
            );
        }
        let final_turn = action_schema(&final_schema, &["reviewer"], true, false);
        super::super::schema::check(&final_turn).unwrap();
        assert!(super::super::schema::validate(&final_turn, &completion).is_ok());
        assert!(super::super::schema::validate(&final_turn, &request).is_err());
    }
}
