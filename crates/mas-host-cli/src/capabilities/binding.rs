//! Bind approved command packages to retained files and the existing dispatch asset gate.
use super::{
    approval::Approval,
    contract::{digest, names, text},
    files,
};
use crate::config::model::{Agent, Implementation};
use mas_host_runtime::command::Argument;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub(super) fn validate_agent(root: &Path, package: &Value, agent: &Agent) -> Result<Value, String> {
    let Implementation::Command(spec) = &agent.implementation else {
        return Err("capability packaging currently requires a command implementation".into());
    };
    if package["operation"] != agent.operation
        || package["agent"] != agent.name
        || package["implementation_sha256"] != digest(&spec.to_value())
    {
        return Err("capability operation or implementation binding mismatch".into());
    }
    if !spec.json_stdin
        || spec.text_stdout
        || spec
            .arguments
            .iter()
            .any(|arg| matches!(arg, Argument::Input(_)))
    {
        return Err("capability requires fixed arguments and JSON stdin/object stdout".into());
    }
    if package["limits"]["timeout_seconds"] != spec.timeout_seconds
        || package["limits"]["max_output_bytes"] != spec.max_output_bytes
    {
        return Err("capability command limits mismatch".into());
    }
    let input = agent
        .input_schema
        .as_ref()
        .ok_or("capability requires an input schema")?;
    let output = agent
        .output_schema
        .as_ref()
        .ok_or("capability requires an output schema")?;
    if package["input_schema_sha256"] != digest(input)
        || package["output_schema_sha256"] != digest(output)
    {
        return Err("capability operation schema binding mismatch".into());
    }
    if names(&package["requires"]["credential_references"])?
        != agent.environment.iter().cloned().collect()
    {
        return Err("capability credential references differ from declared environment".into());
    }
    let directory = Path::new(text(package, "working_directory")?);
    if directory.is_absolute()
        || directory
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
        || root
            .join(directory)
            .canonicalize()
            .map_err(|_| "capability working directory unavailable")?
            != agent.working_directory
    {
        return Err("capability working directory mismatch".into());
    }
    let executable = Path::new(&spec.executable);
    let executable = if executable.is_absolute() {
        executable.to_owned()
    } else {
        files::project_file(root, &spec.executable)?
    };
    files::verify(
        &executable,
        text(package, "executable_sha256")?,
        1024 * 1024 * 1024,
    )?;
    let declared: BTreeMap<_, _> = package["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f["sha256"].as_str().unwrap()))
        .collect();
    for argument in &spec.arguments {
        if let Argument::File(path) = argument {
            if !declared.contains_key(path.as_str()) {
                return Err("capability omits command file argument".into());
            }
        }
    }
    if Path::new(&spec.executable).is_absolute()
        && !package["runtime"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| {
                file["path"] == spec.executable && file["sha256"] == package["executable_sha256"]
            })
    {
        return Err("capability runtime closure omits executable".into());
    }
    files::validate_closure(root, package)?;
    Ok(spec.to_value())
}

pub(super) fn bind(
    root: &Path,
    out: &Path,
    worker: &mut Value,
    package: &Value,
    source_spec: &Value,
    approval: &Approval,
) -> Result<(), String> {
    let working_directory = worker["working_directory"].clone();
    let configuration = worker["configuration"]
        .as_object_mut()
        .ok_or("capability command configuration unavailable")?;
    let mut retained = Vec::new();
    for file in package["files"].as_array().unwrap() {
        let source = files::project_file(root, text(file, "path")?)?;
        let (path, sha256) = crate::build::retain_capability_file(&source, out)?;
        if file["sha256"] != sha256 {
            return Err("capability changed during build".into());
        }
        retained.push(json!({"path":file["path"],"retained_path":path,"sha256":sha256}));
    }
    let executable =
        json!({"path":configuration["executable"],"sha256":package["executable_sha256"]});
    let mut assets = configuration["assets"]
        .as_array()
        .ok_or("capability command assets unavailable")?
        .clone();
    for file in &retained {
        assets.push(json!({"path":file["retained_path"],"sha256":file["sha256"]}));
    }
    assets.extend(package["runtime"]["files"].as_array().unwrap().clone());
    let mut unique = BTreeMap::new();
    for asset in assets {
        let path = text(&asset, "path")?.to_owned();
        let hash = text(&asset, "sha256")?.to_owned();
        if unique
            .insert(path, hash.clone())
            .is_some_and(|previous| previous != hash)
        {
            return Err("capability conflicting asset identity".into());
        }
    }
    configuration.insert(
        "assets".into(),
        json!(unique
            .into_iter()
            .map(|(path, sha256)| json!({"path":path,"sha256":sha256}))
            .collect::<Vec<_>>()),
    );
    configuration.insert(
        "capability".into(),
        json!({"version":"mas-capability-binding/1",
        "declaration":package,"package_sha256":digest(package),"approval_sha256":approval.digest(),
        "source_implementation":source_spec,"working_directory":working_directory,
        "executable":executable,"files":retained}),
    );
    Ok(())
}
