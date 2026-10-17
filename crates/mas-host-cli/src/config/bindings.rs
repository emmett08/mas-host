//! Resolve maintained implementation bindings without application-owned launchers.
use super::{
    authoring, fields,
    model::{Agent, Implementation, Native},
    read_json,
};
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
) -> Result<Agent, String> {
    fields(
        agent,
        &["name", "implementation"],
        &[
            "operation",
            "input_schema",
            "output_schema",
            "environment",
            "working_directory",
        ],
        "agent",
    )?;
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
    let selected = match implementation["kind"].as_str() {
        Some("native") => {
            fields(
                implementation,
                &["kind", "language", "source", "handler"],
                &[],
                "native implementation",
            )?;
            validate_native(root, implementation)?;
            Implementation::Native(Native {
                language: implementation["language"].as_str().unwrap().into(),
                source: implementation["source"].as_str().unwrap().into(),
                handler: implementation["handler"].as_str().unwrap().into(),
            })
        }
        Some("workflow") => {
            fields(
                implementation,
                &["kind", "plan"],
                &[],
                "workflow implementation",
            )?;
            Implementation::Workflow(
                json!({"kind":"workflow","plan":authoring::read(&file(root,&implementation["plan"],"workflow plan")?)?}),
            )
        }
        Some("llm") => Implementation::Llm(llm(root, implementation, connections)?),
        Some("command") => {
            fields(
                implementation,
                &["kind", "executable"],
                &[
                    "arguments",
                    "stdin",
                    "stdout",
                    "success_exit_codes",
                    "timeout_seconds",
                    "max_output_bytes",
                ],
                "command implementation",
            )?;
            Implementation::Command(mas_host_runtime::command::Specification::parse(
                implementation,
            )?)
        }
        _ => return Err("implementation kind must be native, llm, workflow or command".into()),
    };
    let input_schema = schema(root, agent.get("input_schema"))?;
    let output_schema = schema(root, agent.get("output_schema"))?;
    if matches!(
        selected,
        Implementation::Llm(_) | Implementation::Command(_)
    ) && output_schema.is_none()
    {
        return Err("LLM and command operations require an output_schema".into());
    }
    let (environment, directory) = super::launch::resolve(root, agent, &selected)?;
    Ok(Agent {
        name: name.into(),
        operation: operation.into(),
        implementation: selected,
        input_schema,
        output_schema,
        environment,
        working_directory: directory,
    })
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
        &["kind", "connection", "model", "instructions"],
        &[
            "max_output_tokens",
            "max_turns",
            "delegates",
            "min_delegate_calls",
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
    let mut config =
        json!({"kind":"llm","connection":connection,"model":model,"instructions":instructions});
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
    if let Some(value) = implementation.get("min_delegate_calls") {
        let minimum = value
            .as_u64()
            .ok_or("min_delegate_calls must be a non-negative integer")?;
        if minimum >= config["max_turns"].as_u64().unwrap() {
            return Err("min_delegate_calls must be less than max_turns".into());
        }
        if minimum > 0 && list.is_empty() {
            return Err("min_delegate_calls requires at least one delegate".into());
        }
        // Omission must retain the previous resolved configuration bytes and
        // therefore its identity in existing bundles and effect descriptors.
        config["min_delegate_calls"] = json!(minimum);
    }
    config["delegates"] = delegates;
    Ok(config)
}

fn schema(root: &Path, path: Option<&Value>) -> Result<Option<Value>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let schema = read_json(&file(root, path, "operation schema")?)?;
    mas_host_schema::check(&schema)?;
    if schema["type"] != "object" {
        return Err("operation schemas must have object type".into());
    }
    Ok(Some(schema))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_delegate_calls_is_optional_bounded_and_requires_delegates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let implementation = json!({"kind":"llm","connection":"fixture","model":"fixture",
            "instructions":"README.md","max_turns":3,"delegates":["peer"]});
        let connections = json!({"fixture":{"kind":"mock","output":{}}});
        let resolved = llm(&root, &implementation, Some(&connections)).unwrap();
        let previous = json!({"kind":"llm","connection":connections["fixture"],"model":"fixture",
            "instructions":std::fs::read_to_string(root.join("README.md")).unwrap(),
            "max_output_tokens":4096,"max_turns":3,"delegates":["peer"]});
        assert_eq!(resolved.to_string(), previous.to_string());
        for minimum in [json!(0), json!(1), json!(2)] {
            let mut current = implementation.clone();
            current["min_delegate_calls"] = minimum.clone();
            assert_eq!(
                llm(&root, &current, Some(&connections)).unwrap()["min_delegate_calls"],
                minimum
            );
        }
        for minimum in [
            json!(-1),
            json!(1.5),
            json!(true),
            json!("1"),
            json!(null),
            json!(3),
        ] {
            let mut current = implementation.clone();
            current["min_delegate_calls"] = minimum;
            assert!(llm(&root, &current, Some(&connections))
                .unwrap_err()
                .contains("min_delegate_calls"));
        }
        let mut without_delegates = implementation;
        without_delegates["delegates"] = json!([]);
        without_delegates["min_delegate_calls"] = json!(1);
        assert!(llm(&root, &without_delegates, Some(&connections))
            .unwrap_err()
            .contains("at least one delegate"));
        without_delegates["min_delegate_calls"] = json!(0);
        assert!(llm(&root, &without_delegates, Some(&connections)).is_ok());
    }
}
