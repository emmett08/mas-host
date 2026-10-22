//! Bind governed namespaces and original inputs inside the existing host journal.
use super::GovernedSession;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
type Result<T> = std::result::Result<T, String>;

pub(crate) fn is_governed(db: &Connection) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='governed_run')",
        [],
        |r| r.get(0),
    )
    .map_err(|_| "governed-storage-unavailable".into())
}

pub(crate) fn require_ordinary(db: &Connection) -> Result<()> {
    if is_governed(db)? {
        return Err("governed-retrieval-requires-authenticated-session".into());
    }
    Ok(())
}

pub(crate) fn open_namespace(db: &Connection, session: Option<&GovernedSession>) -> Result<()> {
    if is_governed(db)? {
        let session = session.ok_or("governed-namespace-cannot-use-ordinary-execution")?;
        let previous: String = db
            .query_row(
                "SELECT fingerprint FROM governed_run WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "governed-storage-unavailable")?;
        if previous != session.fingerprint() {
            return Err("governed-namespace-identity-conflict".into());
        }
        return Ok(());
    }
    if let Some(session) = session {
        let started: bool = db
            .query_row("SELECT EXISTS(SELECT 1 FROM metadata)", [], |r| r.get(0))
            .map_err(|_| "governed-storage-unavailable")?;
        if started {
            return Err("ordinary-namespace-cannot-become-governed".into());
        }
        db.execute_batch("CREATE TABLE governed_run(singleton INTEGER PRIMARY KEY CHECK(singleton=1),fingerprint TEXT NOT NULL);
            CREATE TABLE governed_inputs(id TEXT PRIMARY KEY,original_commitment TEXT NOT NULL,decision TEXT NOT NULL);
            CREATE TABLE governed_completions(id TEXT PRIMARY KEY,original_commitment TEXT NOT NULL,body TEXT NOT NULL);")
            .map_err(|_|"governed-storage-unavailable")?;
        db.execute(
            "INSERT INTO governed_run VALUES(1,?)",
            [session.fingerprint()],
        )
        .map_err(|_| "governed-storage-unavailable")?;
    }
    Ok(())
}

pub(crate) fn bind_input(
    db: &Connection,
    id: &str,
    commitment: &str,
    decision: &Value,
) -> Result<()> {
    let previous: Option<String> = db
        .query_row(
            "SELECT original_commitment FROM governed_inputs WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "governed-storage-unavailable")?;
    if let Some(previous) = previous {
        if previous != commitment {
            return Err("governed-original-input-conflict".into());
        }
        return Ok(());
    }
    db.execute(
        "INSERT INTO governed_inputs VALUES(?,?,?)",
        params![id, commitment, decision.to_string()],
    )
    .map_err(|_| "governed-storage-unavailable")?;
    Ok(())
}

pub(crate) fn save_completion(
    db: &Connection,
    id: &str,
    original_commitment: &str,
    completion: &Value,
) -> Result<()> {
    let previous: Option<String> = db
        .query_row(
            "SELECT body FROM governed_completions WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "governed-storage-unavailable")?;
    let body = completion.to_string();
    if let Some(previous) = previous {
        if previous != body {
            return Err("governed-completion-conflict".into());
        }
    } else {
        db.execute(
            "INSERT INTO governed_completions VALUES(?,?,?)",
            params![id, original_commitment, body],
        )
        .map_err(|_| "governed-storage-unavailable")?;
    }
    Ok(())
}

pub(crate) fn check_completion(db: &Connection, id: &str, original_commitment: &str) -> Result<()> {
    let previous: String = db
        .query_row(
            "SELECT original_commitment FROM governed_completions WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "governed-completion-unavailable")?;
    if previous != original_commitment {
        return Err("governed-completion-conflict".into());
    }
    Ok(())
}

pub(super) fn completion(db: &Connection, id: &str) -> Result<Value> {
    let body: String = db
        .query_row(
            "SELECT body FROM governed_completions WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "governed-completion-unavailable")?;
    mas_protocol_codec::parse_json(&body).map_err(|_| "governed-completion-invalid".into())
}
