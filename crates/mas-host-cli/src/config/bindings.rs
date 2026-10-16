//! Resolve maintained implementation bindings without application-owned launchers.
use super::{authoring, fields, read_json};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

fn file(root: &Path, value: &Value, label: &str) -> Result<PathBuf, String> {
    let relative = Path::new(
        value
            .as_str()
            .ok_or_else(|| format!("{label} must be a relative file path"))?,
    );
    if relative.is_absolute() {
        return Err(format!("{label} must be relative"));
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("{label}: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("{label} must be a project file"));
    }
    Ok(path)
}

pub(super) fn resolve(
    root: &Path,
    agent: &Value,
    connections: Option<&Value>,
) -> Result<Value, String> {
    fields(agent, &["name", "implementation"], &["operation"], "agent")?;
    let name = agent["name"].as_str().ok_or("agent name required")?;
    let operation = match agent.get("operation") {
        Some(value) => value.as_str().ok_or("agent operation must be a string")?,
        None => name,
    };
    if operation.is_empty()
        || operation.len() > 128
        || !operation.as_bytes()[0].is_ascii_alphanumeric()
        || !operation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
    {
        return Err("invalid agent operation".into());
    }
    let implementation = &agent["implementation"];
    let configuration = match implementation["kind"].as_str() {
        Some("native") => {
            fields(
                implementation,
                &["kind", "language", "source", "handler"],
                &[],
                "native implementation",
            )?;
            return Ok(
                json!({"name":name,"language":implementation["language"],"source":implementation["source"],"handler":implementation["handler"],"operation":operation}),
            );
        }
        Some("workflow") => {
            fields(
                implementation,
                &["kind", "plan"],
                &[],
                "workflow implementation",
            )?;
            json!({"kind":"workflow","plan":authoring::read(&file(root,&implementation["plan"],"workflow plan")?)?})
        }
        Some("llm") => llm(root, implementation, connections)?,
        _ => return Err("implementation kind must be native, llm or workflow".into()),
    };
    Ok(json!({"name":name,"handler":operation,"configuration":configuration}))
}

pub(super) fn validate_native(root: &Path, binding: &Value) -> Result<(), String> {
    let handler = binding["handler"]
        .as_str()
        .ok_or("native handler must be a string")?;
    if !super::identifier(handler) {
        return Err("native handler must be a simple function identifier".into());
    }
    let source = binding["source"]
        .as_str()
        .ok_or("native source must be relative")?;
    if Path::new(source).is_absolute() {
        return Err("native source must be relative".into());
    }
    let source = root
        .join(source)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !source.starts_with(root) {
        return Err("native source escapes project".into());
    }
    match binding["language"].as_str() {
        Some("go") if source.is_dir() => Ok(()),
        Some("rust" | "cpp" | "python") if source.is_file() => Ok(()),
        _ => Err("unsupported native language/source profile".into()),
    }
}

fn llm(root: &Path, implementation: &Value, connections: Option<&Value>) -> Result<Value, String> {
    fields(
        implementation,
        &[
            "kind",
            "connection",
            "model",
            "instructions",
            "output_schema",
        ],
        &[
            "input_schema",
            "max_output_tokens",
            "max_turns",
            "delegates",
        ],
        "LLM implementation",
    )?;
    let connection_name = implementation["connection"]
        .as_str()
        .ok_or("connection must be a name")?;
    let mut connection = connections
        .and_then(Value::as_object)
        .and_then(|c| c.get(connection_name))
        .ok_or("unknown provider connection")?
        .clone();
    let override_model = connection
        .as_object_mut()
        .ok_or("connection must be an object")?
        .remove("model");
    if connection.get("api_key_env").is_none() {
        if connection["kind"] == "openai" {
            connection["api_key_env"] = json!("OPENAI_API_KEY");
        } else if connection["kind"] == "cursor" {
            connection["api_key_env"] = json!("CURSOR_API_KEY");
        }
    }
    mas_host_providers::create(&connection)?;
    let model = override_model
        .as_ref()
        .unwrap_or(&implementation["model"])
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 200)
        .ok_or("model must be explicit")?;
    let mut instructions = String::new();
    std::fs::File::open(file(root, &implementation["instructions"], "instructions")?)
        .map_err(|e| e.to_string())?
        .take(16385)
        .read_to_string(&mut instructions)
        .map_err(|e| e.to_string())?;
    if instructions.is_empty() || instructions.len() > 16384 {
        return Err("instructions require 1..16384 bytes".into());
    }
    let output_schema = read_json(&file(
        root,
        &implementation["output_schema"],
        "output schema",
    )?)?;
    mas_host_runtime::schema::check(&output_schema)?;
    if output_schema["type"] != "object" {
        return Err("operation output schema must have object type".into());
    }
    let mut config = json!({"kind":"llm","connection":connection,"model":model,"instructions":instructions,"output_schema":output_schema});
    if let Some(schema) = implementation.get("input_schema") {
        let schema = read_json(&file(root, schema, "input schema")?)?;
        mas_host_runtime::schema::check(&schema)?;
        config["input_schema"] = schema;
    }
    for (name, default, max) in [("max_output_tokens", 4096, 131072), ("max_turns", 1, 1024)] {
        let value = implementation
            .get(name)
            .map(|v| v.as_u64().ok_or("LLM limits must be positive integers"))
            .transpose()?
            .unwrap_or(default);
        if value == 0 || value > max {
            return Err(format!("{name} must be 1..{max}"));
        }
        config[name] = json!(value);
    }
    let delegates = implementation
        .get("delegates")
        .cloned()
        .unwrap_or(json!([]));
    let list = delegates.as_array().ok_or("delegates must be an array")?;
    let mut names = std::collections::BTreeSet::new();
    for name in list {
        if !names.insert(name.as_str().ok_or("delegate must be a name")?) {
            return Err("duplicate delegate".into());
        }
    }
    config["delegates"] = delegates;
    Ok(config)
}
