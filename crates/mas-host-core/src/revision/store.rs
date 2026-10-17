//! Append-only audit records and atomic invocation reservations in the execution journal.
use super::super::{error, fields, lock, text, Inner, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(super) fn create(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS revision_plan (singleton INTEGER PRIMARY KEY CHECK(singleton=1),body TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS revision_records (kind TEXT NOT NULL,id TEXT NOT NULL,body TEXT NOT NULL,readers TEXT NOT NULL,PRIMARY KEY(kind,id));
        CREATE TABLE IF NOT EXISTS revision_contexts (id TEXT PRIMARY KEY,body TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS revision_terminal (singleton INTEGER PRIMARY KEY CHECK(singleton=1),body TEXT NOT NULL);
        CREATE TRIGGER IF NOT EXISTS immutable_revision_plan_update BEFORE UPDATE ON revision_plan BEGIN SELECT RAISE(ABORT,'immutable revision plan'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_plan_delete BEFORE DELETE ON revision_plan BEGIN SELECT RAISE(ABORT,'immutable revision plan'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_terminal_update BEFORE UPDATE ON revision_terminal BEGIN SELECT RAISE(ABORT,'immutable terminal selection'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_terminal_delete BEFORE DELETE ON revision_terminal BEGIN SELECT RAISE(ABORT,'immutable terminal selection'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_records_update BEFORE UPDATE ON revision_records BEGIN SELECT RAISE(ABORT,'immutable audit record'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_records_delete BEFORE DELETE ON revision_records BEGIN SELECT RAISE(ABORT,'immutable audit record'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_contexts_update BEFORE UPDATE ON revision_contexts BEGIN SELECT RAISE(ABORT,'immutable invocation context'); END;
        CREATE TRIGGER IF NOT EXISTS immutable_revision_contexts_delete BEFORE DELETE ON revision_contexts BEGIN SELECT RAISE(ABORT,'immutable invocation context'); END;") .map_err(error)
}

impl Inner {
    pub(crate) fn revision_mode(&self) -> Result<bool> {
        let db = lock(&self.journal)?;
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='revision_plan')",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if !exists {
            return Ok(false);
        }
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM revision_plan WHERE singleton=1)",
            [],
            |r| r.get(0),
        )
        .map_err(error)
    }
    pub(super) fn freeze_revision(&self, plan: &Value, input: &Value) -> Result<()> {
        let db = lock(&self.journal)?;
        create(&db)?;
        let started: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM steps) AND NOT EXISTS(SELECT 1 FROM revision_plan)",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if started {
            return Err("ordinary run cannot be converted to revision mode".into());
        }
        let body = json!({"plan":plan,"input":input}).to_string();
        db.execute(
            "INSERT OR IGNORE INTO revision_plan(singleton,body) VALUES(1,?)",
            [&body],
        )
        .map_err(error)?;
        let old: String = db
            .query_row(
                "SELECT body FROM revision_plan WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if old != body {
            return Err("revision plan or source input differs from retained run".into());
        }
        Ok(())
    }
    pub(super) fn retained_record(
        &self,
        kind: &str,
        id: &str,
        readers: &Value,
    ) -> Result<Option<Value>> {
        let expected_readers = readers.to_string();
        let db = lock(&self.journal)?;
        let retained: Option<(String, String)> = db
            .query_row(
                "SELECT body,readers FROM revision_records WHERE kind=? AND id=?",
                params![kind, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(error)?;
        retained
            .map(|(body, previous)| {
                if previous != expected_readers {
                    return Err("audit disclosure policy differs from original".into());
                }
                serde_json::from_str(&body).map_err(error)
            })
            .transpose()
    }
    pub(super) fn record(&self, kind: &str, record: &Value, readers: &Value) -> Result<()> {
        let id = text(record, "id")?;
        let body = record.to_string();
        let readers = readers.to_string();
        let db = lock(&self.journal)?;
        db.execute(
            "INSERT OR IGNORE INTO revision_records(kind,id,body,readers) VALUES(?,?,?,?)",
            params![kind, id, body, readers],
        )
        .map_err(error)?;
        let old: (String, String) = db
            .query_row(
                "SELECT body,readers FROM revision_records WHERE kind=? AND id=?",
                params![kind, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(error)?;
        if old != (body, readers) {
            return Err(
                "audit identity cannot be reused with changed content or disclosure policy".into(),
            );
        }
        Ok(())
    }
    pub(super) fn context(&self, id: &str) -> Result<Option<Value>> {
        let db = lock(&self.journal)?;
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='revision_contexts')",[],|r|r.get(0)).map_err(error)?;
        if !exists {
            return Ok(None);
        }
        let body: Option<String> = db
            .query_row("SELECT body FROM revision_contexts WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(error)?;
        body.map(|s| serde_json::from_str(&s).map_err(error))
            .transpose()
    }
    pub(super) fn bind_context(&self, id: &str, context: &Value) -> Result<()> {
        let mut db = lock(&self.journal)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let body = context.to_string();
        if let Some(old) = tx
            .query_row("SELECT body FROM revision_contexts WHERE id=?", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(error)?
        {
            if old != body {
                return Err("invocation context or policy differs from original".into());
            }
        } else {
            // A live check before this transaction cannot authorise a new
            // reservation after a concurrent cancellation or elapsed deadline.
            // Existing context replay above retains its original identity.
            let cancelled: bool = tx
                .query_row(
                    "SELECT cancelled FROM metadata WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(error)?;
            if cancelled {
                return Err("run cancelled; no new revision reservation".into());
            }
            if super::super::now_ms()? >= self.deadline_ms {
                return Err("original run deadline expired".into());
            }
            let frozen: String = tx
                .query_row(
                    "SELECT body FROM revision_plan WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(error)?;
            let plan: Value = serde_json::from_str(&frozen).map_err(error)?;
            let used: i64 = tx
                .query_row("SELECT COUNT(*) FROM revision_contexts", [], |r| r.get(0))
                .map_err(error)?;
            if used as u64 >= plan["plan"]["max_invocations"].as_u64().unwrap_or(0) {
                return Err("revision invocation budget exhausted".into());
            }
            tx.execute(
                "INSERT INTO revision_contexts(id,body) VALUES(?,?)",
                params![id, body],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)
    }
    pub(crate) fn inherit_context(
        &self,
        parent: &str,
        child: &str,
        target: &str,
        input: &Value,
    ) -> Result<()> {
        if let Some(mut context) = self.context(parent)? {
            if !context["policy"]["allowed_agents"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == target))
            {
                return Err("invocation policy forbids this peer".into());
            }
            if !super::context::permitted(&super::context::readers(&context["readers"])?, target) {
                return Err("derived input disclosure forbids this peer".into());
            }
            context["actor"] = json!(target);
            context["input"] = input.clone();
            context["parent"] = json!(parent);
            self.bind_context(child, &context)?;
        }
        Ok(())
    }
    pub(crate) fn check_effect_policy(&self, invocation: &str, descriptor: &Value) -> Result<()> {
        if let Some(context) = self.context(invocation)? {
            // Inference is the only admitted external effect in the revision loop.
            // Native workers remain trusted; this constrains host-mediated effects.
            if self
                .agents
                .get(context["actor"].as_str().unwrap_or(""))
                .is_none_or(|a| a.configuration["kind"] != "llm")
                || descriptor["format"] != "mas-inference/2"
                || !context["policy"]["allowed_effects"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|k| k == &descriptor["format"]))
            {
                return Err("revision invocation forbids non-inference effects".into());
            }
        }
        Ok(())
    }
    pub(crate) fn note_revision_terminal_budget(
        &self,
        id: &str,
        failure: &mas_protocol_codec::admission::AdmissionError,
    ) -> Result<()> {
        if failure.code != mas_protocol_codec::admission::AdmissionCode::BudgetExceeded
            || !self.revision_mode()?
        {
            return Ok(());
        }
        let readers = self
            .context(id)?
            .map(|v| v["readers"].clone())
            .unwrap_or_else(|| json!([]));
        let content = json!({"task_id":id,"phase":"terminal","admission":failure.to_value()});
        let evidence=mas_protocol_codec::assessment::Evidence::from_value(&json!({"schema_version":1,"id":format!("budget-{id}"),"content_sha256":super::canonical_digest(&content),"content":content})).map_err(error)?;
        self.record("evidence", &evidence.to_value(), &readers)
    }
    pub(super) fn revision_exhausted(&self, key: &str) -> Result<bool> {
        self.check_live()?;
        if self.has_unknown()? {
            return Ok(false);
        }
        let id = self.step_id("", key, "request");
        let db = lock(&self.journal)?;
        let frozen: String = db
            .query_row(
                "SELECT body FROM revision_plan WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        let frozen: Value = serde_json::from_str(&frozen).map_err(error)?;
        let used: i64 = db
            .query_row("SELECT COUNT(*) FROM revision_contexts", [], |r| r.get(0))
            .map_err(error)?;
        let reserved: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM revision_contexts WHERE id=?)",
                [&id],
                |r| r.get(0),
            )
            .map_err(error)?;
        if !reserved && used as u64 >= frozen["plan"]["max_invocations"].as_u64().unwrap_or(0) {
            return Ok(true);
        }
        let terminal_budget:bool=db.query_row("WITH RECURSIVE descendants(id) AS (SELECT ? UNION ALL SELECT s.id FROM steps s JOIN descendants d ON s.parent=d.id) SELECT EXISTS(SELECT 1 FROM revision_records r JOIN descendants d ON r.id='budget-'||d.id WHERE r.kind='evidence')",[&id],|r|r.get(0)).map_err(error)?;
        if terminal_budget {
            return Ok(true);
        }
        let mut statement=db.prepare("WITH RECURSIVE descendants(id) AS (SELECT ? UNION ALL SELECT s.id FROM steps s JOIN descendants d ON s.parent=d.id) SELECT evidence FROM steps WHERE id IN (SELECT id FROM descendants) AND state='rejected' AND evidence IS NOT NULL").map_err(error)?;
        let rows = statement
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(error)?;
        for row in rows {
            let evidence: Value = serde_json::from_str(&row.map_err(error)?).map_err(error)?;
            if evidence["admission"]["code"] == "budget_exceeded"
                || evidence["host_budget"]["code"] == "call_budget_exceeded"
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn terminal_revision(&self, value: &Value) -> Result<()> {
        fields(value, &["status", "selected"], &[])?;
        let mut db = lock(&self.journal)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let body = value.to_string();
        let prior: Option<String> = tx
            .query_row(
                "SELECT body FROM revision_terminal WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(old) = prior {
            if old != body {
                return Err("terminal revision result is immutable".into());
            }
        } else {
            // Completion and cancellation share one journal commit order. A
            // prior live check does not authorise success after a later stop.
            let cancelled: bool = tx
                .query_row(
                    "SELECT cancelled FROM metadata WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(error)?;
            if cancelled {
                return Err("run cancelled; no new revision completion".into());
            }
            if super::super::now_ms()? >= self.deadline_ms {
                return Err("original run deadline expired".into());
            }
            tx.execute(
                "INSERT INTO revision_terminal(singleton,body) VALUES(1,?)",
                [&body],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)
    }
}

pub(super) fn audit(db: &Connection) -> Result<Value> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='revision_plan')",
            [],
            |r| r.get(0),
        )
        .map_err(error)?;
    if !exists {
        return Ok(Value::Null);
    }
    let frozen: Option<String> = db
        .query_row(
            "SELECT body FROM revision_plan WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    let Some(frozen) = frozen else {
        return Ok(Value::Null);
    };
    let frozen: Value = serde_json::from_str(&frozen).map_err(error)?;
    let mut statement = db
        .prepare("SELECT kind,id,body,readers FROM revision_records ORDER BY rowid")
        .map_err(error)?;
    let records=statement.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).map_err(error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(error)?.into_iter().map(|(kind,id,body,readers)|Ok(json!({"kind":kind,"id":id,"record":serde_json::from_str::<Value>(&body).map_err(error)?,"readers":serde_json::from_str::<Value>(&readers).map_err(error)?,"sha256":super::canonical_digest(&serde_json::from_str::<Value>(&body).map_err(error)?)}))).collect::<Result<Vec<_>>>()?;
    let mut statement = db
        .prepare("SELECT id,body FROM revision_contexts ORDER BY rowid")
        .map_err(error)?;
    let contexts = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(error)?
        .into_iter()
        .map(|(id, body)| {
            let mut v: Value = serde_json::from_str(&body).map_err(error)?;
            v["id"] = json!(id);
            Ok(v)
        })
        .collect::<Result<Vec<_>>>()?;
    let terminal: Option<String> = db
        .query_row(
            "SELECT body FROM revision_terminal WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    let terminal = terminal
        .map(|s| serde_json::from_str::<Value>(&s).map_err(error))
        .transpose()?
        .unwrap_or_else(|| json!({"status":"in_progress","selected":null}));
    Ok(
        json!({"version":"mas-host-revision-audit/1","budget":{"max_revisions":frozen["plan"]["max_revisions"],"max_invocations":frozen["plan"]["max_invocations"],"used_invocations":contexts.len()},"rubric":frozen["plan"]["rubric"],"status":terminal["status"],"selected":terminal["selected"],"candidates":records.iter().filter(|r|r["kind"]=="candidate").map(|r|r["record"].clone()).collect::<Vec<_>>(),"assessments":records.iter().filter(|r|r["kind"]=="assessment").map(|r|r["record"].clone()).collect::<Vec<_>>(),"records":records,"contexts":contexts}),
    )
}
