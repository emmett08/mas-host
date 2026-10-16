//! Resolve launch properties without putting provider selection in the host core.
use super::model::Implementation;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) fn resolve(
    root: &Path,
    agent: &Value,
    implementation: &Implementation,
) -> Result<(Vec<String>, PathBuf), String> {
    let mut environment = Vec::new();
    if let Some(value) = agent.get("environment") {
        for name in value
            .as_array()
            .ok_or("environment must be an array of names")?
        {
            environment.push(
                name.as_str()
                    .ok_or("environment names must be strings")?
                    .to_owned(),
            );
        }
    }
    if let Implementation::Llm(configuration) = implementation {
        let connection = &configuration["connection"];
        if let Some(name) = connection["api_key_env"].as_str() {
            environment.push(name.into());
        }
        if let Some(names) = connection["env"].as_array() {
            for name in names {
                environment.push(
                    name.as_str()
                        .ok_or("provider env names must be strings")?
                        .into(),
                );
            }
        }
    }
    let environment = mas_host_process::environment_names(environment.iter().map(String::as_str))?;
    let directory = match agent.get("working_directory") {
        None => root.to_owned(),
        Some(value) => {
            let path = Path::new(
                value
                    .as_str()
                    .ok_or("working_directory must be a relative path")?,
            );
            if path.is_absolute() {
                return Err("working_directory must be project-relative".into());
            }
            root.join(path).canonicalize().map_err(|e| e.to_string())?
        }
    };
    if !directory.starts_with(root) || !directory.is_dir() {
        return Err("working_directory must be a project directory".into());
    }
    Ok((environment, directory))
}
