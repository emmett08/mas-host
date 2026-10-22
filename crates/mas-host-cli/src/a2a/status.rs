//! Bounded service-state materialisation, independent of retained application output.
use super::{fields, text, Caller, Result, RpcError, RpcResult, Service};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(super) const MAX_BYTES: usize = 16_384;

/// Materialise only bounded service facts whenever their owning task changes.
/// Triggers share the task transaction, including restart's unknown transition.
pub(super) fn initialise(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_status(
            id TEXT PRIMARY KEY,principal TEXT NOT NULL,agent TEXT NOT NULL,
            state TEXT NOT NULL,cancelled INTEGER NOT NULL,output_retained INTEGER NOT NULL,
            revision INTEGER NOT NULL,materialised_at_ms INTEGER NOT NULL);
         CREATE TRIGGER IF NOT EXISTS task_status_insert AFTER INSERT ON tasks BEGIN
            INSERT INTO task_status VALUES(NEW.id,NEW.principal,NEW.agent,NEW.state,
                NEW.cancelled,NEW.output IS NOT NULL,1,CAST(unixepoch('subsec')*1000 AS INTEGER));
         END;
         CREATE TRIGGER IF NOT EXISTS task_status_update
         AFTER UPDATE OF state,cancelled,output ON tasks BEGIN
            UPDATE task_status SET state=NEW.state,cancelled=NEW.cancelled,
                output_retained=NEW.output IS NOT NULL,revision=revision+1,
                materialised_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER) WHERE id=NEW.id;
         END;
         INSERT OR IGNORE INTO task_status SELECT id,principal,agent,state,cancelled,
            output IS NOT NULL,1,CAST(unixepoch('subsec')*1000 AS INTEGER) FROM tasks;",
    )
    .map_err(super::error)
}

impl Service {
    pub(super) fn task_status(&self, caller: &Caller, request: &Value) -> RpcResult<Value> {
        fields(request, &["id"], &[])?;
        let id = text(request, "id")?;
        caller.current().map_err(|_| unavailable())?;
        let db = self.db.lock().map_err(|_| unavailable())?;
        // Exactly one primary-key lookup. Application input/output, diagnostic,
        // protocol history and provider state are never loaded or refreshed.
        let row = db
            .query_row(
                "SELECT agent,state,cancelled,output_retained,revision,materialised_at_ms
             FROM task_status WHERE id=? AND principal=?",
                params![id, caller.owner],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        if !caller.agents.contains(&row.0) {
            return Err(unavailable());
        }
        if let Some(session) = &caller.session {
            let fingerprint: String = db
                .query_row(
                    "SELECT fingerprint FROM governed_tasks WHERE id=?",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|_| unavailable())?;
            if fingerprint != session.fingerprint() {
                return Err(unavailable());
            }
        }
        drop(db);
        let observed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_millis();
        let view = json!({"version":"mas-host-task-status/1","id":id,
            "agent":row.0,"bundleDigest":self.descriptor["bundle_digest"],
            "serviceState":row.1,"cancelRequested":row.2,
            "hostSettlement":if row.1=="completed" {"completed"} else {"not-established"},
            "externalExecution":"not-assessed",
            "payloadAvailability":if row.3 {"retained-requires-authorised-read"} else {"not-retained-in-service"},
            "evidenceCoverage":"not-assessed","applicationAssessment":"not-assessed",
            "sourceEvidenceFreshness":"not-assessed",
            "materialisation":{"revision":row.4,"updatedAtMs":row.5,"observedAtMs":observed,
                "consistency":"committed-service-snapshot","incomplete":false},
            "executionPermission":false});
        if view.to_string().len() > MAX_BYTES {
            return Err(unavailable());
        }
        if let Some(session) = &caller.session {
            let directory = self.state.join("runs").join(&id);
            if self.configuration.source_reads_supported()
                && (row.1 != "submitted" || directory.join("execution.sqlite").is_file())
            {
                mas_host_core::governance::read_governed_root_delivery(
                    &directory, session, &row.0, None, &view,
                )
                .map_err(|_| unavailable())?;
            } else {
                session
                    .check_status_value(&row.0, &view)
                    .map_err(|_| unavailable())?;
            }
        }
        caller.current().map_err(|_| unavailable())?;
        Ok(view)
    }
}

fn unavailable() -> RpcError {
    RpcError(-32001, "task_status_unavailable")
}

#[cfg(test)]
mod tests {
    use super::initialise;
    use rusqlite::Connection;

    #[test]
    fn materialisation_is_atomic_and_never_contains_application_values() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY,principal TEXT,agent TEXT,state TEXT,cancelled INTEGER,output TEXT);").unwrap();
        initialise(&db).unwrap();
        db.execute(
            "INSERT INTO tasks VALUES('id','owner','actor','submitted',0,NULL)",
            [],
        )
        .unwrap();
        let revision = |db: &Connection| {
            db.query_row(
                "SELECT revision,state,output_retained FROM task_status WHERE id='id'",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, bool>(2)?,
                    ))
                },
            )
            .unwrap()
        };
        assert_eq!(revision(&db), (1, "submitted".into(), false));
        let tx = db.transaction().unwrap();
        tx.execute(
            "UPDATE tasks SET state='completed',output='private-marker'",
            [],
        )
        .unwrap();
        assert_eq!(revision(&tx), (2, "completed".into(), true));
        tx.rollback().unwrap();
        assert_eq!(revision(&db), (1, "submitted".into(), false));
        db.execute("UPDATE tasks SET state='unknown'", []).unwrap();
        assert_eq!(revision(&db), (2, "unknown".into(), false));
        initialise(&db).unwrap();
        assert_eq!(revision(&db), (2, "unknown".into(), false));
    }
}
