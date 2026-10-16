//! Generated native worker entrypoints. Application source remains wrapper-free.
mod cpp;
mod python;
mod rust;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;

type Result<T> = std::result::Result<T, String>;
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("agent {name} must be a string"))
}
fn identifier(name: &str) -> bool {
    let mut chars = name.bytes();
    matches!(chars.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn write(path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> Result<()> {
    fs::write(path.as_ref(), content).map_err(|e| format!("{}: {e}", path.as_ref().display()))
}
fn run(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .map_err(|e| format!("cannot run {command:?}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "native application build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Build Rust, C++ and Python agents; the caller combines these records with Go.
/// `sdk_root` is the MAS host distribution root; `agents` is the configuration array.
pub fn build(project: &Path, out: &Path, sdk_root: &Path, agents: &Value) -> Result<Vec<Value>> {
    let project = project.canonicalize().map_err(|e| e.to_string())?;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let sdk_root = sdk_root.canonicalize().map_err(|e| e.to_string())?;
    let mut built = Vec::new();
    for (index, agent) in agents
        .as_array()
        .ok_or("agents must be an array")?
        .iter()
        .enumerate()
    {
        let language = field(agent, "language")?;
        if language == "go" {
            continue;
        }
        if !matches!(language, "rust" | "cpp" | "python") {
            return Err(format!("unsupported language: {language}"));
        }
        let name = field(agent, "name")?;
        let handler = field(agent, "handler")?;
        if !identifier(handler) {
            return Err(format!(
                "{name}: handler must be a single native function identifier"
            ));
        }
        let source = project
            .join(field(agent, "source")?)
            .canonicalize()
            .map_err(|e| format!("{name} source: {e}"))?;
        if !source.starts_with(&project) || !source.is_file() {
            return Err(format!(
                "{name}: source must be a file inside the application directory"
            ));
        }
        let directory = out.join(format!("native-{index}"));
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let executable = directory.join("worker");
        match language {
            "rust" => rust::build(&source, &directory, &executable, &sdk_root, handler, index)?,
            "cpp" => cpp::build(&source, &directory, &executable, &sdk_root, handler)?,
            "python" => python::build(&project, &source, &executable, &sdk_root, handler)?,
            _ => unreachable!(),
        }
        built.push(json!({"name":name,"executable":executable,"handler":handler}));
    }
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_symbols_cannot_inject_source() {
        for name in ["Assess", "assess_invoice", "_handler"] {
            assert!(identifier(name));
        }
        for name in ["", "0entry", "foo::bar", "run();", "a\nb"] {
            assert!(!identifier(name));
        }
    }
}
