//! Operator-owned local PostgreSQL connection and guardian lifetime locks.
use postgres::{
    config::{Host, SslMode},
    Client, Config, NoTls,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    str::FromStr,
};

pub(super) struct Authority {
    pub configuration: Value,
    pub repository: mas_protocol_codec::postgres_repository::PostgresRepository,
}
impl Authority {
    pub fn open(args: &BTreeMap<String, String>) -> Result<Self, String> {
        let authority = crate::config::read_json(Path::new(crate::require(args, "--authority")?))?;
        crate::config::fields(
            &authority,
            &[
                "version",
                "schema",
                "deployment",
                "controller",
                "epoch",
                "principal",
                "owner",
            ],
            &[],
            "shared native authority",
        )?;
        if authority["version"] != "mas-host-shared-native-authority/1" {
            return Err("unsupported shared native authority".into());
        }
        let text = |key: &str| {
            authority[key]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 1024)
                .ok_or_else(|| format!("invalid {key}"))
        };
        let dsn = std::env::var("MAS_POSTGRES_DSN")
            .map_err(|_| "shared PostgreSQL connection not configured")?;
        let config = Config::from_str(&dsn).map_err(|_| "invalid shared PostgreSQL connection")?;
        // This initial CLI slice accepts only an operator-owned local server/tunnel.
        // The core API accepts an authenticated TLS client for a qualified connector.
        let remote = config.get_hostaddrs().iter().any(|ip| !ip.is_loopback())
            || config.get_hosts().iter().any(|h| match h {
                Host::Tcp(name) => !matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1"),
                #[cfg(unix)]
                Host::Unix(_) => false,
            });
        if remote || config.get_ssl_mode() == SslMode::Require {
            return Err(
            "shared native CLI requires a local PostgreSQL endpoint or authenticated local tunnel"
                .into(),
        );
        }
        let client: Client = config
            .connect(NoTls)
            .map_err(|_| "shared PostgreSQL unavailable")?;
        let binding = mas_protocol_codec::postgres_repository::AuthorityBinding {
            deployment: text("deployment")?.into(),
            controller: text("controller")?.into(),
            epoch: authority["epoch"]
                .as_i64()
                .ok_or("invalid authority epoch")?,
        };
        let repository = mas_protocol_codec::postgres_repository::PostgresRepository::open(
            client,
            text("schema")?,
            binding,
        )?;
        Ok(Self {
            configuration: authority,
            repository,
        })
    }
    pub fn text(&self, key: &str) -> Result<&str, String> {
        self.configuration[key]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 1024)
            .ok_or_else(|| format!("invalid authority {key}"))
    }
}
pub(super) fn lifetime(args: &BTreeMap<String, String>, identity: &str) -> Result<File, String> {
    let state = Path::new(crate::require(args, "--state")?);
    std::fs::create_dir_all(state).map_err(|_| "shared native lifetime state unavailable")?;
    if std::fs::symlink_metadata(state)
        .map_err(|_| "shared native lifetime state unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("shared native lifetime state cannot be symlink".into());
    }
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "shared native lifetime state unavailable")?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(state.join(format!("shared-native-{}.lock", {
            use sha2::{Digest, Sha256};
            Sha256::digest(identity.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })))
        .map_err(|_| "shared native lifetime lock unavailable")?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("shared native local lifetime already owned".into());
    }
    Ok(lock)
}
