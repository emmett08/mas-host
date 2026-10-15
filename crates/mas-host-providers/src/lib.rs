//! Provider strategies for one externally observable inference operation.
//!
//! These adapters perform no retries and own no MAS state. The caller reserves
//! and checkpoints each invocation with the host's durable effect protocol.
//! Output schemas are sent to providers; the host remains responsible for local
//! schema validation before treating model output as an operation result.
mod command;
mod http;
mod process;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// One provider call, already admitted by the host effect journal.
#[derive(Clone, Debug)]
pub struct Request {
    pub model: String,
    pub instructions: String,
    pub input: Value,
    pub output_schema: Value,
    pub max_output_tokens: u64,
    pub request_id: String,
    /// Absolute Unix deadline in milliseconds, inherited from the task.
    pub deadline_ms: u64,
}

/// External execution certainty is separate from usefulness of the response.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Completed { output: Value, evidence: Value },
    NotSent { error: String, evidence: Value },
    Unusable { error: String, evidence: Value },
    Unknown { error: String, evidence: Value },
}

/// Strategy interface. Custom providers can implement it without changing host
/// execution, scheduling, protocol admission or workflow composition.
pub trait Provider: Send + Sync {
    fn infer(&self, request: &Request) -> Outcome;
}

/// Factory for built-in strategies; external integrations use the command port.
/// Configuration contains credential *environment names*, never secret values.
pub fn create(config: &Value) -> Result<Box<dyn Provider>, String> {
    let object = config
        .as_object()
        .ok_or("provider connection must be an object")?;
    if object.contains_key("api_key") || object.contains_key("headers") {
        return Err("provider credentials must use api_key_env or explicitly scoped env".into());
    }
    let kind = config["kind"].as_str().ok_or("provider kind is required")?;
    let keys: &[&str] = match kind {
        "mock" => &["kind", "output"],
        "openai" => &["kind", "endpoint", "api_key_env", "env"],
        "openai-compatible" => &[
            "kind",
            "endpoint",
            "api_key_env",
            "env",
            "structured_outputs",
        ],
        "cursor" | "command" => &["kind", "command", "api_key_env", "env"],
        _ => return Err("unknown provider kind".into()),
    };
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("unknown provider connection field".into());
    }
    match kind {
        "mock" => Ok(Box::new(Mock {
            output: object.get("output").cloned(),
        })),
        "openai" => Ok(Box::new(http::Http::new(config, http::Api::Responses)?)),
        "openai-compatible" => Ok(Box::new(http::Http::new(config, http::Api::Chat)?)),
        "cursor" => Ok(Box::new(command::External::new(config, true)?)),
        "command" => Ok(Box::new(command::External::new(config, false)?)),
        _ => Err("unknown provider kind".into()),
    }
}

struct Mock {
    output: Option<Value>,
}
impl Provider for Mock {
    fn infer(&self, request: &Request) -> Outcome {
        if let Err(error) = validate(request) {
            return not_sent(&error, "mock");
        }
        Outcome::Completed {
            output: self.output.clone().unwrap_or_else(|| request.input.clone()),
            evidence: evidence("mock", request, "valid", true),
        }
    }
}

