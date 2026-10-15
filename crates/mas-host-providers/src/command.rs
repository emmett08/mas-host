use crate::{
    environment_names, evidence, not_sent, process, request_value, retain_metadata, valid_env,
    validate, Outcome, Provider, Request,
};
use serde_json::{json, Value};
use std::process::Command;

/// Adapter for the versioned stdin/stdout extension port, including the
/// maintained Cursor SDK helper. No shell interpolation is performed.
pub(crate) struct External {
    command: Vec<String>,
    env: Vec<String>,
    cursor: bool,
    key_env: Option<String>,
}
impl External {
    pub fn new(config: &Value, cursor: bool) -> Result<Self, String> {
        let command = config["command"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 64)
            .ok_or("provider command must be a nonempty bounded argv array")?;
        let command = command
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0'))
                    .map(str::to_owned)
                    .ok_or_else(|| "invalid provider command argument".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let key_env = match config.get("api_key_env") {
            Some(value) => {
                let name = value.as_str().ok_or("api_key_env must be a string")?;
                if !valid_env(name) {
                    return Err("invalid api_key_env".into());
                }
                Some(name.to_string())
            }
            None if cursor => Some("CURSOR_API_KEY".into()),
            None => None,
        };
        let mut env = environment_names(config)?;
        if let Some(name) = &key_env {
            if !env.contains(name) {
                env.push(name.clone());
            }
        }
        Ok(Self {
            command,
            env,
            cursor,
            key_env,
        })
    }
    fn kind(&self) -> &str {
        if self.cursor {
            "cursor"
        } else {
            "command"
        }
    }
}
impl Provider for External {
    fn infer(&self, request: &Request) -> Outcome {
        if let Err(error) = validate(request) {
            return not_sent(&error, self.kind());
        }
        if let Some(name) = &self.key_env {
            if std::env::var(name)
                .ok()
                .filter(|s| !s.is_empty() && !s.contains('\0'))
                .is_none()
            {
                return not_sent(
                    "provider credential environment is missing or invalid",
                    self.kind(),
                );
            }
        }
        let mut input = request_value(request);
        input["version"] = json!("mas-provider/1");
        // Opaque exact documents preserve application integers in JS helpers.
        input["input_json"] = json!(request.input.to_string());
        input["output_schema_json"] = json!(request.output_schema.to_string());
        // This is the environment *name*, not the credential. A Cursor
        // connection may use a team-specific variable rather than the default.
        if let Some(name) = &self.key_env {
            input["api_key_env"] = json!(name);
        }
        let mut command = Command::new(&self.command[0]);
        command.args(&self.command[1..]);
        process::scoped_environment(&mut command, &self.env);
        let mut ev = evidence(self.kind(), request, "unavailable", false);
        let result =
            match process::run(command, input.to_string().into_bytes(), request.deadline_ms) {
                Ok(output) => output,
                Err(process::Failure::NotStarted) => {
                    return not_sent("cannot start provider command", self.kind())
                }
                Err(process::Failure::Incomplete) => {
                    return Outcome::Unknown {
                        error: "provider command did not complete within its limits".into(),
                        evidence: ev,
                    }
                }
            };
        if !result.status.success() {
            return Outcome::Unknown {
                error: "provider command failed; external outcome may be unknown".into(),
                evidence: ev,
            };
        }
        let response: Value = match std::str::from_utf8(&result.stdout)
            .ok()
            .and_then(|text| mas_protocol_codec::parse_json(text).ok())
        {
            Some(value) => value,
            None => {
                return Outcome::Unknown {
                    error: "provider command returned an invalid outcome envelope".into(),
                    evidence: ev,
                }
            }
        };
        if response["version"] != "mas-provider/1" {
            return Outcome::Unknown {
                error: "provider command outcome version is unsupported".into(),
                evidence: ev,
            };
        }
        // Extensions cannot inject arbitrary evidence or raw provider errors.
        retain_metadata(&mut ev, &response["evidence"]);
        if let Some(cleanup) = response["evidence"]["cleanup"]
            .as_str()
            .filter(|s| matches!(*s, "confirmed" | "incomplete"))
        {
            ev["cleanup"] = json!(cleanup);
        }
        match response["status"].as_str() {
            Some("completed") if response["output_json"].is_string() => {
                match mas_protocol_codec::parse_json(response["output_json"].as_str().unwrap()) {
                    Ok(output) => {
                        ev["output_status"] = json!("json");
                        Outcome::Completed {
                            output,
                            evidence: ev,
                        }
                    }
                    Err(_) => {
                        ev["output_status"] = json!("invalid");
                        Outcome::Unusable {
                            error: "provider command output violates the MAS JSON contract".into(),
                            evidence: ev,
                        }
                    }
                }
            }
            Some("completed") if response.get("output").is_some() => {
                ev["output_status"] = json!("json");
                Outcome::Completed {
                    output: response["output"].clone(),
                    evidence: ev,
                }
            }
            Some("not_sent") => {
                ev["output_status"] = json!("unavailable");
                Outcome::NotSent {
                    error: "provider command did not send the inference request".into(),
                    evidence: ev,
                }
            }
            Some("unusable") => {
                ev["output_status"] = json!("invalid");
                if let Some(status) = response["evidence"]["output_status"]
                    .as_str()
                    .filter(|s| matches!(*s, "invalid" | "refused" | "incomplete"))
                {
                    ev["output_status"] = json!(status);
                }
                Outcome::Unusable {
                    error: "provider command completed without usable output".into(),
                    evidence: ev,
                }
            }
            _ => Outcome::Unknown {
                error: "provider command did not establish an inference outcome".into(),
                evidence: ev,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider(script: &str, cursor: bool) -> External {
        External {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: Vec::new(),
            cursor,
            key_env: None,
        }
    }
    #[test]
    fn command_contract_and_cursor_fixture_use_the_same_port() {
        let script="cat >/dev/null; printf '%s' '{\"version\":\"mas-provider/1\",\"status\":\"completed\",\"output\":{\"answer\":7},\"evidence\":{\"model\":\"cursor-test\",\"cleanup\":\"confirmed\",\"secret\":\"excluded\"}}'";
        for cursor in [false, true] {
            let outcome = provider(script, cursor).infer(&crate::tests::request());
            assert!(
                matches!(outcome,Outcome::Completed {output,evidence} if output==json!({"answer":7}) && evidence["cleanup"]=="confirmed" && evidence.get("secret").is_none() && evidence["token_limit_enforced"]==false)
            );
        }
    }
    #[test]
    fn command_never_retries_or_copies_provider_error_text() {
        let outcome=provider("cat >/dev/null; printf '%s' '{\"version\":\"mas-provider/1\",\"status\":\"unusable\",\"error\":\"credential-secret\",\"evidence\":{\"output_status\":\"refused\"}}'",false).infer(&crate::tests::request());
        assert!(
            matches!(outcome,Outcome::Unusable {error,evidence} if !error.contains("secret") && evidence["output_status"]=="refused")
        );
    }
    #[test]
    fn a_spawned_command_with_no_valid_envelope_is_unknown() {
        assert!(matches!(
            provider("cat >/dev/null; exit 3", false).infer(&crate::tests::request()),
            Outcome::Unknown { .. }
        ));
        assert!(External::new(&json!({"command":[]}), false).is_err());
    }
    #[test]
    fn cursor_raw_output_uses_the_exact_mas_json_parser() {
        for (raw, valid) in [
            ("{\"n\":340282366920938463463374607431768211457}", true),
            ("{\"n\":1,\"n\":2}", false),
            ("{\"n\":1.5}", false),
        ] {
            let envelope = json!({"version":"mas-provider/1","status":"completed","output_json":raw,"evidence":{}});
            let script = format!("cat >/dev/null; printf '%s' '{}'", envelope);
            let outcome = provider(&script, true).infer(&crate::tests::request());
            if valid {
                let expected = mas_protocol_codec::parse_json(raw).unwrap();
                assert!(matches!(outcome,Outcome::Completed{output,..} if output==expected));
            } else {
                assert!(matches!(outcome, Outcome::Unusable { .. }));
            }
        }
        // Duplicate envelope fields are rejected before a JSON value can erase them.
        let script="cat >/dev/null; printf '%s' '{\"version\":\"mas-provider/1\",\"status\":\"unknown\",\"status\":\"completed\",\"output\":{}}'";
        assert!(matches!(
            provider(script, false).infer(&crate::tests::request()),
            Outcome::Unknown { .. }
        ));
    }
}
