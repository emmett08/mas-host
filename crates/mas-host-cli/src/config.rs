//! Validate the two application-owned configuration files before running builds.
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};
pub mod authoring;
mod bindings;
mod launch;
pub mod model;
use model::{Agent, Implementation};

pub fn read_json(path: &Path) -> Result<Value, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    file.take(65_537)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    mas_protocol_codec::parse_json(&text)
        .map_err(|e| format!("{}: {}: {}", path.display(), e.code, e.message))
}

/// Resolve one explicit file, or require exactly one supported default file.
pub fn discover(
    project: &Path,
    stem: &str,
    explicit: Option<&Path>,
) -> Result<std::path::PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(if path.is_absolute() {
            path.to_owned()
        } else {
            project.join(path)
        });
    }
    let candidates: Vec<_> = ["toml", "yaml", "yml", "json"]
        .into_iter()
        .map(|extension| project.join(format!("{stem}.{extension}")))
        .filter(|path| path.exists())
        .collect();
    match candidates.as_slice() {
        [path] => Ok(path.to_owned()),
        [] => Err(format!("missing {stem} configuration: expected {stem}.toml, {stem}.yaml, {stem}.yml or {stem}.json")),
        _ => Err(format!("ambiguous {stem} configuration; select one with --{stem}")),
    }
}

pub fn fields(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    label: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label}: expected object"))?;
    for key in required {
        if !object.contains_key(*key) {
            return Err(format!("{label}: missing {key}"));
        }
    }
    for key in object.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(format!("{label}: unknown field {key}"));
        }
    }
    Ok(())
}

fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.as_bytes()[0].is_ascii_alphabetic()
        && text.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

pub fn load_profile(
    project: &Path,
    agents_path: &Path,
    topology_path: &Path,
    profile: Option<&Path>,
) -> Result<(Vec<Agent>, Value), String> {
    let mut agents = authoring::read(agents_path)?;
    if let Some(profile) = profile {
        let profile = authoring::read(profile)?;
        fields(
            &profile,
            &["version", "connections"],
            &[],
            "connection profile",
        )?;
        if profile["version"] != "mas-connections/1" || agents["version"] != "mas-agents/3" {
            return Err("connection profiles require mas-agents/3 and mas-connections/1".into());
        }
        let replacements = profile["connections"]
            .as_object()
            .ok_or("profile connections must be an object")?;
        let connections = agents["connections"]
            .as_object_mut()
            .ok_or("agents connections must be an object")?;
        for (name, connection) in replacements {
            if !connections.contains_key(name) {
                return Err(format!(
                    "connection profile names unknown connection: {name}"
                ));
            }
            connections.insert(name.clone(), connection.clone());
        }
    }
    let topology = authoring::read(topology_path)?;
    validate(project, &agents, &topology)
}