fn validate(request: &Request) -> Result<(), String> {
    if request.model.trim().is_empty() || request.model.len() > 256 || request.model.contains('\0')
    {
        return Err("an explicit bounded model identifier is required".into());
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 256
        || request.request_id.contains(['\r', '\n', '\0'])
    {
        return Err("an explicit bounded request identifier is required".into());
    }
    if request.max_output_tokens == 0 || request.max_output_tokens > 1_000_000 {
        return Err("max_output_tokens must be between 1 and 1000000".into());
    }
    if !request.output_schema.is_object() {
        return Err("output_schema must be a JSON Schema object".into());
    }
    if process::remaining(request.deadline_ms).is_none() {
        return Err("provider deadline has elapsed".into());
    }
    if serde_json::to_vec(&request_value(request))
        .map_err(|_| "invalid provider input")?
        .len()
        > process::MAX_BYTES
    {
        return Err("provider request exceeds byte limit".into());
    }
    Ok(())
}

fn request_value(request: &Request) -> Value {
    json!({"model": request.model, "instructions": request.instructions,
        "input":request.input,"output_schema":request.output_schema,
        "max_output_tokens":request.max_output_tokens,"request_id":request.request_id,
        "deadline_ms":request.deadline_ms})
}

fn evidence(kind: &str, request: &Request, status: &str, token_limit_enforced: bool) -> Value {
    let digest = Sha256::digest(serde_json::to_vec(&request_value(request)).unwrap());
    let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    json!({"adapter":format!("mas-host-{kind}/1"),"requested_model":request.model,
        "request_id":request.request_id,"request_sha256":hash,"output_status":status,
        "token_limit_enforced":token_limit_enforced,"hidden_retries":false})
}

fn not_sent(error: &str, kind: &str) -> Outcome {
    Outcome::NotSent {
        error: error.into(),
        evidence: json!({"adapter":format!("mas-host-{kind}/1"),"execution":"not_sent"}),
    }
}

/// Only preserve bounded identifiers and non-negative integer usage counters.
/// Raw provider errors, headers, stderr and credential-bearing configuration
/// are deliberately excluded from the durable evidence record.
fn retain_metadata(target: &mut Value, source: &Value) {
    for field in ["id", "request_id", "model", "agent_id", "run_id"] {
        if let Some(value) = source[field]
            .as_str()
            .filter(|s| s.len() <= 256 && !s.contains('\0'))
        {
            target[field] = json!(value);
        }
    }
    let mut usage = serde_json::Map::new();
    if let Some(object) = source["usage"].as_object() {
        for field in [
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "prompt_tokens",
            "completion_tokens",
            "inputTokens",
            "outputTokens",
            "totalTokens",
            "cacheReadTokens",
            "cacheWriteTokens",
            "reasoningTokens",
        ] {
            if let Some(value) = object.get(field).and_then(Value::as_u64) {
                usage.insert(field.into(), json!(value));
            }
        }
    }
    if !usage.is_empty() {
        target["usage"] = Value::Object(usage);
    }
}

fn environment_names(config: &Value) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if let Some(env) = config.get("env") {
        for item in env
            .as_array()
            .ok_or("provider env must be an array of environment names")?
        {
            let name = item.as_str().ok_or("provider env name must be a string")?;
            if !valid_env(name) {
                return Err("invalid provider environment name".into());
            }
            if !names.iter().any(|existing| existing == name) {
                names.push(name.into());
            }
        }
    }
    Ok(names)
}

fn valid_env(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && (name.as_bytes()[0].is_ascii_alphabetic() || name.as_bytes()[0] == b'_')
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn request() -> Request {
        Request {
            model: "test-model".into(),
            instructions: "Return JSON".into(),
            input: json!({"n":3}),
            output_schema: json!({"type":"object"}),
            max_output_tokens: 128,
            request_id: "call-1".into(),
            deadline_ms: process::unix_ms() + 5_000,
        }
    }
    #[test]
    fn mock_has_the_same_contract_and_echoes_only_when_output_is_absent() {
        assert!(
            matches!(create(&json!({"kind":"mock"})).unwrap().infer(&request()),
            Outcome::Completed { output,.. } if output == json!({"n":3}))
        );
        assert!(
            matches!(create(&json!({"kind":"mock","output":null})).unwrap().infer(&request()),
            Outcome::Completed { output,.. } if output.is_null())
        );
    }
    #[test]
    fn missing_deadline_is_not_dispatched() {
        let mut request = request();
        request.deadline_ms = 0;
        assert!(matches!(
            create(&json!({"kind":"mock"})).unwrap().infer(&request),
            Outcome::NotSent { .. }
        ));
    }
    #[test]
    fn metadata_does_not_retain_errors_or_decimal_usage() {
        let mut result = json!({});
        retain_metadata(
            &mut result,
            &json!({"id":"response-1","error":"secret","usage":{"total_tokens":4,"cost":0.4}}),
        );
        assert_eq!(
            result,
            json!({"id":"response-1","usage":{"total_tokens":4}})
        );
        assert!(create(&json!({"kind":"openai","api_key":"secret"})).is_err());
        assert!(create(&json!({"kind":"mock","typo":true})).is_err());
    }
}
