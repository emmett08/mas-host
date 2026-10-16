//! Retain command artefacts and bind external executable identities.
use mas_host_runtime::command::{Argument, Specification};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(super) fn command(
    project: &Path,
    out: &Path,
    mut spec: Specification,
) -> Result<Value, String> {
    let original = Path::new(&spec.executable);
    let path = if original.is_absolute() {
        original.to_owned()
    } else {
        project_file(project, original)?
    };
    if !path.is_file()
        || std::fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111
            == 0
    {
        return Err("command executable must be an executable regular file".into());
    }
    let (path, hash) = if original.is_absolute() {
        // Preserve absolute interpreter/virtual-environment invocation paths.
        let digest: String = Sha256::digest(std::fs::read(&path).map_err(|e| e.to_string())?)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        (path, digest)
    } else {
        retain(&path, out, "commands", original.file_name(), 0o500)?
    };
    spec.executable = path.to_str().ok_or("command path must be UTF-8")?.into();
    let mut assets = vec![json!({"path":path,"sha256":hash})];
    for argument in &mut spec.arguments {
        if let Argument::File(name) = argument {
            let original = Path::new(name);
            let source = project_file(project, original)?;
            let (path, hash) = retain(&source, out, "resources", original.file_name(), 0o400)?;
            *name = path
                .to_str()
                .ok_or("command file path must be UTF-8")?
                .into();
            assets.push(json!({"path":path,"sha256":hash}));
        }
    }
    let mut configuration = spec.to_value();
    configuration["assets"] = json!(assets);
    Ok(configuration)
}

fn project_file(project: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute() {
        return Err("command file binding must be project-relative".into());
    }
    let path = project
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("command file: {e}"))?;
    if !path.starts_with(project) || !path.is_file() {
        return Err("command file must remain inside the project".into());
    }
    Ok(path)
}

pub(super) fn retain_binary(source: &Path, out: &Path) -> Result<(PathBuf, String), String> {
    retain(source, out, "artifacts", None, 0o500)
}

fn retain(
    source: &Path,
    out: &Path,
    category: &str,
    basename: Option<&OsStr>,
    mode: u32,
) -> Result<(PathBuf, String), String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let artifacts = match basename {
        Some(_) => out.join(category).join(&digest),
        None => out.join(category),
    };
    std::fs::create_dir_all(&artifacts).map_err(|e| e.to_string())?;
    let destination = artifacts.join(basename.unwrap_or_else(|| OsStr::new(&digest)));
    if destination.exists() {
        if std::fs::read(&destination).map_err(|e| e.to_string())? != bytes {
            return Err("retained asset digest conflict".into());
        }
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
    } else {
        let temporary = artifacts.join(format!("{}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        // Publish the snapshot that was hashed, even if the source is replaced
        // during this build. Retained modes never inherit writable/set-id bits.
        let published = (|| -> std::io::Result<()> {
            file.write_all(&bytes)?;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            file.sync_all()?;
            std::fs::rename(&temporary, &destination)
        })();
        if let Err(error) = published {
            let _ = std::fs::remove_file(temporary);
            return Err(error.to_string());
        }
    }
    Ok((destination, digest))
}

pub(super) fn bind_provider_assets(
    configuration: &mut serde_json::Value,
    sdk_root: &Path,
) -> Result<(), String> {
    let Some(command) = configuration["connection"]
        .get_mut("command")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    let mut assets = Vec::new();
    for argument in command {
        if let Some(relative) = argument.as_str().and_then(|s| s.strip_prefix("@sdk/")) {
            let file = sdk_root
                .join(relative)
                .canonicalize()
                .map_err(|e| format!("provider asset: {e}"))?;
            if !file.starts_with(sdk_root) || !file.is_file() {
                return Err("provider asset escapes distribution".into());
            }
            let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
            let hash: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assets.push(json!({"path":file,"sha256":hash}));
            *argument = json!(file);
        }
    }
    if !assets.is_empty() {
        configuration["assets"] = json!(assets);
    }
    Ok(())
}
