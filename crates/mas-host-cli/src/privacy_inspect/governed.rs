//! Bind the existing offline detector launcher to operator-approved governed sessions.
use super::{config, runner};
use mas_host_core::governance::{GovernedSession, PrivacyInspector};
use mas_host_privacy::{DetectorPins, Limits, NeuralAssessment, Source};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub(crate) struct GovernedInspector {
    inspector: Arc<LocalInspector>,
    destination: String,
}

struct LocalInspector {
    file: PathBuf,
    binding: String,
    arguments: BTreeMap<String, String>,
    pins: DetectorPins,
    python_sha256: String,
    worker_slot: Mutex<()>,
}

fn read_configuration(path: &Path) -> Result<Value> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "governed-privacy-configuration-unavailable")?;
    if !file
        .metadata()
        .map_err(|_| "governed-privacy-configuration-unavailable")?
        .is_file()
    {
        return Err("governed-privacy-configuration-unavailable".into());
    }
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "governed-privacy-configuration-unavailable")?;
    if bytes.len() > 65_536 {
        return Err("governed-privacy-configuration-limit".into());
    }
    mas_protocol_codec::parse_json(
        std::str::from_utf8(&bytes).map_err(|_| "governed-privacy-configuration-invalid")?,
    )
    .map_err(|_| "governed-privacy-configuration-invalid".into())
}

