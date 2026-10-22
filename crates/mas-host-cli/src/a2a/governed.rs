//! Derive governed sessions from the operator's current authentication table.
use super::{configuration::Configuration, Result};
use mas_host_core::governance::GovernedSession;
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(super) struct Caller {
    pub owner: String,
    pub agents: BTreeSet<String>,
    pub session: Option<GovernedSession>,
    pub lease: Option<AuthenticationLease>,
}

#[derive(Clone)]
pub(super) struct AuthenticationLease {
    file: PathBuf,
    record: Value,
    exports: BTreeSet<String>,
    key_file: PathBuf,
    key_digest: String,
}

pub(super) struct GovernedIngress {
    authentication_file: PathBuf,
    session_config: Value,
    key_file: PathBuf,
    key_digest: String,
    privacy_inspector: Option<crate::privacy_inspect::GovernedInspector>,
}

fn identifier(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:/-".contains(&c))
        })
        .map(str::to_owned)
        .ok_or_else(|| "invalid governed identity".into())
}

impl GovernedIngress {
    pub fn load(config: &Value) -> Result<Self> {
        let path = |key: &str| -> Result<PathBuf> {
            let path = PathBuf::from(config[key].as_str().ok_or("governed path required")?);
            if !path.is_absolute() {
                return Err("governed paths must be absolute".into());
            }
            Ok(path)
        };
        let authentication_file = path("authentication_file")?;
        let session_config = crate::config::read_json(&path("governance_file")?)
            .map_err(|_| "governance configuration unavailable")?;
        let key_file = PathBuf::from(
            session_config["commitment_key_file"]
                .as_str()
                .ok_or("commitment key path required")?,
        );
        let key_digest = key_identity(&key_file)?;
        let privacy_inspector = config
            .get("privacy_inspector_file")
            .map(|_| {
                crate::privacy_inspect::GovernedInspector::load(&path("privacy_inspector_file")?)
            })
            .transpose()?;
        Ok(Self {
            authentication_file,
            session_config,
            key_file,
            key_digest,
            privacy_inspector,
        })
    }

    fn records(&self, configuration: &Configuration) -> Result<Vec<Value>> {
        read_records(
            &self.authentication_file,
            &configuration.exports.keys().cloned().collect(),
        )
    }

    pub fn validate(&self, configuration: &Configuration) -> Result<()> {
        self.records(configuration)?;
        Ok(())
    }

    pub fn authenticate(&self, configuration: &Configuration, bearer: &str) -> Option<Caller> {
        if key_identity(&self.key_file).ok()? != self.key_digest {
            return None;
        }
        let hash = super::digest(bearer.as_bytes());
        let records = self.records(configuration).ok()?;
        let record = records.iter().find(|record| {
            hash.bytes()
                .zip(record["token_sha256"].as_str().unwrap().bytes())
                .fold(0u8, |d, (a, b)| d | (a ^ b))
                == 0
        })?;
        let session = GovernedSession::from_value(
            &self.session_config,
            record["tenant"].as_str()?,
            record["principal"].as_str()?,
            record["purpose"].as_str()?,
            record["grant_ref"].as_str()?,
        )
        .ok()?;
        if key_identity(&self.key_file).ok()? != self.key_digest {
            return None;
        }
        let expected = super::digest(
            json!([
                "mas-governed-key/1",
                self.session_config["commitment_key_id"],
                self.key_digest
            ])
            .to_string()
            .as_bytes(),
        );
        if session.key_fingerprint() != expected {
            return None;
        }
        let owner = session
            .identity_commitment(&json!(["a2a-owner", record["tenant"], record["principal"]]))
            .ok()?;
        let lease = AuthenticationLease {
            file: self.authentication_file.clone(),
            record: record.clone(),
            exports: configuration.exports.keys().cloned().collect(),
            key_file: self.key_file.clone(),
            key_digest: self.key_digest.clone(),
        };
        let revalidation = lease.clone();
        let session = session.with_authentication_check(Arc::new(move || revalidation.check()));
        let session = match &self.privacy_inspector {
            Some(inspector) => inspector.attach(session).ok()?,
            None => session,
        };
        Some(Caller {
            owner,
            session: Some(session),
            agents: record["agents"]
                .as_array()?
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect(),
            lease: Some(lease),
        })
    }

    pub fn identity(&self) -> Value {
        let mut identity = json!({"version":"mas-governed-ingress/1", "configuration":self.session_config,
            "authentication_file":self.authentication_file,"commitment_key_fingerprint":self.key_digest});
        if let Some(inspector) = &self.privacy_inspector {
            identity["privacy_inspector"] = inspector.identity();
        }
        identity
    }

