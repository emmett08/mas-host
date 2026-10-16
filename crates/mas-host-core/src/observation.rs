//! Read-only projections for live displays of the retained local execution.
//!
//! Do not use Store::open/inspect here: they migrate or reconcile state. This
//! projection follows the native SQLite v2 schema of the pinned protocol crate.
//! It is an observation API, not another protocol participant or event journal.
use super::{error, now_ms, run_directory, Result};
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
            if include_payloads {
                step["input"] = serde_json::from_str(&input).map_err(error)?;
                step["output"] = optional_json(output)?;
                step["evidence"] = optional_json(evidence)?;
            }
            steps.push(step);
        }
        steps
    };
    journal.commit().map_err(error)?;

    let mut protocol = reader(&directory.join("protocol.sqlite"))?;
    let protocol = protocol.transaction().map_err(error)?;
    let application: i64 = protocol
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(error)?;
    let version: i64 = protocol
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(error)?;
    if application != 0x4d415331 || version != 2 {
        return Err("observation requires MAS native SQLite schema version 2".into());
    }
    let run = protocol
        .query_row(
            "SELECT config,deadline,message_count,byte_count,call_count,active,expired FROM runs WHERE id=?",
            [run_id],
            |row| Ok((row.get::<_,String>(0)?,row.get::<_,f64>(1)?,row.get::<_,i64>(2)?,
                row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,row.get::<_,bool>(6)?)),
        )
        .optional()
        .map_err(error)?
        .ok_or("unknown protocol run or run initialization incomplete")?;
    let config: Value = serde_json::from_str(&run.0).map_err(error)?;
    let messages = {
        let mut statement = protocol
            .prepare("SELECT seq,id,sender,recipient,content,received,acked FROM messages WHERE run=? ORDER BY seq")
            .map_err(error)?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, bool>(6)?,
                ))
            })
            .map_err(error)?;
        let mut messages = Vec::new();
        for row in rows {
            let (sequence, id, sender, recipient, content, received, acked) = row.map_err(error)?;
            let envelope = mas_protocol_codec::encode_message(
                &content,
                mas_protocol_codec::MAX_BYTES,
                mas_protocol_codec::MAX_DEPTH,
            )
            .and_then(|wire| {
                mas_protocol_codec::parse_message(
                    &wire,
                    mas_protocol_codec::MAX_BYTES,
                    mas_protocol_codec::MAX_DEPTH,
                )
            })
            .map_err(|_| "invalid retained protocol envelope")?;
            if envelope.message_id != id
                || envelope.run_id != run_id
                || envelope.sender != sender
                || envelope.recipient != recipient
            {
                return Err("retained protocol envelope columns differ".into());
            }
            let mut message = json!({"sequence":sequence,"message_id":id,"run_id":run_id,
                "sender":sender,"recipient":recipient,"version":envelope.version,
                "kind":envelope.kind,"received":received,"acked":acked,
                "task_id":envelope.payload.get("task_id").and_then(Value::as_str),
                "reply_to":envelope.payload.get("reply_to").and_then(Value::as_str),
                "operation":envelope.payload.get("operation").and_then(Value::as_str)});
            if include_payloads {
                message["payload"] = Value::Object(envelope.payload);
            }
            messages.push(message);
        }
        messages
    };
    let calls = {
        let mut statement = protocol
            .prepare("SELECT id,actor,state,result FROM calls WHERE run=? ORDER BY rowid")
            .map_err(error)?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((
                    json!({"id":row.get::<_,String>(0)?,"actor":row.get::<_,String>(1)?,
                "state":row.get::<_,String>(2)?}),
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(error)?;
        let mut calls = Vec::new();
        for row in rows {
            let (mut call, evidence) = row.map_err(error)?;
            if include_payloads {
                call["evidence"] = optional_json(evidence)?;
            }
            calls.push(call);
        }
        calls
    };
    let tasks = {
        let mut statement = protocol
            .prepare("SELECT id,request_id,state,terminal_id,closed_at FROM tasks WHERE run=? ORDER BY rowid")
            .map_err(error)?;
        let tasks = statement
            .query_map([run_id], |row| {
                Ok(json!({"task_id":row.get::<_,String>(0)?,
            "request_id":row.get::<_,String>(1)?,"state":row.get::<_,String>(2)?,
            "terminal_message_id":row.get::<_,Option<String>>(3)?,
            "closed_at":row.get::<_,Option<f64>>(4)?}))
            })
            .map_err(error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(error)?;
        tasks
    };
    protocol.commit().map_err(error)?;
    Ok(json!({"version":"mas-host-observation/1","run_id":run_id,
        "observed_at_ms":now_ms()?,"payloads_included":include_payloads,"cancelled":cancelled,
        "run":{"config":config,"deadline":run.1,"usage":{"messages":run.2,"bytes":run.3,"calls":run.4},
            "active_protocol":run.5,"expired":run.6},
        "messages":messages,"steps":steps,"calls":calls,"tasks":tasks}))
}
