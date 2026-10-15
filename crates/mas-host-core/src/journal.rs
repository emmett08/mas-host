//! Durable local checkpoints, kept separate from the protocol database.
use super::{error, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::{path::Path, time::Duration};

pub(crate) fn journal(directory: &Path) -> Result<Connection> {
    let connection = Connection::open(directory.join("execution.sqlite")).map_err(error)?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(error)?;
    connection.execute_batch("PRAGMA synchronous=EXTRA; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS metadata (
          singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL,
          run_id TEXT NOT NULL, bundle TEXT NOT NULL, fingerprint TEXT NOT NULL, nonce TEXT NOT NULL,
          cancelled INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS steps (
          id TEXT PRIMARY KEY, parent TEXT NOT NULL, key TEXT NOT NULL,
          kind TEXT NOT NULL, actor TEXT NOT NULL, target TEXT NOT NULL,
          input TEXT NOT NULL, state TEXT NOT NULL, output TEXT, evidence TEXT,
          UNIQUE(parent,key));").map_err(error)?;
    Ok(connection)
}

#[derive(Debug, Clone)]
pub(crate) struct Step {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) actor: String,
    pub(crate) target: String,
    pub(crate) input: Value,
    pub(crate) state: String,
    pub(crate) output: Option<Value>,
    pub(crate) evidence: Option<Value>,
}
pub(crate) fn read_step(db: &Connection, id: &str) -> Result<Option<Step>> {
    let record = db
        .query_row(
            "SELECT id,kind,actor,target,input,state,output,evidence FROM steps WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()
        .map_err(error)?;
    record
        .map(|r| {
            Ok(Step {
                id: r.0,
                kind: r.1,
                actor: r.2,
                target: r.3,
                input: serde_json::from_str(&r.4).map_err(error)?,
                state: r.5,
                output: r
                    .6
                    .map(|v| serde_json::from_str(&v))
                    .transpose()
                    .map_err(error)?,
                evidence: r
                    .7
                    .map(|v| serde_json::from_str(&v))
                    .transpose()
                    .map_err(error)?,
            })
        })
        .transpose()
}
