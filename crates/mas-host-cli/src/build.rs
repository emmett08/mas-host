//! Turn application configuration into generated native executables and a bundle.
use crate::config;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
};

fn retain_binary(source: &Path, out: &Path) -> Result<(PathBuf, String), String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let artifacts = out.join("artifacts");
    std::fs::create_dir_all(&artifacts).map_err(|e| e.to_string())?;
    let destination = artifacts.join(&digest);
    if destination.exists() {
        if std::fs::read(&destination).map_err(|e| e.to_string())? != bytes {
            return Err("retained executable digest conflict".into());
        }
    } else {
        let temporary = artifacts.join(format!("{}.tmp", std::process::id()));
        std::fs::copy(source, &temporary).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, &destination).map_err(|e| e.to_string())?;
    }
    Ok((destination, digest))
}

pub fn build(project: &Path, sdk_root: &Path) -> Result<PathBuf, String> {
    let project = project.canonicalize().map_err(|e| e.to_string())?;
    let sdk_root = sdk_root.canonicalize().map_err(|e| e.to_string())?;
    let (agents, topology) = config::load(&project)?;
    let out = project.join(".mas/build");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let owner = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(out.join("build.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("another build owns this application directory".into());
    }
    let mut workers = Vec::new();
    if agents
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["language"] == "go")
    {
        let status = Command::new("go")
            .current_dir(sdk_root.join("sdk/go"))
            .args(["run", "./cmd/mas-go-gen", "--project"])
            .arg(&project)
            .arg("--output")
            .arg(&out)
            .arg("--sdk-dir")
            .arg(sdk_root.join("sdk/go"))
            .stdout(std::process::Stdio::from(std::io::stderr()))
            .stderr(std::process::Stdio::inherit())
            .status()
            .map_err(|e| format!("cannot start Go generator: {e}"))?;
        if !status.success() {
            return Err("Go generation/build failed".into());
        }
        let registry = config::read_json(&out.join("go-workers.json"))?;
        workers.extend(
            registry
                .as_array()
                .ok_or("invalid generated Go registry")?
                .clone(),
        );
    }
    workers.extend(mas_host_native_build::build(
        &project, &out, &sdk_root, &agents,
    )?);
    workers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    for worker in &mut workers {
        let executable = worker["executable"].as_str().ok_or("missing executable")?;
        let source = Path::new(executable)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let (path, digest) = retain_binary(&source, &out)?;
        worker["sha256"] = json!(digest);
        worker["executable"] = json!(path.to_str().ok_or("executable path is not UTF-8")?);
    }
    let bundle = json!({"version":"mas-bundle/1","agents":workers,"topology":topology});
    let destination = out.join("bundle.json");
    let snapshots = out.join("bundles");
    std::fs::create_dir_all(&snapshots).map_err(|e| e.to_string())?;
    let content = serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?;
    let bundle_digest: String = Sha256::digest(&content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let snapshot = snapshots.join(format!("{bundle_digest}.json"));
    if !snapshot.exists() {
        std::fs::write(&snapshot, &content).map_err(|e| e.to_string())?;
    }
    let temporary = out.join("bundle.json.tmp");
    std::fs::write(&temporary, content).map_err(|e| e.to_string())?;
    std::fs::rename(temporary, &destination).map_err(|e| e.to_string())?;
    Ok(snapshot)
}
