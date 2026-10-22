//! Validate bounded project and runtime file identities.
use super::contract::text;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(super) fn project_file(root: &Path, value: &str) -> Result<PathBuf, String> {
    let relative = Path::new(value);
    if relative.is_absolute()
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("capability file must be a relative project path".into());
    }
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        if path.is_symlink() {
            return Err("capability file symlink unsupported".into());
        }
    }
    let path = path
        .canonicalize()
        .map_err(|_| "capability file unavailable")?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("capability file escapes project".into());
    }
    Ok(path)
}

pub(super) fn verify(path: &Path, expected: &str, maximum: u64) -> Result<u64, String> {
    let (actual, size) = digest_file(path, maximum)?;
    if actual != expected {
        return Err("capability asset digest mismatch".into());
    }
    Ok(size)
}

pub(super) fn digest_file(path: &Path, maximum: u64) -> Result<(String, u64), String> {
    let size = path
        .metadata()
        .map_err(|_| "capability asset unavailable")?
        .len();
    if !path.is_file() || size > maximum {
        return Err("capability asset size unsupported".into());
    }
    let mut stream = File::open(path)
        .map_err(|_| "capability asset unavailable")?
        .take(maximum + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut observed = 0;
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|_| "capability asset unreadable")?;
        if count == 0 {
            break;
        }
        observed += count as u64;
        digest.update(&buffer[..count]);
    }
    let actual: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if observed != size {
        return Err("capability asset digest mismatch".into());
    }
    Ok((actual, size))
}

pub(super) fn validate_closure(root: &Path, package: &Value) -> Result<(), String> {
    let mut bytes = 0;
    for file in package["files"].as_array().unwrap() {
        let path = project_file(root, text(file, "path")?)?;
        bytes += verify(&path, text(file, "sha256")?, 16 * 1024 * 1024)?;
    }
    if bytes > 64 * 1024 * 1024 {
        return Err("capability project closure exceeds byte limit".into());
    }
    runtime(&package["runtime"]["files"])
}

pub(super) fn runtime(files: &Value) -> Result<(), String> {
    let mut bytes = 0;
    for file in files
        .as_array()
        .ok_or("capability runtime files required")?
    {
        let path = Path::new(text(file, "path")?);
        if !path.is_absolute() {
            return Err("capability runtime path must be absolute".into());
        }
        bytes += verify(path, text(file, "sha256")?, 1024 * 1024 * 1024)?;
    }
    if bytes > 2 * 1024 * 1024 * 1024 {
        return Err("capability runtime closure exceeds byte limit".into());
    }
    Ok(())
}
