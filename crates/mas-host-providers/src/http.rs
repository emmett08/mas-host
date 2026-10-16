use crate::{
    environment_names, evidence, not_sent, process, retain_metadata, valid_env, validate, Outcome,
    Provider, Request,
};
use serde_json::{json, Value};
use std::process::Command;

#[derive(Clone, Copy)]
pub(crate) enum Api {
    Responses,
    Chat,
}
pub(crate) struct Http {
    api: Api,
    endpoint: String,
    key_env: Option<String>,
    env: Vec<String>,
    structured: bool,
}
impl Http {
    pub fn new(config: &Value, api: Api) -> Result<Self, String> {
        let endpoint = match config.get("endpoint") {
            Some(value) => value.as_str().ok_or("provider endpoint must be a string")?,
            None if matches!(api, Api::Responses) => "https://api.openai.com/v1/responses",
            None => {
                return Err("OpenAI-compatible provider requires its complete endpoint URL".into())
            }
        };
        validate_endpoint(endpoint)?;
        let key_env = match config.get("api_key_env") {
            Some(value) => {
                let name = value
                    .as_str()
                    .ok_or("api_key_env must be an environment name")?;
                if !valid_env(name) {
                    return Err("invalid api_key_env".into());
                }
                Some(name.into())
            }
            None if matches!(api, Api::Responses) => Some("OPENAI_API_KEY".into()),
            None => None,
        };
        let structured = match config.get("structured_outputs") {
            Some(value) => value
                .as_bool()
                .ok_or("structured_outputs must be boolean")?,
            None => true,
        };
        Ok(Self {
            api,
            endpoint: endpoint.into(),
            key_env,
            env: environment_names(config)?,
            structured,
        })
    }
    fn kind(&self) -> &str {
        match self.api {
            Api::Responses => "openai",
            Api::Chat => "openai-compatible",
        }
    }
    fn body(&self, request: &Request) -> Value {
        match self.api {
            Api::Responses => json!({"model":request.model,"instructions":request.instructions,
                "input":serde_json::to_string(&request.input).unwrap(),"max_output_tokens":request.max_output_tokens,
                "store":false,"tools":[],"text":{"format":{"type":"json_schema","name":"mas_output",
                    "schema":request.output_schema,"strict":true}}}),
            Api::Chat => {
                let format = if self.structured {
                    json!({"type":"json_schema","json_schema":{"name":"mas_output","schema":request.output_schema,"strict":true}})
                } else {
                    json!({"type":"json_object"})
                };
                json!({"model":request.model,"messages":[{"role":"system","content":format!("{}\nReturn only a JSON value matching this JSON Schema: {}",request.instructions,request.output_schema)},
                    {"role":"user","content":serde_json::to_string(&request.input).unwrap()}],
                    "max_tokens":request.max_output_tokens,"response_format":format,"stream":false})
            }
        }
    }
}
impl Provider for Http {
    fn infer(&self, request: &Request) -> Outcome {
        if let Err(error) = validate(request) {
            return not_sent(&error, self.kind());
        }
        let key = match &self.key_env {
            Some(name) => match std::env::var(name) {
                Ok(value) if !value.is_empty() && !value.contains(['\r', '\n', '\0']) => {
                    Some(value)
                }
                _ => {
                    return not_sent(
                        "provider credential environment is missing or invalid",
                        self.kind(),
                    )
                }
            },
            None => None,
        };
        let duration = process::remaining(request.deadline_ms).unwrap();
        // curl's private config arrives over stdin: the key and prompt never
        // appear in argv, temporary files, curl stderr or the MAS evidence.
        let quote = |value: &str| serde_json::to_string(value).unwrap();
        let mut input=format!("url = {}\nrequest = \"POST\"\nheader = \"Content-Type: application/json\"\nheader = {}\ndata-binary = {}\n",
            quote(&self.endpoint),quote(&format!("X-Client-Request-Id: {}",request.request_id)),quote(&self.body(request).to_string()));
        if let Some(key) = key {
            input.push_str(&format!(
                "header = {}\n",
                quote(&format!("Authorization: Bearer {key}"))
            ));
        }
        let timeout = format!("{}.{:03}", duration.as_secs(), duration.subsec_millis());
        let mut command = Command::new("curl");
        command.args([
            "--disable",
            "--silent",
            "--show-error",
            "--config",
            "-",
            "--max-time",
            &timeout,
            "--connect-timeout",
            &timeout,
            "--retry",
            "0",
            "--max-redirs",
            "0",
            "--max-filesize",
            "1048570",
            "--write-out",
            "\n%{http_code}",
        ]);
        process::scoped_environment(&mut command, &self.env);
        let result = match process::run(command, input.into_bytes(), request.deadline_ms) {
            Ok(output) => output,
            Err(process::Failure::NotStarted) => {
                return not_sent("cannot start HTTP provider transport", self.kind())
            }
            Err(process::Failure::Incomplete) => {
                return Outcome::Unknown {
                    error: "HTTP provider transport did not complete within its limits".into(),
                    evidence: evidence(self.kind(), request, "unavailable", true),
                }
            }
        };
        let mut ev = evidence(self.kind(), request, "unavailable", true);
        if !result.status.success() {
            return Outcome::Unknown {
                error: "HTTP provider transport failed; external outcome may be unknown".into(),
                evidence: ev,
            };
        }
        let Some(split) = result.stdout.iter().rposition(|byte| *byte == b'\n') else {
            return Outcome::Unknown {
                error: "HTTP provider status is unavailable".into(),
                evidence: ev,
            };
        };
        let status = std::str::from_utf8(&result.stdout[split + 1..])
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        ev["http_status"] = json!(status);
        if status >= 500 || status == 0 {
            return Outcome::Unknown {
                error: "provider server did not establish an inference outcome".into(),
                evidence: ev,
            };
        }
        if !(200..300).contains(&status) {
            ev["output_status"] = json!("rejected");
            ev["execution"] = json!("rejected");
            crate::http_error::retain(&mut ev, &result.stdout[..split]);
            return Outcome::Unusable {
                error: "provider rejected the request".into(),
                evidence: ev,
            };
        }
        let response: Value = match serde_json::from_slice(&result.stdout[..split]) {
            Ok(value) => value,
            Err(_) => {
                ev["output_status"] = json!("invalid");
                return Outcome::Unusable {
                    error: "provider returned invalid JSON".into(),
                    evidence: ev,
                };
            }
        };
        retain_metadata(&mut ev, &response);
        parse_response(self.api, response, ev)
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.len() > 2048
        || endpoint
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("invalid provider endpoint".into());
    }
    let rest = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .ok_or("provider endpoint must use HTTPS (HTTP is allowed only on loopback)")?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty()
        || authority.contains(['@', '?', '#', '\\'])
        || rest.contains(['?', '#', '\\'])
    {
        return Err("provider endpoint must contain no credentials, query or fragment".into());
    }
    let loopback = authority == "localhost"
        || authority.starts_with("localhost:")
        || authority == "127.0.0.1"
        || authority.starts_with("127.0.0.1:")
        || authority == "[::1]"
        || authority.starts_with("[::1]:");
    if endpoint.starts_with("http://") && !loopback {
        return Err("unencrypted HTTP provider endpoint must be loopback".into());
    }
    Ok(())
}

