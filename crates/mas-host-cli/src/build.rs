//! Turn application configuration into generated native executables and a bundle.
use crate::config;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
};

mod assets;
use assets::retain_binary;
use config::model::{Agent, Implementation};

pub(crate) fn retain_capability_file(
    source: &Path,
    out: &Path,
) -> Result<(PathBuf, String), String> {
    assets::retain_resource(source, out)
}

pub fn build_with_capabilities(
    project: &Path,
    sdk_root: &Path,
    agents_path: Option<&Path>,
    connections_path: Option<&Path>,
    topology_path: Option<&Path>,
    capabilities_path: Option<&Path>,
    policy_path: Option<&Path>,
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
    let packages = match (capabilities_path, policy_path) {
        (None, None) => None,
        (Some(sidecar), Some(policy)) => Some(crate::capabilities::Packages::load(
            &project, &agents, sidecar, policy,
        )?),
        _ => return Err("capability sidecar and operator policy must be supplied together".into()),
    };
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
    let native: Vec<_> = agents.iter().filter_map(Agent::native).collect();
    let native = json!(native);
    let native_config = out.join("native-agents.json");
    std::fs::write(
        &native_config,
        serde_json::to_vec(&json!({"version":"mas-native-build/1","agents":native}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if agents.iter().any(|agent| matches!(&agent.implementation, Implementation::Native(native) if native.language == "go")) {
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
    for agent in &agents {
        let configuration = match &agent.implementation {
            Implementation::Native(_) => None,
            Implementation::Llm(configuration) | Implementation::Workflow(configuration) => {
                Some(configuration.clone())
            }
            Implementation::Command(specification) => {
                Some(assets::command(&project, &out, specification.clone())?)
            }
        };
        if let Some(mut configuration) = configuration {
            if let Some(schema) = &agent.input_schema {
                configuration["input_schema"] = schema.clone();
            }
            if let Some(schema) = &agent.output_schema {
                configuration["output_schema"] = schema.clone();
            }
            if let Implementation::Llm(_) = &agent.implementation {
                let contracts = configuration["delegates"]
                    .as_array()
                    .ok_or("resolved delegates must be an array")?
                    .iter()
                    .map(|name| {
                        let peer = agents
                            .iter()
                            .find(|peer| Some(peer.name.as_str()) == name.as_str())
                            .ok_or("resolved delegate has no application operation")?;
                        Ok(json!({"name":peer.name,"operation":peer.operation,
                            "input_schema":peer.input_schema,"output_schema":peer.output_schema}))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                configuration["delegate_contracts"] = json!(contracts);
            }
            assets::bind_provider_assets(&mut configuration, &sdk_root)?;
            workers.push(json!({"name":agent.name,"executable":std::env::current_exe().map_err(|e|e.to_string())?,"arguments":["__builtin"],"configuration":configuration}));
        }
        let worker = workers
            .iter_mut()
            .find(|w| w["name"] == agent.name)
            .ok_or("resolved agent has no generated worker")?;
        worker["operation"] = json!(agent.operation);
        worker["environment"] = json!(agent.environment);
        worker["working_directory"] = json!(agent.working_directory);
        if let Some(schema) = &agent.input_schema {
            worker["input_schema"] = schema.clone();
        }
        if let Some(schema) = &agent.output_schema {
            worker["output_schema"] = schema.clone();
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
    if let Some(packages) = &packages {
        packages.bind(&project, &out, &mut workers)?;
    }
    let bundle = json!({"version":"mas-bundle/3","agents":workers,"topology":topology});
    // The assembled document must obey the same strict bound as its reader.
    // Compact encoding leaves the budget for configuration rather than layout.
    // Validate before publishing either the snapshot or current bundle pointer.
    let content = serde_json::to_string(&bundle).map_err(|e| e.to_string())?;
    mas_protocol_codec::parse_json(&content)
        .map_err(|e| format!("generated bundle: {}: {}", e.code, e.message))?;
    let content = content.into_bytes();
    let destination = out.join("bundle.json");
    let snapshots = out.join("bundles");
    std::fs::create_dir_all(&snapshots).map_err(|e| e.to_string())?;
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