    pub(super) fn inspection_timeout_ms(&self) -> Option<u64> {
        self.privacy_inspector.as_ref().map(|i| i.timeout_ms())
    }

    pub(super) fn source_reads_supported(&self) -> bool {
        self.session_config["version"] == "mas-governed-local/2"
    }
}

fn key_identity(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "commitment key unavailable")?;
    let metadata = file.metadata().map_err(|_| "commitment key unavailable")?;
    if !metadata.is_file() || metadata.len() != 32 || metadata.permissions().mode() & 0o077 != 0 {
        return Err("commitment key requires private regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(33)
        .read_to_end(&mut bytes)
        .map_err(|_| "commitment key unavailable")?;
    if bytes.len() != 32 {
        return Err("commitment key length".into());
    }
    Ok(super::digest(&bytes))
}

fn read_records(file: &std::path::Path, exports: &BTreeSet<String>) -> Result<Vec<Value>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let source = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(file)
        .map_err(|_| "authentication authority unavailable")?;
    if !source
        .metadata()
        .map_err(|_| "authentication authority unavailable")?
        .is_file()
    {
        return Err("authentication authority unavailable".into());
    }
    let mut text = String::new();
    source
        .take(65_537)
        .read_to_string(&mut text)
        .map_err(|_| "authentication authority unavailable")?;
    let value = mas_protocol_codec::parse_json(&text)
        .map_err(|_| "authentication authority unavailable")?;
    crate::config::fields(
        &value,
        &["version", "principals"],
        &[],
        "authentication authority",
    )?;
    if value["version"] != "mas-host-authentication/1" {
        return Err("unsupported authentication authority".into());
    }
    let records = value["principals"]
        .as_array()
        .filter(|v| v.len() <= 128)
        .ok_or("authentication principal limit")?;
    let mut names = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    for record in records {
        crate::config::fields(
            record,
            &[
                "tenant",
                "principal",
                "purpose",
                "grant_ref",
                "token_sha256",
                "agents",
            ],
            &[],
            "authentication principal",
        )?;
        for key in ["tenant", "principal", "purpose", "grant_ref"] {
            identifier(record, key)?;
        }
        if !names.insert((
            record["tenant"].clone().to_string(),
            record["principal"].clone().to_string(),
        )) {
            return Err("duplicate authentication principal".into());
        }
        let token = record["token_sha256"]
            .as_str()
            .ok_or("invalid token identity")?;
        if token.len() != 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !tokens.insert(token.to_owned())
        {
            return Err("invalid or duplicate token identity".into());
        }
        let agents = record["agents"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 31)
            .ok_or("invalid authentication grants")?;
        let mut unique = BTreeSet::new();
        for agent in agents {
            let agent = agent.as_str().ok_or("invalid authentication grant")?;
            if !exports.contains(agent) || !unique.insert(agent) {
                return Err("unknown or duplicate authentication grant".into());
            }
        }
    }
    Ok(records.clone())
}

impl Caller {
    pub fn current(&self) -> Result<()> {
        if let Some(lease) = &self.lease {
            lease.check()?;
        }
        match &self.session {
            Some(session) => session.check_current(),
            None => Ok(()),
        }
    }
    pub fn input(&self, agent: &str, value: &Value) -> Result<Value> {
        self.current()?;
        if !self.agents.contains(agent) {
            return Err("export unavailable".into());
        }
        match &self.session {
            Some(session) => session.project_input(agent, value),
            None => Ok(value.clone()),
        }
    }
    pub fn delivery(&self, agent: &str, value: &Value) -> Result<()> {
        self.current()?;
        if !self.agents.contains(agent) {
            return Err("export unavailable".into());
        }
        match &self.session {
            Some(session) => session.check_delivery(agent, value),
            None => Ok(()),
        }
    }
    pub fn message_key(&self, original: &str) -> Result<String> {
        match &self.session {
            Some(session) => session.identity_commitment(&json!(["a2a-message", original])),
            None => Ok(original.to_owned()),
        }
    }
    pub fn content(&self, original: &Value) -> Result<String> {
        match &self.session {
            Some(session) => session.commitment(original),
            None => Ok(original.to_string()),
        }
    }
}

impl AuthenticationLease {
    fn check(&self) -> Result<()> {
        if key_identity(&self.key_file)? != self.key_digest
            || !read_records(&self.file, &self.exports)?.contains(&self.record)
        {
            return Err("authentication authority changed".into());
        }
        Ok(())
    }
}

pub(super) fn rpc_id_allowed(value: &Value) -> bool {
    value.is_i64()
        || value.is_u64()
        || value.as_str().is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
}
