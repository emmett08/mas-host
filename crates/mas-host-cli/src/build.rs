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

pub fn build_configured(
    project: &Path,
    sdk_root: &Path,
    agents_path: Option<&Path>,
    connections_path: Option<&Path>,
    topology_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let project = project.canonicalize().map_err(|e| e.to_string())?;
    let sdk_root = sdk_root.canonicalize().map_err(|e| e.to_string())?;
    let agents_path = config::discover(&project, "agents", agents_path)?;
    let topology_path = config::discover(&project, "topology", topology_path)?;
    let profile = connections_path.map(|path| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            project.join(path)
        }
    });
    let (agents, topology) =
        config::load_profile(&project, &agents_path, &topology_path, profile.as_deref())?;
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
    let native: Vec<_> = agents
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a.get("configuration").is_none())
        .map(|a| {
            let mut native = a.clone();
            native.as_object_mut().unwrap().remove("operation");
            native
        })
        .collect();
    let native = json!(native);
    let native_config = out.join("native-agents.json");
    std::fs::write(
        &native_config,
        serde_json::to_vec(&json!({"version":"mas-agents/1","agents":native}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
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
            .arg("--agents")
            .arg(&native_config)
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
        &project, &out, &sdk_root, &native,
    )?);
    for agent in agents.as_array().unwrap() {
        if let Some(configuration) = agent.get("configuration") {
            let mut configuration = configuration.clone();
            bind_assets(&mut configuration, &sdk_root)?;
            workers.push(json!({"name":agent["name"],"executable":std::env::current_exe().map_err(|e|e.to_string())?,"handler":agent["handler"],"arguments":["__builtin"],"configuration":configuration}));
        }
        if let Some(operation) = agent.get("operation") {
            if let Some(worker) = workers.iter_mut().find(|w| w["name"] == agent["name"]) {
                worker["operation"] = operation.clone();
            }
        }
    }
    workers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let mut retained: std::collections::BTreeMap<PathBuf, (PathBuf, String)> =
        std::collections::BTreeMap::new();
    for worker in &mut workers {
        let executable = worker["executable"].as_str().ok_or("missing executable")?;
        let source = Path::new(executable)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let (path, digest) = match retained.get(&source) {
            Some(binding) => binding.clone(),
            None => {
                let binding = retain_binary(&source, &out)?;
                retained.insert(source, binding.clone());
                binding
            }
        };
        worker["sha256"] = json!(digest);
        worker["executable"] = json!(path.to_str().ok_or("executable path is not UTF-8")?);
    }
    let version = if workers
        .iter()
        .any(|w| w.get("configuration").is_some() || w.get("operation").is_some())
    {
        "mas-bundle/2"
    } else {
        "mas-bundle/1"
    };
    let bundle = json!({"version":version,"agents":workers,"topology":topology});
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

fn bind_assets(configuration: &mut serde_json::Value, sdk_root: &Path) -> Result<(), String> {
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