fn executable_identity(path: &Path) -> Result<String> {
    // Preserve a virtual environment's interpreter path while pinning the
    // executable bytes it resolves to. Runtime dependencies are checked by the
    // approved N1 worker against its manifest before accepting source content.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "governed-privacy-runtime-unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "governed-privacy-runtime-unavailable")?;
    if !metadata.is_file() || metadata.len() > 134_217_728 {
        return Err("governed-privacy-runtime-unavailable".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut total = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "governed-privacy-runtime-unavailable")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > 134_217_728 {
            return Err("governed-privacy-runtime-unavailable".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn configuration_binding(configuration: &Value) -> String {
    config::hex_digest(
        json!(["mas-governed-privacy-inspector-binding/1", configuration,
            {"max_active_workers":1,"overload":"reject-without-queue"}])
        .to_string()
        .as_bytes(),
    )
}

impl GovernedInspector {
    pub(crate) fn load(file: &Path) -> Result<Self> {
        if !file.is_absolute() {
            return Err("governed-privacy-configuration-path".into());
        }
        let configuration = read_configuration(file)?;
        crate::config::fields(
            &configuration,
            &[
                "version",
                "python",
                "worker",
                "manifest",
                "assets",
                "detector",
                "python_sha256",
                "destination",
                "timeout_ms",
                "memory_mib",
            ],
            &[],
            "governed privacy configuration",
        )
        .map_err(|_| "governed-privacy-configuration-invalid")?;
        if configuration["version"] != "mas-governed-privacy-inspector/1" {
            return Err("governed-privacy-configuration-version".into());
        }
        let pins = DetectorPins::from_value(&configuration["detector"])
            .map_err(|_| "governed-privacy-detector-invalid")?;
        let destination = configuration["destination"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
            })
            .ok_or("governed-privacy-destination-invalid")?
            .to_owned();
        let python_sha256 = configuration["python_sha256"]
            .as_str()
            .filter(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or("governed-privacy-runtime-identity-invalid")?
            .to_owned();
        let mut arguments = BTreeMap::new();
        for name in ["python", "worker", "manifest", "assets"] {
            let value = configuration[name]
                .as_str()
                .filter(|s| {
                    s.len() <= 4096
                        && !s.chars().any(char::is_control)
                        && Path::new(s).is_absolute()
                })
                .ok_or("governed-privacy-launch-path")?;
            arguments.insert(format!("--{name}"), value.to_owned());
        }
        for name in ["timeout_ms", "memory_mib"] {
            let value = configuration[name]
                .as_u64()
                .ok_or("governed-privacy-launch-limit")?;
            if name == "timeout_ms" && !(1..=120_000).contains(&value) {
                return Err("governed-privacy-launch-limit".into());
            }
            arguments.insert(format!("--{}", name.replace('_', "-")), value.to_string());
        }
        let result = Self {
            destination,
            inspector: Arc::new(LocalInspector {
                file: file.to_owned(),
                binding: configuration_binding(&configuration),
                arguments,
                pins,
                python_sha256,
                worker_slot: Mutex::new(()),
            }),
        };
        result.inspector.launch_configuration()?;
        Ok(result)
    }

    pub(crate) fn attach(&self, session: GovernedSession) -> Result<GovernedSession> {
        session.with_privacy_inspector(self.inspector.clone(), &self.destination)
    }

    pub(crate) fn identity(&self) -> Value {
        json!({"version":"mas-governed-privacy-inspector-identity/1",
            "configuration_binding":self.inspector.binding,
            "detector":self.inspector.pins.value(),"destination":self.destination,
            "max_active_workers":1,"overload":"reject-without-queue"})
    }

    pub(crate) fn timeout_ms(&self) -> u64 {
        self.inspector.timeout_ms()
    }
}

impl LocalInspector {
    fn launch_configuration(&self) -> Result<config::Config> {
        if configuration_binding(&read_configuration(&self.file)?) != self.binding {
            return Err("governed-privacy-configuration-changed".into());
        }
        for name in ["--python", "--worker", "--manifest"] {
            if !std::fs::metadata(&self.arguments[name])
                .map_err(|_| "governed-privacy-launch-unavailable")?
                .is_file()
            {
                return Err("governed-privacy-launch-unavailable".into());
            }
        }
        let config = config::Config::load(&self.arguments)
            .map_err(|_| "governed-privacy-launch-unavailable")?;
        if config.pins != self.pins || executable_identity(&config.python)? != self.python_sha256 {
            return Err("governed-privacy-launch-identity-changed".into());
        }
        Ok(config)
    }
}

impl PrivacyInspector for LocalInspector {
    fn detector(&self) -> &DetectorPins {
        &self.pins
    }

    fn configuration_binding(&self) -> String {
        match read_configuration(&self.file) {
            Ok(value) if configuration_binding(&value) == self.binding => self.binding.clone(),
            _ => String::new(),
        }
    }

    fn timeout_ms(&self) -> u64 {
        self.arguments["--timeout-ms"].parse().unwrap()
    }

    fn inspect(
        &self,
        source: &Source,
        limits: Limits,
        deadline_ms: u64,
    ) -> Result<NeuralAssessment> {
        // One shared inspector instance serves every authenticated session in
        // this service. Keep the slot through guardian descendant cleanup.
        // Excess demand stops at the caller's existing hold/withhold boundary.
        let _worker = self
            .worker_slot
            .try_lock()
            .map_err(|_| "governed-privacy-inspector-busy")?;
        runner::inspect_deadline(&self.launch_configuration()?, source, limits, deadline_ms)
            .map_err(|_| "governed-privacy-inspection-failed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn revoked_configuration_invalidates_identity_without_launching_a_text_scan() {
        let file = std::env::temp_dir().join(format!(
            "mas-inspector-config-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let configuration = json!({"approved":"contract-1"});
        std::fs::write(&file, configuration.to_string()).unwrap();
        let inspector = LocalInspector {
            file: file.clone(),
            binding: configuration_binding(&configuration),
            arguments: BTreeMap::new(),
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            python_sha256: "0".repeat(64),
            worker_slot: Mutex::new(()),
        };
        assert_eq!(inspector.configuration_binding(), inspector.binding);
        std::fs::write(&file, json!({"approved":"contract-2"}).to_string()).unwrap();
        assert!(inspector.configuration_binding().is_empty());
        std::fs::remove_file(&file).unwrap();
        assert!(inspector.configuration_binding().is_empty());
    }

    #[test]
    fn occupied_inspector_rejects_concurrent_work_without_a_launch_or_queue() {
        let inspector = Arc::new(LocalInspector {
            file: PathBuf::from("/unavailable-inspector-config"),
            binding: "0".repeat(64),
            arguments: BTreeMap::new(),
            pins: DetectorPins::new(&"a".repeat(64), &"b".repeat(40), &"c".repeat(40)).unwrap(),
            python_sha256: "0".repeat(64),
            worker_slot: Mutex::new(()),
        });
        let holder = Arc::clone(&inspector);
        let (occupied, acquired) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _slot = holder.worker_slot.lock().unwrap();
            occupied.send(()).unwrap();
            released.recv().unwrap();
        });
        acquired.recv().unwrap();
        let source = Source::text("concurrent-source", "synthetic", 16384).unwrap();
        let result = inspector.inspect(&source, Limits::default(), u64::MAX);
        assert_eq!(result.unwrap_err(), "governed-privacy-inspector-busy");
        release.send(()).unwrap();
        worker.join().unwrap();
    }
}