pub fn validate(
    project: &Path,
    config: &Value,
    topology: &Value,
) -> Result<(Vec<Agent>, Value), String> {
    fields(
        config,
        &["version", "agents"],
        &["limits", "connections"],
        "agents configuration",
    )?;
    fields(
        topology,
        &["version", "edges"],
        &[],
        "topology configuration",
    )?;
    if config["version"] != "mas-agents/3" || topology["version"] != "mas-topology/1" {
        return Err("unsupported agents/topology version".into());
    }
    let agents = config["agents"]
        .as_array()
        .ok_or("agents must be an array")?;
    if agents.is_empty() || agents.len() > 31 {
        return Err("expected 1..31 application agents".into());
    }
    let root = project.canonicalize().map_err(|e| e.to_string())?;
    let mut names = BTreeSet::from(["client".to_string()]);
    let mut resolved = Vec::new();
    for agent in agents {
        let name = agent["name"]
            .as_str()
            .ok_or("agent name must be a string")?;
        if !identifier(name) || !names.insert(name.to_string()) {
            return Err(format!("invalid, duplicate or reserved agent name: {name}"));
        }
        let binding = bindings::resolve(&root, agent, config.get("connections"))?;
        resolved.push(binding);
    }
    let agents = resolved;
    for agent in &agents {
        match &agent.implementation {
            Implementation::Workflow(configuration) => {
                mas_host_workflow::validate(&configuration["plan"], &names)?
            }
            Implementation::Llm(configuration) => {
                for delegate in configuration["delegates"].as_array().unwrap() {
                    let target = delegate.as_str().unwrap();
                    if target == agent.name || !names.contains(target) {
                        return Err("invalid LLM delegate".into());
                    }
                }
            }
            _ => {}
        }
    }
    let edges = topology["edges"]
        .as_array()
        .ok_or("edges must be an array")?;
    let mut unique = BTreeSet::new();
    for edge in edges {
        let pair = edge
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("each edge must contain two names")?;
        let from = pair[0].as_str().ok_or("edge source must be a name")?;
        let to = pair[1].as_str().ok_or("edge target must be a name")?;
        if from == to || !names.contains(from) || !names.contains(to) {
            return Err(format!("invalid edge {from} -> {to}"));
        }
        if !unique.insert((from, to)) {
            return Err(format!("duplicate edge {from} -> {to}"));
        }
    }
    for (from, to) in &unique {
        if !unique.contains(&(*to, *from)) {
            return Err(format!("{from} -> {to} needs the reverse reply edge"));
        }
    }
    let mut native = json!({"agents": names, "edges": edges, "application":"agent-task/1"});
    let limits = config.get("limits").cloned().unwrap_or(json!({}));
    let definitions = [
        ("wall_seconds", 60, 3600),
        ("max_calls", 32, 10_000),
        ("max_messages", 1000, 100_000),
        ("max_bytes", 16_777_216, 1_073_741_824),
        ("mailbox_capacity", 32, 100),
    ];
    let allowed: Vec<_> = definitions.iter().map(|(key, _, _)| *key).collect();
    fields(&limits, &[], &allowed, "limits")?;
    for (key, default, maximum) in definitions {
        let value = match limits.get(key) {
            None => default,
            Some(v) => v
                .as_u64()
                .ok_or_else(|| format!("{key}: expected positive integer"))?,
        };
        if value == 0 || value > maximum {
            return Err(format!("{key}: must be 1..{maximum}"));
        }
        native[key] = json!(value);
    }
    Ok((agents, native))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Value {
        json!({"version":"mas-agents/3","agents":[{"name":"worker","implementation":{"kind":"native","language":"go","source":".","handler":"Assess"}}]})
    }
    fn routes() -> Value {
        json!({"version":"mas-topology/1","edges":[["client","worker"],["worker","client"]]})
    }
    #[test]
    fn rejects_implicit_permissions_bad_limits_and_unknown_settings() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(validate(root, &config(), &routes()).is_ok());
        let mut c = config();
        c["limits"] = json!({"max_calls":0});
        assert!(validate(root, &c, &routes())
            .unwrap_err()
            .contains("max_calls"));
        let mut c = config();
        c["python_factory"] = json!("x:y");
        assert!(validate(root, &c, &routes())
            .unwrap_err()
            .contains("unknown"));
        let mut r = routes();
        r["edges"] = json!([["client", "worker"]]);
        assert!(validate(root, &config(), &r)
            .unwrap_err()
            .contains("reply edge"));
        let mut c = config();
        c["agents"][0]["implementation"]["source"] = json!("../..");
        assert!(validate(root, &c, &routes())
            .unwrap_err()
            .contains("escapes"));
    }

    #[test]
    fn requires_current_version_and_explicit_implementation_kind() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut previous_version = config();
        previous_version["version"] = json!("mas-agents/1");
        assert!(validate(root, &previous_version, &routes())
            .unwrap_err()
            .contains("unsupported"));

        let mut flat_native = config();
        flat_native["agents"][0] =
            json!({"name":"worker","language":"go","source":".","handler":"Assess"});
        assert!(validate(root, &flat_native, &routes())
            .unwrap_err()
            .contains("missing implementation"));

        let mut missing_kind = config();
        missing_kind["agents"][0]["implementation"]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        assert!(validate(root, &missing_kind, &routes())
            .unwrap_err()
            .contains("implementation kind"));

        let (resolved, _) = validate(root, &config(), &routes()).unwrap();
        assert_eq!(resolved[0].operation, "worker");
    }

    #[test]
    fn public_operations_are_explicit_mas_identities() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for operation in [json!(7), json!("bad operation"), json!(":bad"), json!("")] {
            let agent = json!({"name":"worker","operation":operation,"implementation":{"kind":"native","language":"go","source":".","handler":"Assess"}});
            assert!(bindings::resolve(root, &agent, None)
                .unwrap_err()
                .contains("operation"));
        }
        let agent = json!({"name":"worker","operation":"assess/1","implementation":{"kind":"native","language":"go","source":".","handler":"Assess"}});
        let resolved = bindings::resolve(root, &agent, None).unwrap();
        assert_eq!(resolved.operation, "assess/1");
        assert!(
            matches!(resolved.implementation, Implementation::Native(native) if native.handler == "Assess")
        );
    }
}
