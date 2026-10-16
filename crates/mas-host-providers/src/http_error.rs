//! Retain only fixed diagnostic identifiers from rejected HTTP requests.
use serde_json::{json, Value};

/// Provider errors can echo credentials, prompts and schema property names.
/// Matching complete strings against these fixed lists keeps response text out
/// of evidence while distinguishing actionable rejection causes.
pub(crate) fn retain(target: &mut Value, body: &[u8]) {
    let Ok(response) = serde_json::from_slice::<Value>(body) else {
        return;
    };
    let error = &response["error"];
    for (field, allowed) in [
        (
            "code",
            &[
                "invalid_api_key",
                "invalid_json_schema",
                "insufficient_quota",
                "credit_balance_exhausted",
                "billing_hard_limit_reached",
                "rate_limit_exceeded",
                "slow_down",
                "model_not_found",
                "permission_denied",
                "unsupported_country_region_territory",
                "organization_restricted",
                "project_archived",
                "invalid_parameter",
                "missing_required_parameter",
                "unsupported_parameter",
                "unsupported_value",
                "context_length_exceeded",
                "content_policy_violation",
            ][..],
        ),
        (
            "type",
            &[
                "invalid_request_error",
                "authentication_error",
                "permission_error",
                "rate_limit_error",
                "insufficient_quota",
                "server_error",
            ][..],
        ),
        (
            "param",
            &[
                "model",
                "input",
                "instructions",
                "max_output_tokens",
                "max_tokens",
                "messages",
                "response_format",
                "response_format.type",
                "response_format.json_schema",
                "response_format.json_schema.name",
                "response_format.json_schema.schema",
                "response_format.json_schema.strict",
                "text",
                "text.format",
                "text.format.type",
                "text.format.name",
                "text.format.schema",
                "text.format.strict",
                "tools",
                "tool_choice",
                "parallel_tool_calls",
                "store",
                "stream",
                "temperature",
                "top_p",
                "reasoning",
                "reasoning.effort",
                "service_tier",
                "previous_response_id",
            ][..],
        ),
    ] {
        if let Some(value) = error[field].as_str() {
            if let Some(known) = allowed.iter().find(|known| **known == value) {
                target[format!("error_{field}")] = json!(known);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_non_object_error_diagnostics_are_omitted() {
        for body in [
            b"not JSON".as_slice(),
            br#"{"error":"invalid_api_key"}"#,
            br#"{"error":{"code":["invalid_api_key"],"type":4,"param":null}}"#,
        ] {
            let mut evidence = json!({"http_status": 400});
            retain(&mut evidence, body);
            assert_eq!(evidence, json!({"http_status": 400}));
        }
    }
}
