//! Verify retained packages preserve the approved command and maintained dispatch closure.
use super::{
    approval::Approval,
    contract::{declaration, digest, names, text},
    files,
};
use crate::config::fields;
use mas_host_runtime::command::{Argument, Specification};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub(super) fn validate_worker(
    worker: &Value,
    approval: &Approval,
    runner_sha256: &str,
) -> Result<(), String> {
    let configuration = &worker["configuration"];
    fields(
        configuration,
        &[
            "kind",
            "executable",
            "arguments",
            "stdin",
            "stdout",
            "success_exit_codes",
            "timeout_seconds",
            "max_output_bytes",
            "input_schema",
            "output_schema",
            "assets",
            "capability",
        ],
        &["connection"],
        "packaged command configuration",
    )?;
    // Existing provider-asset binding leaves this null entry on command
    // configurations. Preserve its representation while rejecting a provider.
    if configuration
        .get("connection")
        .is_some_and(|value| !value.is_null())
    {
        return Err("packaged command configuration has unexpected connection".into());
    }
    if configuration["input_schema"] != worker["input_schema"]
        || configuration["output_schema"] != worker["output_schema"]
    {
        return Err("capability retained schema mismatch".into());
    }
    if configuration["kind"] != "command"
        || worker["arguments"] != json!(["__builtin"])
        || worker.get("handler").is_some()
        || worker["sha256"] != runner_sha256
    {
        return Err("capability requires the current maintained command runner".into());
    }
    let binding = &configuration["capability"];
    fields(
        binding,
        &[
            "version",
            "declaration",
            "package_sha256",
            "approval_sha256",
            "source_implementation",
            "working_directory",
            "executable",
            "files",
        ],
        &[],
        "capability retained binding",
    )?;
    if binding["version"] != "mas-capability-binding/1" {
        return Err("unsupported capability binding".into());
    }
    super::contract::hash(binding, "approval_sha256")?;
    fields(
        &binding["executable"],
        &["path", "sha256"],
        &[],
        "capability executable",
    )?;
    let package = &binding["declaration"];
    fields(
        &binding["source_implementation"],
        &[
            "kind",
            "executable",
            "arguments",
            "stdin",
            "stdout",
            "success_exit_codes",
            "timeout_seconds",
            "max_output_bytes",
        ],
        &[],
        "capability source command",
    )?;
    if binding["source_implementation"]["kind"] != "command" {
        return Err("capability source implementation must be command".into());
    }
    declaration(package)?;
    approval.check(package)?;
    if binding["package_sha256"] != digest(package)
        || binding["executable"]["sha256"] != package["executable_sha256"]
        || package["agent"] != worker["name"]
        || package["operation"] != worker["operation"]
        || package["input_schema_sha256"] != digest(&worker["input_schema"])
        || package["output_schema_sha256"] != digest(&worker["output_schema"])
        || package["implementation_sha256"] != digest(&binding["source_implementation"])
        || binding["working_directory"] != worker["working_directory"]
        || names(&package["requires"]["credential_references"])? != names(&worker["environment"])?
    {
        return Err("capability retained contract mismatch".into());
    }
    let mut expected = Specification::parse(&binding["source_implementation"])?;
    let source_executable = expected.executable.clone();
    expected.executable = text(&binding["executable"], "path")?.into();
    files::verify(
        Path::new(&expected.executable),
        text(package, "executable_sha256")?,
        1024 * 1024 * 1024,
    )?;
    let retained = binding["files"]
        .as_array()
        .ok_or("capability retained files required")?;
    if retained.len() != package["files"].as_array().unwrap().len() {
        return Err("capability retained closure mismatch".into());
    }
    let mut expected_assets = BTreeMap::from([(
        expected.executable.clone(),
        text(package, "executable_sha256")?.to_owned(),
    )]);
    for (record, declared) in retained.iter().zip(package["files"].as_array().unwrap()) {
        fields(
            record,
            &["path", "retained_path", "sha256"],
            &[],
            "capability retained file",
        )?;
        if record["path"] != declared["path"] || record["sha256"] != declared["sha256"] {
            return Err("capability retained closure mismatch".into());
        }
        let path = text(record, "retained_path")?;
        if !Path::new(path).is_absolute() {
            return Err("capability retained path must be absolute".into());
        }
        files::verify(Path::new(path), text(record, "sha256")?, 16 * 1024 * 1024)?;
        expected_assets.insert(path.into(), text(record, "sha256")?.into());
        for argument in &mut expected.arguments {
            if matches!(argument, Argument::File(name) if Some(name.as_str()) == record["path"].as_str())
            {
                *argument = Argument::File(path.into());
            }
        }
    }
    if Specification::parse(configuration)?.to_value() != expected.to_value() {
        return Err("capability retained command mismatch".into());
    }
    files::runtime(&package["runtime"]["files"])?;
    for file in package["runtime"]["files"].as_array().unwrap() {
        expected_assets.insert(text(file, "path")?.into(), text(file, "sha256")?.into());
    }
    if Path::new(&source_executable).is_absolute() && source_executable != expected.executable {
        return Err("capability executable path changed".into());
    }
    let mut actual_assets = BTreeMap::new();
    for asset in configuration["assets"]
        .as_array()
        .ok_or("capability dispatch assets required")?
    {
        fields(asset, &["path", "sha256"], &[], "capability dispatch asset")?;
        let key = text(asset, "path")?.to_owned();
        let value = text(asset, "sha256")?.to_owned();
        if actual_assets
            .insert(key, value.clone())
            .is_some_and(|previous| previous != value)
        {
            return Err("capability conflicting dispatch assets".into());
        }
    }
    if actual_assets != expected_assets {
        return Err("capability dispatch closure mismatch".into());
    }
    Ok(())
}