fn parse_response(api: Api, response: Value, mut ev: Value) -> Outcome {
    let unusable = |error: &str, ev: Value| Outcome::Unusable {
        error: error.into(),
        evidence: ev,
    };
    let text = match api {
        Api::Responses => {
            if response["status"] != "completed" {
                ev["output_status"] = json!("incomplete");
                return if matches!(
                    response["status"].as_str(),
                    Some("incomplete" | "failed" | "cancelled")
                ) {
                    unusable("provider response did not complete", ev)
                } else {
                    Outcome::Unknown {
                        error: "provider response has no terminal inference outcome".into(),
                        evidence: ev,
                    }
                };
            }
            let mut text = String::new();
            if let Some(output) = response["output"].as_array() {
                for item in output {
                    if let Some(content) = item["content"].as_array() {
                        for block in content {
                            if block["type"] == "refusal" {
                                ev["output_status"] = json!("refused");
                                return unusable("provider refused the request", ev);
                            }
                            if block["type"] == "output_text" {
                                if let Some(value) = block["text"].as_str() {
                                    text.push_str(value);
                                }
                            }
                        }
                    }
                }
            }
            text
        }
        Api::Chat => {
            let choice = &response["choices"][0];
            if !choice["message"]["refusal"].is_null() {
                ev["output_status"] = json!("refused");
                return unusable("provider refused the request", ev);
            }
            if choice["finish_reason"] != "stop" {
                ev["output_status"] = json!("incomplete");
                return unusable("provider response did not complete", ev);
            }
            choice["message"]["content"].as_str().unwrap_or("").into()
        }
    };
    match mas_protocol_codec::parse_json(&text) {
        Ok(output) => {
            ev["output_status"] = json!("json");
            Outcome::Completed {
                output,
                evidence: ev,
            }
        }
        Err(_) => {
            ev["output_status"] = json!("invalid");
            unusable("provider output is not JSON", ev)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responses_distinguish_refusal_incomplete_and_invalid_output() {
        let body =
            |block| json!({"status":"completed","output":[{"type":"message","content":[block]}]});
        assert!(
            matches!(parse_response(Api::Responses,body(json!({"type":"refusal","refusal":"no"})),json!({})),Outcome::Unusable{evidence,..} if evidence["output_status"]=="refused")
        );
        assert!(
            matches!(parse_response(Api::Responses,json!({"status":"incomplete"}),json!({})),Outcome::Unusable{evidence,..} if evidence["output_status"]=="incomplete")
        );
        assert!(matches!(
            parse_response(Api::Responses, json!({"status":"in_progress"}), json!({})),
            Outcome::Unknown { .. }
        ));
        assert!(
            matches!(parse_response(Api::Responses,body(json!({"type":"output_text","text":"not json"})),json!({})),Outcome::Unusable{evidence,..} if evidence["output_status"]=="invalid")
        );
    }
    #[test]
    fn chat_finish_reason_and_json_are_required() {
        let response =
            json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"ok\":true}"}}]});
        assert!(
            matches!(parse_response(Api::Chat,response,json!({})),Outcome::Completed {output,..} if output==json!({"ok":true}))
        );
        assert!(matches!(
            parse_response(
                Api::Chat,
                json!({"choices":[{"finish_reason":"length"}]}),
                json!({})
            ),
            Outcome::Unusable { .. }
        ));
    }
    #[test]
    fn model_json_retains_exact_integers_and_rejects_duplicate_or_decimal_syntax() {
        let response = |text: &str| json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":text}]}]});
        for invalid in ["{\"n\":1,\"n\":2}", "{\"n\":1.5}", "{\"n\":1e3}"] {
            assert!(matches!(
                parse_response(Api::Responses, response(invalid), json!({})),
                Outcome::Unusable { .. }
            ));
        }
        let raw = "{\"n\":340282366920938463463374607431768211457}";
        let expected = mas_protocol_codec::parse_json(raw).unwrap();
        assert!(
            matches!(parse_response(Api::Responses,response(raw),json!({})),Outcome::Completed{output,..} if output==expected)
        );
    }
    #[test]
    fn endpoint_validation_rejects_credential_urls_and_remote_plaintext() {
        for endpoint in [
            "file:///tmp/key",
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=x",
            "https://example.com\n",
        ] {
            assert!(validate_endpoint(endpoint).is_err());
        }
        assert!(validate_endpoint("http://127.0.0.1:3333/v1/chat/completions").is_ok());
    }
    #[test]
    fn loopback_transport_uses_responses_contract_and_records_usage() {
        let body=json!({"id":"response-1","model":"resolved-model","status":"completed","usage":{"input_tokens":8,"output_tokens":5},"output":[{"type":"message","content":[{"type":"output_text","text":"{\"answer\":13}"}]}]}).to_string();
        let (outcome, request) = infer_loopback(Api::Responses, 200, body);
        assert_eq!(request["tools"], json!([]));
        assert_eq!(request["store"], false);
        assert_eq!(request["text"]["format"]["type"], "json_schema");
        assert!(
            matches!(outcome,Outcome::Completed{output,evidence} if output==json!({"answer":13}) && evidence["usage"]["input_tokens"]==8 && evidence["model"]=="resolved-model")
        );
    }
    #[test]
    fn loopback_rejections_retain_safe_schema_authentication_and_quota_identifiers() {
        for (api, status, code, kind, param) in [
            (
                Api::Responses,
                400,
                "invalid_json_schema",
                "invalid_request_error",
                Some("text.format.schema"),
            ),
            (
                Api::Chat,
                400,
                "invalid_json_schema",
                "invalid_request_error",
                Some("response_format.json_schema.schema"),
            ),
            (
                Api::Responses,
                401,
                "invalid_api_key",
                "invalid_request_error",
                None,
            ),
            (
                Api::Responses,
                403,
                "permission_denied",
                "permission_error",
                None,
            ),
            (
                Api::Responses,
                404,
                "model_not_found",
                "invalid_request_error",
                Some("model"),
            ),
            (
                Api::Responses,
                429,
                "insufficient_quota",
                "insufficient_quota",
                None,
            ),
            (
                Api::Responses,
                429,
                "rate_limit_exceeded",
                "rate_limit_error",
                None,
            ),
        ] {
            let body = json!({"error": {
                "code": code, "type": kind, "param": param,
                "message": "sk-secret-key: private prompt; rejected schema contents"
            }})
            .to_string();
            let (outcome, _) = infer_loopback(api, status, body);
            let Outcome::Unusable { error, evidence } = outcome else {
                panic!("HTTP rejection classification changed: {outcome:?}");
            };
            assert_eq!(error, "provider rejected the request");
            assert_eq!(evidence["http_status"], status);
            assert_eq!(evidence["execution"], "rejected");
            assert_eq!(evidence["output_status"], "rejected");
            assert_eq!(evidence["hidden_retries"], false);
            assert_eq!(evidence["error_code"], code);
            assert_eq!(evidence["error_type"], kind);
            assert_eq!(evidence.get("error_param").and_then(Value::as_str), param);
            let retained = evidence.to_string();
            assert!(!retained.contains("sk-secret-key"));
            assert!(!retained.contains("private prompt"));
            assert!(!retained.contains("schema contents"));
        }
    }
    #[test]
    fn loopback_rejections_omit_arbitrary_strings_even_in_diagnostic_fields() {
        for secret in [
            "sk-secret-key",
            "private prompt",
            "text.format.schema.properties.secret",
        ] {
            let body = json!({
                "id": secret, "model": secret, "headers": {"authorization": secret},
                "error": {"code": secret, "type": secret, "param": secret, "message": secret}
            })
            .to_string();
            let (outcome, _) = infer_loopback(Api::Responses, 400, body);
            let Outcome::Unusable { evidence, .. } = outcome else {
                panic!("HTTP rejection classification changed: {outcome:?}");
            };
            for field in [
                "error_code",
                "error_type",
                "error_param",
                "id",
                "model",
                "headers",
            ] {
                assert!(evidence.get(field).is_none());
            }
            assert!(!evidence.to_string().contains(secret));
        }
    }
    #[test]
    fn loopback_server_failure_remains_unknown() {
        let body = json!({"error": {
            "code": "invalid_api_key", "type": "authentication_error", "param": "model"
        }})
        .to_string();
        let (outcome, _) = infer_loopback(Api::Responses, 503, body);
        assert!(matches!(outcome, Outcome::Unknown { evidence, .. }
            if evidence["http_status"] == 503
                && evidence.get("execution").is_none()
                && evidence.get("error_code").is_none()));
    }

    fn infer_loopback(api: Api, status: u16, body: String) -> (Outcome, Value) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::mpsc,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let (completed, wait_for_completion) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let request = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..split]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    let request: Value =
                        serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap();
                    break request;
                }
            };
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            drop(stream);
            wait_for_completion.recv().unwrap();
            listener.set_nonblocking(true).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "provider issued an additional HTTP request"
            );
            request
        });
        // No key is required for compatible local servers. Exercise Responses
        // through the same transport with the private test configuration.
        let provider = Http {
            api,
            endpoint,
            key_env: None,
            env: Vec::new(),
            structured: true,
        };
        let outcome = provider.infer(&crate::tests::request());
        completed.send(()).unwrap();
        (outcome, server.join().unwrap())
    }
}
