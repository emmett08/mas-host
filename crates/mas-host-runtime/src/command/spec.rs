//! Validated command arguments and explicit stdin/stdout conversion.
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub enum Argument {
    Literal(String),
    Input(String),
    File(String),
    InvocationId,
}

#[derive(Clone, Debug)]
pub struct Specification {
    pub executable: String,
    pub arguments: Vec<Argument>,
    pub json_stdin: bool,
    pub text_stdout: bool,
    pub success_exit_codes: Vec<i32>,
    pub timeout_seconds: u64,
    pub max_output_bytes: usize,
}

fn text(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| s.len() <= 4096 && !s.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(|| "command argument must be a bounded string without NUL".into())
}

fn pointer(value: &str) -> bool {
    if !value.is_empty() && !value.starts_with('/') {
        return false;
    }
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

impl Specification {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let executable = text(&value["executable"])?;
        if executable.is_empty() {
            return Err("command executable must be nonempty".into());
        }
        let mut arguments = Vec::new();
        if let Some(values) = value.get("arguments") {
            for item in values
                .as_array()
                .filter(|a| a.len() <= 64)
                .ok_or("command arguments must be an array of at most 64 bindings")?
            {
                let object = item
                    .as_object()
                    .filter(|o| o.len() == 1)
                    .ok_or("each command argument needs exactly one binding")?;
                let (kind, value) = object.iter().next().unwrap();
                arguments.push(match kind.as_str() {
                    "literal" => Argument::Literal(text(value)?),
                    "file" => Argument::File(text(value)?),
                    "input" => {
                        let path = text(value)?;
                        if !pointer(&path) {
                            return Err("command input binding must be a JSON Pointer".into());
                        }
                        Argument::Input(path)
                    }
                    "invocation_id" if value == true => Argument::InvocationId,
                    _ => return Err("unknown command argument binding".into()),
                });
            }
        }
        let json_stdin = match value.get("stdin").and_then(Value::as_str) {
            Some("json") => true,
            Some("none") => false,
            None if value.get("stdin").is_none() => false,
            _ => return Err("command stdin must be none or json".into()),
        };
        let text_stdout = match value.get("stdout").and_then(Value::as_str) {
            Some("text") => true,
            Some("json-object") => false,
            None if value.get("stdout").is_none() => false,
            _ => return Err("command stdout must be json-object or text".into()),
        };
        let codes = value
            .get("success_exit_codes")
            .cloned()
            .unwrap_or(json!([0]));
        let mut success_exit_codes = Vec::new();
        for code in codes
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 32)
            .ok_or("success_exit_codes needs 1..32 exit codes")?
        {
            let code = code
                .as_u64()
                .filter(|n| *n <= 255)
                .ok_or("exit codes must be integers in 0..255")? as i32;
            if success_exit_codes.contains(&code) {
                return Err("duplicate success exit code".into());
            }
            success_exit_codes.push(code);
        }
        success_exit_codes.sort();
        let timeout_seconds = limit(value, "timeout_seconds", 60, 3600)?;
        let max_output_bytes = limit(value, "max_output_bytes", 32768, 49152)? as usize;
        Ok(Self {
            executable,
            arguments,
            json_stdin,
            text_stdout,
            success_exit_codes,
            timeout_seconds,
            max_output_bytes,
        })
    }

    pub fn to_value(&self) -> Value {
        let arguments: Vec<_> = self
            .arguments
            .iter()
            .map(|arg| match arg {
                Argument::Literal(s) => json!({"literal":s}),
                Argument::Input(s) => json!({"input":s}),
                Argument::File(s) => json!({"file":s}),
                Argument::InvocationId => json!({"invocation_id":true}),
            })
            .collect();
        json!({"kind":"command","executable":self.executable,"arguments":arguments,
            "stdin":if self.json_stdin {"json"} else {"none"},
            "stdout":if self.text_stdout {"text"} else {"json-object"},
            "success_exit_codes":self.success_exit_codes,"timeout_seconds":self.timeout_seconds,
            "max_output_bytes":self.max_output_bytes})
    }

    pub fn render(&self, input: &Value, invocation_id: &str) -> Result<Vec<String>, String> {
        self.arguments
            .iter()
            .map(|arg| match arg {
                Argument::Literal(s) | Argument::File(s) => Ok(s.clone()),
                Argument::InvocationId => Ok(invocation_id.to_owned()),
                Argument::Input(pointer) => {
                    let value = input
                        .pointer(pointer)
                        .ok_or_else(|| format!("command input missing at {pointer}"))?;
                    match value {
                        Value::String(_) => text(value),
                        Value::Number(_) | Value::Bool(_) => text(&json!(value.to_string())),
                        _ => Err(
                            "command input argument must be a string, integer or boolean".into(),
                        ),
                    }
                }
            })
            .collect()
    }
}

fn limit(value: &Value, name: &str, default: u64, maximum: u64) -> Result<u64, String> {
    match value.get(name) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n > 0 && *n <= maximum)
            .ok_or_else(|| format!("{name} must be 1..{maximum}")),
    }
}
