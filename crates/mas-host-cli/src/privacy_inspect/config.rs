//! Load a bounded local detector launch configuration without exposing source values.
use mas_host_privacy::DetectorPins;
use mas_host_worker::WorkerLimits;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) struct Config {
    pub python: PathBuf,
    pub worker: PathBuf,
    pub manifest: PathBuf,
    pub assets: PathBuf,
    pub pins: DetectorPins,
    pub timeout_ms: u64,
    pub limits: WorkerLimits,
    pub details: bool,
}

pub(super) fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "privacy file unavailable")?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "privacy file unreadable")?;
    if bytes.len() > maximum {
        return Err("privacy file exceeds byte limit".into());
    }
    Ok(bytes)
}

pub(super) fn required<'a>(
    args: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, String> {
    args.get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn count(
    args: &BTreeMap<String, String>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    let value = args
        .get(key)
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|_| format!("invalid {key}"))?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("invalid {key}"));
    }
    Ok(value)
}

impl Config {
    pub fn load(args: &BTreeMap<String, String>) -> Result<Self, String> {
        let timeout_ms = count(args, "--timeout-ms", 120_000, 1, 600_000)?;
        let memory_mib = count(args, "--memory-mib", 12_288, 256, 65_536)?;
        let limits = WorkerLimits::new(memory_mib * 1_048_576)?;
        let details = match args.get("--include-details").map(String::as_str) {
            None | Some("false") => false,
            Some("true") => true,
            _ => return Err("invalid --include-details".into()),
        };
        // Preserve the venv's executable path: resolving its symlink would select
        // the base interpreter and discard the provisioned runtime environment.
        let python = PathBuf::from(required(args, "--python")?);
        if !python.is_absolute() || !python.is_file() {
            return Err("privacy interpreter must be an absolute file".into());
        }
        let manifest = Path::new(required(args, "--manifest")?)
            .canonicalize()
            .map_err(|_| "privacy manifest unavailable")?;
        let assets = Path::new(required(args, "--assets")?)
            .canonicalize()
            .map_err(|_| "privacy assets unavailable")?;
        if !assets.is_dir() {
            return Err("privacy assets must be a directory".into());
        }
        let default_worker =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/privacy/worker.py");
        let worker = args
            .get("--worker")
            .map(PathBuf::from)
            .unwrap_or(default_worker)
            .canonicalize()
            .map_err(|_| "privacy worker unavailable")?;
        if [&worker, &manifest, &assets]
            .iter()
            .any(|path| path.to_str().is_none())
        {
            return Err("privacy resolved paths must be UTF-8".into());
        }
        let raw = read_bounded(&manifest, 262_144)?;
        let manifest_value = mas_protocol_codec::parse_json(
            std::str::from_utf8(&raw).map_err(|_| "invalid privacy manifest")?,
        )
        .map_err(|_| "invalid privacy manifest")?;
        let object = manifest_value
            .as_object()
            .ok_or("invalid privacy manifest")?;
        let fields = [
            "version",
            "code_revision",
            "model_revision",
            "backend",
            "adapter",
            "worker_sha256",
            "adapter_files",
            "files",
            "runtime",
            "settings",
        ];
        if object.len() != fields.len()
            || fields.iter().any(|name| !object.contains_key(*name))
            || manifest_value["version"] != "mas-privacy-bundle/1"
            || manifest_value["settings"]["max_input_bytes"] != 16_384
            || manifest_value["settings"]["max_response_bytes"] != 65_536
            || manifest_value["settings"]["max_findings"] != 128
        {
            return Err("unsupported privacy manifest".into());
        }
        let worker_bytes = read_bounded(&worker, 1_048_576)?;
        if manifest_value["worker_sha256"].as_str() != Some(hex_digest(&worker_bytes).as_str()) {
            return Err("privacy worker identity mismatch".into());
        }
        let pins = DetectorPins::from_value(&json!({
            "bundle_sha256": hex_digest(&raw),
            "code_revision": manifest_value["code_revision"],
            "model_revision": manifest_value["model_revision"],
            "backend": manifest_value["backend"],
            "adapter": manifest_value["adapter"],
        }))
        .map_err(|_| "unsupported privacy detector identity")?;
        Ok(Self {
            python,
            worker,
            manifest,
            assets,
            pins,
            timeout_ms,
            limits,
            details,
        })
    }
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
