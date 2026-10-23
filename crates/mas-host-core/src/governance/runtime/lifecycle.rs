//! Retire a governed namespace against an independently retained control ledger.
use super::GovernedSession;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};

type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-lifecycle-unavailable";

fn ledger(path: &Path, create: bool) -> Result<Connection> {
    if !path.is_absolute() {
        return Err(UNAVAILABLE.into());
    }
    if create && !path.exists() {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file.sync_all().map_err(|_| UNAVAILABLE)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(UNAVAILABLE.into()),
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(UNAVAILABLE.into());
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|_| UNAVAILABLE)?;
    db.execute_batch(
        "PRAGMA synchronous=EXTRA; PRAGMA journal_mode=DELETE;
        CREATE TABLE IF NOT EXISTS namespaces(id TEXT PRIMARY KEY, run_id TEXT NOT NULL,
        fingerprint TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('available','retired')),
        expires_at_ms INTEGER, held INTEGER NOT NULL CHECK(held IN (0,1)), reason TEXT NOT NULL,
        retired_at_ms INTEGER, UNIQUE(run_id,fingerprint));",
    )
    .map_err(|_| UNAVAILABLE)?;
    Ok(db)
}

fn read_ledger(path: &Path) -> Result<Connection> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(UNAVAILABLE.into());
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|_| UNAVAILABLE)?;
    Ok(db)
}

fn namespace(session: &GovernedSession, run_id: &str, nonce: &str) -> Result<String> {
    session.identity_commitment(&json!([
        "mas-governed-lifecycle/1",
        run_id,
        nonce,
        session.fingerprint()
    ]))
}

/// Register only a newly created namespace. An existing namespace must find its
/// current control record: restoring execution state alone cannot recreate it.
pub(crate) fn check_namespace(
    session: &GovernedSession,
    run_id: &str,
    nonce: &str,
    fresh: bool,
) -> Result<()> {
    let Some(path) = session.lifecycle_file() else {
        return Ok(());
    };
    let db = if fresh {
        ledger(path, true)?
    } else {
        read_ledger(path)?
    };
    let id = namespace(session, run_id, nonce)?;
    if fresh {
        db.execute(
            "INSERT OR IGNORE INTO namespaces VALUES(?,?,?,'available',NULL,0,'',NULL)",
            params![id, run_id, session.fingerprint()],
        )
        .map_err(|_| UNAVAILABLE)?;
    }
    check_record(&db, session, run_id, &id)
}

