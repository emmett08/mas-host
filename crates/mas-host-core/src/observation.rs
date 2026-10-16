//! Read-only projections for live displays of the retained local execution.
//!
//! The protocol owns its read-only projection and storage compatibility. This
//! module joins that public snapshot with host-owned execution checkpoints.
//! Observation performs no protocol transition.
use super::{error, now_ms, run_directory, Result};
use mas_protocol_codec::observation::Observer;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

fn reader(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        return Err("unknown run or run initialization incomplete".into());
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(error)?;
    db.busy_timeout(Duration::from_millis(250)).map_err(error)?;
    Ok(db)
}

fn optional_json(value: Option<String>) -> Result<Value> {
    value
        .map(|source| serde_json::from_str(&source).map_err(error))
        .transpose()
        .map(|value| value.unwrap_or(Value::Null))
}

/// Observe committed state while the run's execution owner remains active.
///
/// Payloads, operation inputs/outputs and evidence require explicit opt-in.
/// Each database has a separate read snapshot; a concurrently committed host
/// checkpoint and protocol message can therefore appear in adjacent polls.
/// No owner lock, migration, expiry reconciliation, receive or ack is performed.
pub fn observe(state_dir: &Path, run_id: &str, include_payloads: bool) -> Result<Value> {
    let directory = run_directory(state_dir, run_id)?;
    let mut journal = reader(&directory.join("execution.sqlite"))?;
    let journal = journal.transaction().map_err(error)?;
    let metadata: Option<(i64, String, bool)> = journal
        .query_row(
            "SELECT version,run_id,cancelled FROM metadata WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    let (version, recorded_run, cancelled) = metadata.ok_or("run initialization incomplete")?;
    if version != 1 || recorded_run != run_id {
        return Err("incompatible execution journal".into());
    }
    let steps = {
        let mut statement = journal
            .prepare("SELECT id,parent,key,kind,actor,target,state,input,output,evidence FROM steps ORDER BY rowid")
            .map_err(error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    json!({"id":row.get::<_,String>(0)?,"parent":row.get::<_,String>(1)?,
                        "key":row.get::<_,String>(2)?,"kind":row.get::<_,String>(3)?,
                        "actor":row.get::<_,String>(4)?,"target":row.get::<_,String>(5)?,
                        "state":row.get::<_,String>(6)?}),
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .map_err(error)?;
        let mut steps = Vec::new();
        for row in rows {
            let (mut step, input, output, evidence) = row.map_err(error)?;
            if include_payloads
                && super::revision::step_public(&journal, step["id"].as_str().unwrap())?
            {
                step["input"] = serde_json::from_str(&input).map_err(error)?;
                step["output"] = optional_json(output)?;
                step["evidence"] = optional_json(evidence)?;
            }
            steps.push(step);
        }
        steps
    };
    let mut revision = super::revision::audit(&journal)?;
    if !revision.is_null() && cancelled && revision["status"] == "in_progress" {
        revision["status"] = json!("cancelled");
    }
    journal.commit().map_err(error)?;

    let mut protocol =
        Observer::open(directory.join("protocol.sqlite"))?.snapshot(run_id, include_payloads)?;
    if !revision.is_null() {
        let db = reader(&directory.join("execution.sqlite"))?;
        if let Some(messages) = protocol["messages"].as_array_mut() {
            for delivery in messages {
                let task = delivery["task_id"].as_str().unwrap_or("");
                if !super::revision::step_public(&db, task)? {
                    delivery.as_object_mut().unwrap().remove("payload");
                    delivery["payload_withheld"] = json!(true);
                }
            }
        }
        if let Some(calls) = protocol["calls"].as_array_mut() {
            for call in calls {
                if !super::revision::step_public(&db, call["id"].as_str().unwrap_or(""))? {
                    call.as_object_mut().unwrap().remove("evidence");
                    call["payload_withheld"] = json!(true);
                }
            }
        }
        let uncertain: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM steps WHERE kind='effect' AND state='unknown')",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if !cancelled && uncertain && revision["status"] == "in_progress" {
            revision["status"] = json!("blocked");
        }
        if !cancelled
            && revision["status"] == "in_progress"
            && (protocol["run"]["expired"] == true
                || protocol["run"]["deadline"]
                    .as_f64()
                    .is_some_and(|v| v * 1000.0 <= now_ms().unwrap_or(u64::MAX) as f64))
        {
            revision["status"] = json!("expired");
        }
    }
    Ok(json!({"version":"mas-host-observation/1","run_id":run_id,
        "observed_at_ms":now_ms()?,"payloads_included":include_payloads,"cancelled":cancelled,
        "run":protocol["run"],"messages":protocol["messages"],"steps":steps,
        "calls":protocol["calls"],"tasks":protocol["tasks"],"revision":super::revision::public_audit(revision,include_payloads)}))
}
