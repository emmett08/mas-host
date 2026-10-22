//! Resolve currently scoped credential references immediately before worker launch.
use super::GovernedSession;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-credential-unavailable";

fn records(path: &Path) -> Result<Value> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| UNAVAILABLE)?;
    let metadata = file.metadata().map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() > 65_536 {
        return Err(UNAVAILABLE.into());
    }
    let value =
        mas_protocol_codec::parse_json(std::str::from_utf8(&bytes).map_err(|_| UNAVAILABLE)?)
            .map_err(|_| UNAVAILABLE)?;
    crate::fields(&value, &["version", "credentials"], &[]).map_err(|_| UNAVAILABLE)?;
    if value["version"] != "mas-governed-credentials/1"
        || !value["credentials"]
            .as_array()
            .is_some_and(|v| v.len() <= 128)
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(value)
}

pub(crate) struct CredentialEnvironment {
    pub(crate) values: BTreeMap<String, String>,
    bindings: Vec<(String, u64, String)>,
}
impl CredentialEnvironment {
    pub(crate) fn admit(&self, db: &rusqlite::Connection, actor: &str) -> Result<()> {
        use rusqlite::{params, OptionalExtension};
        let tx = db.unchecked_transaction().map_err(|_| UNAVAILABLE)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS governed_credential_epochs(actor TEXT NOT NULL,reference TEXT NOT NULL,epoch INTEGER NOT NULL,binding TEXT NOT NULL,PRIMARY KEY(actor,reference));").map_err(|_|UNAVAILABLE)?;
        for (name, epoch, binding) in &self.bindings {
            let epoch = i64::try_from(*epoch).map_err(|_| UNAVAILABLE)?;
            let prior:Option<(i64,String)>=tx.query_row("SELECT epoch,binding FROM governed_credential_epochs WHERE actor=? AND reference=?",params![actor,name],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|UNAVAILABLE)?;
            if prior.is_some_and(|(old, hash)| old > epoch || (old == epoch && hash != *binding)) {
                return Err("governed-credential-epoch-conflict".into());
            }
            tx.execute("INSERT INTO governed_credential_epochs VALUES(?,?,?,?) ON CONFLICT(actor,reference) DO UPDATE SET epoch=excluded.epoch,binding=excluded.binding",params![actor,name,epoch,binding]).map_err(|_|UNAVAILABLE)?;
        }
        tx.commit().map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
}

impl GovernedSession {
    /// The declared environment names remain part of the retained executable
    /// contract. Only their current secret values rotate. A credential record
    /// cannot add a name or widen actor, destination, principal or grant scope.
    pub(crate) fn credential_environment(
        &self,
        actor: &str,
        allowed: &BTreeSet<String>,
    ) -> Result<Option<CredentialEnvironment>> {
        let Some(path) = self.0.credentials_file.as_deref() else {
            return Ok(None);
        };
        self.check_current()?;
        let value = records(path)?;
        let mut resolved = BTreeMap::new();
        let mut bindings = Vec::new();
        let mut seen = BTreeSet::new();
        let destination = &self.agent(actor)?.destination;
        let now = crate::now_ms()?;
        for record in value["credentials"].as_array().ok_or(UNAVAILABLE)? {
            crate::fields(
                record,
                &[
                    "tenant",
                    "principal",
                    "grant_ref",
                    "grant_epoch",
                    "actor",
                    "destination",
                    "reference",
                    "credential_epoch",
                    "expires_at_ms",
                    "value",
                ],
                &[],
            )
            .map_err(|_| UNAVAILABLE)?;
            for field in [
                "tenant",
                "principal",
                "grant_ref",
                "actor",
                "destination",
                "reference",
            ] {
                super::super::identifier(record, field).map_err(|_| UNAVAILABLE)?;
            }
            if record["credential_epoch"].as_u64().is_none_or(|n| n == 0)
                || record["grant_epoch"].as_u64().is_none()
                || record["expires_at_ms"].as_u64().is_none()
            {
                return Err(UNAVAILABLE.into());
            }
            let scope = record["tenant"] == self.0.tenant
                && record["principal"] == self.0.principal
                && record["grant_ref"] == self.0.grant_ref
                && record["actor"] == actor
                && record["destination"] == *destination;
            if !scope {
                continue;
            }
            let name = record["reference"].as_str().ok_or(UNAVAILABLE)?;
            if !seen.insert(name.to_owned()) {
                return Err(UNAVAILABLE.into());
            }
            if !allowed.contains(name) {
                continue;
            }
            if record["grant_epoch"] != self.0.epoch
                || record["expires_at_ms"].as_u64().is_none_or(|n| n <= now)
            {
                return Err(UNAVAILABLE.into());
            }
            let secret = record["value"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 16_384 && !s.contains('\0'))
                .ok_or(UNAVAILABLE)?;
            bindings.push((
                name.to_owned(),
                record["credential_epoch"].as_u64().ok_or(UNAVAILABLE)?,
                self.identity_commitment(&serde_json::json!(["credential", actor, name, secret]))?,
            ));
            resolved.insert(name.to_owned(), secret.to_owned());
        }
        if !allowed.iter().all(|name| resolved.contains_key(name)) {
            return Err(UNAVAILABLE.into());
        }
        // A concurrent grant revocation/epoch change stops this launch. Existing
        // trusted workers retain their separately declared trust limitation.
        self.check_current()?;
        Ok(Some(CredentialEnvironment {
            values: resolved,
            bindings,
        }))
    }
}
