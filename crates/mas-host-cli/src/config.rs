//! Validate the two application-owned configuration files before running builds.
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};

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

pub fn load(project: &Path) -> Result<(Value, Value), String> {
    let agents = read_json(&project.join("agents.json"))?;
    let topology = read_json(&project.join("topology.json"))?;
    validate(project, &agents, &topology)
}

pub fn validate(
    project: &Path,
    config: &Value,
    topology: &Value,
) -> Result<(Value, Value), String> {
    fields(config, &["version", "agents"], &["limits"], "agents.json")?;
    fields(topology, &["version", "edges"], &[], "topology.json")?;
    if config["version"] != "mas-agents/1" || topology["version"] != "mas-topology/1" {
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
    for agent in agents {
        fields(
            agent,
            &["name", "language", "source", "handler"],
            &[],
            "agent",
        )?;
        let name = agent["name"]
            .as_str()
            .ok_or("agent name must be a string")?;
        let handler = agent["handler"]
            .as_str()
            .ok_or("handler must be a string")?;
        if !identifier(name) || !names.insert(name.to_string()) {
            return Err(format!("invalid, duplicate or reserved agent name: {name}"));
        }
        if !identifier(handler) {
            return Err(format!(
                "{name}: handler must be a simple function identifier"
            ));
        }
        let language = agent["language"]
            .as_str()
            .ok_or("language must be a string")?;
        if !matches!(language, "go" | "rust" | "cpp") {
            return Err(format!("{name}: unsupported language {language}"));
        }
        let source = Path::new(
            agent["source"]
                .as_str()
                .ok_or("source must be a relative path")?,
        );
        if source.is_absolute() {
            return Err(format!("{name}: source must be relative to the project"));
        }
        let resolved = root
            .join(source)
            .canonicalize()
            .map_err(|e| format!("{name}: source: {e}"))?;
        if !resolved.starts_with(&root) {
            return Err(format!("{name}: source escapes the application directory"));
        }
        if language == "go" && !resolved.is_dir() {
            return Err(format!("{name}: Go source must name a package directory"));
        }
        if language != "go" && !resolved.is_file() {
            return Err(format!("{name}: native source must name a file"));
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
    Ok((config["agents"].clone(), native))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Value {
        json!({"version":"mas-agents/1","agents":[{"name":"worker","language":"go","source":".","handler":"Assess"}]})
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
        c["agents"][0]["source"] = json!("../..");
        assert!(validate(root, &c, &routes())
            .unwrap_err()
            .contains("escapes"));
    }
}