fn check_record(db: &Connection, session: &GovernedSession, run_id: &str, id: &str) -> Result<()> {
    let row: Option<(String, String, String, Option<i64>)> = db
        .query_row(
            "SELECT run_id,fingerprint,status,expires_at_ms FROM namespaces WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| UNAVAILABLE)?;
    let Some((saved_run, fingerprint, status, expiry)) = row else {
        return Err(UNAVAILABLE.into());
    };
    if saved_run != run_id || fingerprint != session.fingerprint() {
        return Err(UNAVAILABLE.into());
    }
    if status == "retired" {
        return Err("governed-payload-deleted".into());
    }
    if status != "available" || expiry.is_some_and(|expiry| expiry < 0) {
        return Err(UNAVAILABLE.into());
    }
    if expiry.is_some_and(|expiry| crate::now_ms().map_or(true, |now| now >= expiry as u64)) {
        return Err("governed-payload-expired".into());
    }
    Ok(())
}

fn retained_identity(directory: &Path, session: &GovernedSession) -> Result<(String, String)> {
    let db = Connection::open_with_flags(
        directory.join("execution.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    let fingerprint: String = db
        .query_row(
            "SELECT fingerprint FROM governed_run WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| UNAVAILABLE)?;
    if fingerprint != session.fingerprint() {
        return Err(UNAVAILABLE.into());
    }
    db.query_row(
        "SELECT run_id,nonce FROM metadata WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|_| UNAVAILABLE.into())
}
pub(super) fn custody_identity(
    directory: &Path,
    session: &GovernedSession,
) -> Result<serde_json::Value> {
    let (run, nonce) = retained_identity(directory, session)?;
    let directory = std::fs::canonicalize(directory).map_err(|_| UNAVAILABLE)?;
    Ok(json!({"run":run,"nonce":nonce,"directory":directory,"fingerprint":session.fingerprint()}))
}
pub(super) fn with_custody_admission<T>(
    identities: &[serde_json::Value],
    session: &GovernedSession,
    read: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let identities = identities
        .iter()
        .filter(|v| !v.is_null())
        .collect::<Vec<_>>();
    if identities.is_empty() {
        return read();
    }
    let path = session
        .lifecycle_file()
        .ok_or("quarantine-namespace-requires-lifecycle-ledger")?;
    let mut db = read_ledger(path)?;
    let tx = db.transaction().map_err(|_| UNAVAILABLE)?;
    for identity in identities {
        let directory = Path::new(identity["directory"].as_str().ok_or(UNAVAILABLE)?);
        if custody_identity(directory, session)? != *identity {
            return Err(UNAVAILABLE.into());
        }
        let run = identity["run"].as_str().ok_or(UNAVAILABLE)?;
        let nonce = identity["nonce"].as_str().ok_or(UNAVAILABLE)?;
        check_record(&tx, session, run, &namespace(session, run, nonce)?)?;
    }
    let result = read()?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    Ok(result)
}
/// Restrict all dependent views when the namespace's raw source custody is
/// retired/expired. This only shortens access; it preserves execution journals
/// and unresolved effects and does not require taking the payload owner lock.
pub(super) fn expire_for_custody(
    directory: &Path,
    session: &GovernedSession,
    expiry: u64,
) -> Result<()> {
    let path = session
        .lifecycle_file()
        .ok_or("quarantine-namespace-requires-lifecycle-ledger")?;
    let (run, nonce) = retained_identity(directory, session)?;
    let id = namespace(session, &run, &nonce)?;
    let mut db = ledger(path, false)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    let (state, old): (String, Option<i64>) = tx
        .query_row(
            "SELECT status,expires_at_ms FROM namespaces WHERE id=? AND fingerprint=?",
            params![id, session.fingerprint()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| UNAVAILABLE)?;
    if state != "retired" {
        if state != "available" {
            return Err(UNAVAILABLE.into());
        }
        let expiry = i64::try_from(expiry).map_err(|_| UNAVAILABLE)?;
        if old.is_none_or(|old| expiry < old) {
            tx.execute(
                "UPDATE namespaces SET expires_at_ms=? WHERE id=?",
                params![expiry, id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
    }
    session.check_current()?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    Ok(())
}

/// Check lifecycle before any retained delivery or further execution. Missing
/// control state fails closed; it is never reconstructed from an old backup.
pub(crate) fn check_directory(directory: &Path, session: &GovernedSession) -> Result<()> {
    if session.lifecycle_file().is_none() {
        return Ok(());
    }
    let (run, nonce) = retained_identity(directory, session)?;
    check_namespace(session, &run, &nonce, false)
}

/// Hold lifecycle admission through an existing journal mutation. Retirement
/// cannot commit its tombstone until this admitted mutation has finished.
/// The lock order is control ledger, then the payload journal.
pub(crate) fn with_admission<T>(
    directory: &Path,
    session: &GovernedSession,
    write: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let Some(path) = session.lifecycle_file() else {
        return write();
    };
    if path.starts_with(directory) {
        return Err("governed-lifecycle-ledger-must-be-independent".into());
    }
    let (run, nonce) = retained_identity(directory, session)?;
    let mut db = read_ledger(path)?;
    let tx = db.transaction().map_err(|_| UNAVAILABLE)?;
    check_record(&tx, session, &run, &namespace(session, &run, &nonce)?)?;
    let result = write()?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    Ok(result)
}

/// Trusted operator administration. The caller owns the host state and supplies
/// a currently authenticated session. A hold blocks destructive retirement; it
/// does not extend disclosure authority or expiry. No worker API exposes this.
pub fn govern_namespace_lifecycle(
    directory: &Path,
    session: &GovernedSession,
    action: &str,
    expires_at_ms: Option<u64>,
) -> Result<Value> {
    session.check_current()?;
    let _owner = crate::ownership(directory)?;
    let (run, nonce) = retained_identity(directory, session)?;
    let path = session
        .lifecycle_file()
        .ok_or("governed-lifecycle-not-configured")?;
    if path.starts_with(directory) {
        return Err("governed-lifecycle-ledger-must-be-independent".into());
    }
    let mut db = ledger(path, false)?;
    let id = namespace(session, &run, &nonce)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    let (status, held, old_expiry): (String, bool, Option<i64>) = tx
        .query_row(
            "SELECT status,held,expires_at_ms FROM namespaces WHERE id=? AND fingerprint=?",
            params![id, session.fingerprint()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| UNAVAILABLE)?;
    match action {
        "hold" | "release-hold" if status == "available" && expires_at_ms.is_none() => {
            tx.execute(
                "UPDATE namespaces SET held=? WHERE id=?",
                params![action == "hold", id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
        "expire" if status == "available" => {
            let expiry = expires_at_ms
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or("governed-lifecycle-expiry-required")?;
            if old_expiry.is_some_and(|old| expiry > old as u64) {
                return Err("governed-lifecycle-expiry-cannot-extend".into());
            }
            tx.execute(
                "UPDATE namespaces SET expires_at_ms=? WHERE id=?",
                params![expiry as i64, id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
        "retire" if expires_at_ms.is_none() => {
            require_settled_obligations(directory)?;
            if held {
                return Err("governed-lifecycle-held".into());
            }
            // The independent admission transaction prevents new case writes.
            // Key retirement precedes payload purge and cannot restore access
            // if a later namespace-store operation fails.
            super::quarantine::retire_namespace(session, directory, &run)?;
            tx.execute("UPDATE namespaces SET status='retired',reason='operator-retirement',retired_at_ms=COALESCE(retired_at_ms,?) WHERE id=?",
                params![i64::try_from(crate::now_ms()?).map_err(|_|UNAVAILABLE)?,id]).map_err(|_|UNAVAILABLE)?;
        }
        _ => return Err("governed-lifecycle-action-invalid".into()),
    }
    tx.commit().map_err(|_| UNAVAILABLE)?;
    // Control retirement commits first. Every failure below leaves the namespace
    // unavailable and can be retried without re-enabling execution.
    if action == "retire" {
        purge(directory)?;
    }
    Ok(
        json!({"version":"mas-governed-lifecycle-result/1","run_id":run,"action":action,
        "payload_disposition":if action=="retire" {"deleted"} else {"governed"},
        "execution_permission":"none","erasure_claim":"logical-retirement-and-local-store-purge"}),
    )
}

fn require_settled_obligations(directory: &Path) -> Result<()> {
    let db = Connection::open_with_flags(
        directory.join("execution.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    let mut statement=db.prepare("SELECT s.id,s.state,CASE WHEN length(CAST(s.output AS BLOB))<=65536 THEN s.output END,CASE WHEN length(CAST(s.evidence AS BLOB))<=65536 THEN s.evidence END,CASE WHEN length(CAST(c.body AS BLOB))<=65536 THEN c.body END FROM steps s LEFT JOIN governed_completions c ON c.id=s.id WHERE s.kind='effect'").map_err(|_|UNAVAILABLE)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|_| UNAVAILABLE)?;
    for (count, row) in rows.enumerate() {
        if count >= 1_000_000 {
            return Err(UNAVAILABLE.into());
        }
        let (id, state, output, evidence, completion) = row.map_err(|_| UNAVAILABLE)?;
        match state.as_str() {
            "prepared" | "failed" | "rejected" => (),
            "completed" | "resolved" => {
                let parse = |value: Option<String>| -> Result<Value> {
                    mas_protocol_codec::parse_json(
                        &value.ok_or("governed-lifecycle-reconciliation-required")?,
                    )
                    .map_err(|_| "governed-lifecycle-reconciliation-required".into())
                };
                let output = parse(output)?;
                let evidence = parse(evidence)?;
                let completion = parse(completion)?;
                super::resolver_record::validate_completion(&id, &output, &evidence, &completion)
                    .map_err(|_| "governed-lifecycle-reconciliation-required")?;
                if completion["external_execution"] == "unknown" {
                    return Err("governed-lifecycle-reconciliation-required".into());
                }
            }
            _ => return Err("governed-lifecycle-reconciliation-required".into()),
        }
    }
    Ok(())
}

fn purge(directory: &Path) -> Result<()> {
    let db = Connection::open_with_flags(
        directory.join("execution.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|_| UNAVAILABLE)?;
    db.execute_batch("PRAGMA secure_delete=ON; PRAGMA synchronous=EXTRA; BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS governed_retired_effects(id TEXT PRIMARY KEY,state TEXT NOT NULL,external_execution TEXT NOT NULL);
        INSERT OR IGNORE INTO governed_retired_effects SELECT s.id,s.state,
        CASE WHEN json_valid(c.body) AND json_extract(c.body,'$.external_execution') IN ('completed','unknown','not-sent','refused')
        THEN json_extract(c.body,'$.external_execution') ELSE 'unknown' END
        FROM steps s LEFT JOIN governed_completions c ON c.id=s.id WHERE s.kind='effect';
        UPDATE steps SET input='{}',output=NULL,evidence=NULL;
        DELETE FROM governed_inputs; DELETE FROM governed_completions;
        UPDATE metadata SET bundle='{}'; COMMIT;").map_err(|_|UNAVAILABLE)?;
    for table in [
        "governed_sources",
        "governed_exposures",
        "governed_contexts",
        "governed_pending",
        "governed_assessment_reviews",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
                [table],
                |r| r.get(0),
            )
            .map_err(|_| UNAVAILABLE)?;
        if exists {
            db.execute(&format!("DELETE FROM {table}"), [])
                .map_err(|_| UNAVAILABLE)?;
        }
    }
    for table in [
        "revision_plan",
        "revision_records",
        "revision_contexts",
        "revision_terminal",
    ] {
        db.execute(&format!("DROP TABLE IF EXISTS {table}"), [])
            .map_err(|_| UNAVAILABLE)?;
    }
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
        .map_err(|_| UNAVAILABLE)?;
    drop(db);
    // The protocol owns its schema. A retired namespace can never reopen its
    // protocol store, so retire the whole local file rather than editing tables.
    for name in [
        "protocol.sqlite",
        "protocol.sqlite-wal",
        "protocol.sqlite-shm",
    ] {
        let path = directory.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(UNAVAILABLE.into()),
        }
    }
    std::fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
