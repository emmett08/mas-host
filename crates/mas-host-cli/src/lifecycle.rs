//! Administer the lifecycle of an offline governed namespace.
use crate::require;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub(crate) fn execute(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let config = crate::config::read_json(Path::new(require(args, "--governance")?))?;
    let mut session = mas_host_core::governance::GovernedSession::from_value(
        &config,
        require(args, "--tenant")?,
        require(args, "--principal")?,
        require(args, "--purpose")?,
        require(args, "--grant")?,
    )?;
    if let Some(path) = args.get("--privacy-inspector") {
        session =
            crate::privacy_inspect::GovernedInspector::load(Path::new(path))?.attach(session)?;
    }
    let run = require(args, "--run")?;
    if run.is_empty()
        || run.len() > 96
        || !run
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || matches!(run, "." | "..")
    {
        return Err("invalid lifecycle run identifier".into());
    }
    let expiry = args
        .get("--expires-at-ms")
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| "invalid lifecycle expiry".to_owned())
        })
        .transpose()?;
    let directory = Path::new(require(args, "--state")?).join(run);
    let service_owner = args
        .get("--service-state")
        .map(|state| offline_service(Path::new(state), &directory))
        .transpose()?;
    let result = mas_host_core::governance::govern_namespace_lifecycle(
        &directory,
        &session,
        require(args, "--action")?,
        expiry,
    )?;
    if require(args, "--action")? == "retire" {
        if let Some(state) = args.get("--service-state") {
            purge_service(Path::new(state), run)?;
        }
    }
    drop(service_owner);
    Ok(result)
}

fn offline_service(state: &Path, directory: &Path) -> Result<std::fs::File, String> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    if directory.parent() != Some(state.join("runs").as_path()) {
        return Err("service-state-does-not-own-run".into());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(state.join("service.lock"))
        .map_err(|_| "service-lifecycle-unavailable")?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("service-must-be-offline-for-retirement".into());
    }
    Ok(file)
}

fn purge_service(state: &Path, run: &str) -> Result<(), String> {
    let db = rusqlite::Connection::open_with_flags(
        state.join("service.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| "service-lifecycle-unavailable")?;
    db.execute_batch("PRAGMA secure_delete=ON; PRAGMA synchronous=EXTRA;")
        .map_err(|_| "service-lifecycle-unavailable")?;
    let published:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='published_sources')",[],|r|r.get(0)).map_err(|_|"service-lifecycle-unavailable")?;
    if published {
        db.execute(
            "DELETE FROM published_sources WHERE json_extract(body,'$.task_id')=?",
            [run],
        )
        .map_err(|_| "service-lifecycle-unavailable")?;
    }
    db.execute("UPDATE tasks SET output=NULL,error=NULL WHERE id=?", [run])
        .map_err(|_| "service-lifecycle-unavailable")?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
        .map_err(|_| "service-lifecycle-unavailable")?;
    Ok(())
}
