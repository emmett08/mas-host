//! Resolve one retained effect through a bounded read-only journal snapshot.
use super::{exposure, resolver_record, GovernedSession};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-evidence-unavailable";
type JournalRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(super) struct Retained {
    pub(super) id: String,
    pub(super) state: String,
    pub(super) descriptor: Value,
    pub(super) output: Option<Value>,
    pub(super) evidence: Option<Value>,
    pub(super) completion: Option<Value>,
}

pub(super) struct Snapshot {
    pub(super) namespace_ref: String,
    pub(super) parent: String,
    pub(super) effect: Option<Retained>,
    pub(super) exposure: exposure::Exposure,
    pub(super) source_exposure: exposure::Exposure,
}

pub(super) struct ReadBinding {
    namespace: String,
    parent: String,
    selector: Option<String>,
    key: String,
    admission: exposure::Exposure,
    source: exposure::Exposure,
}

fn same_exposure(left: &exposure::Exposure, right: &exposure::Exposure) -> bool {
    left.roots == right.roots
        && left.pending == right.pending
        && left.sources.keys().eq(right.sources.keys())
}

impl Snapshot {
    pub(super) fn binding(&self, parent: Option<&str>, key: &str) -> ReadBinding {
        ReadBinding {
            namespace: self.namespace_ref.clone(),
            parent: self.parent.clone(),
            selector: parent.map(str::to_owned),
            key: key.into(),
            admission: self.exposure.clone(),
            source: self.source_exposure.clone(),
        }
    }
}

impl ReadBinding {
    /// No callbacks follow this pure re-read. New/pending exposure stops the
    /// read instead of silently delivering under an earlier task context.
    pub(super) fn verify(
        &self,
        directory: &Path,
        session: &GovernedSession,
        actor: &str,
    ) -> Result<()> {
        if !session.is_v2() {
            return Ok(());
        }
        let current = snapshot(
            directory,
            session,
            actor,
            self.selector.as_deref(),
            &self.key,
        )?;
        if self.namespace != current.namespace_ref
            || self.parent != current.parent
            || !same_exposure(&self.admission, &current.exposure)
            || !same_exposure(&self.source, &current.source_exposure)
            || crate::now_ms()? >= current.exposure.expiry()
        {
            return Err(UNAVAILABLE.into());
        }
        Ok(())
    }
}

pub(super) struct Context {
    pub(super) namespace_ref: String,
    pub(super) parent: String,
    run_id: String,
    nonce: String,
}

fn parsed(value: Option<String>) -> Result<Option<Value>> {
    value
        .map(|text| mas_protocol_codec::parse_json(&text).map_err(|_| UNAVAILABLE.into()))
        .transpose()
}

fn bounded_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn task_id(value: &str) -> bool {
    value.strip_prefix("task-").is_some_and(|tail| {
        tail.len() == 64
            && tail
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub(super) fn with_snapshot<T>(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    parent: Option<&str>,
    read: impl FnOnce(&Transaction<'_>, &Context) -> Result<T>,
) -> Result<T> {
    let metadata = std::fs::symlink_metadata(directory).map_err(|_| UNAVAILABLE)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(UNAVAILABLE.into());
    }
    let path = directory.join("execution.sqlite");
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(UNAVAILABLE.into());
    }
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| UNAVAILABLE)?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|_| UNAVAILABLE)?;
    let tx = db.transaction().map_err(|_| UNAVAILABLE)?;
    let fingerprint: Option<String> = tx.query_row(
        "SELECT CASE WHEN length(CAST(fingerprint AS BLOB))<=128 THEN fingerprint END FROM governed_run WHERE singleton=1",
        [], |r| r.get(0)).map_err(|_| UNAVAILABLE)?;
    if fingerprint.as_deref() != Some(session.fingerprint().as_str()) {
        return Err(UNAVAILABLE.into());
    }
    let (version, run_id, nonce): (i64,Option<String>,Option<String>) = tx.query_row(
        "SELECT version,CASE WHEN length(CAST(run_id AS BLOB))<=96 THEN run_id END,CASE WHEN length(CAST(nonce AS BLOB))<=128 THEN nonce END FROM metadata WHERE singleton=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_| UNAVAILABLE)?;
    let run_id = run_id.ok_or(UNAVAILABLE)?;
    let nonce = nonce.ok_or(UNAVAILABLE)?;
    if version != 1
        || directory.file_name().and_then(|name| name.to_str()) != Some(run_id.as_str())
        || crate::run_directory(Path::new("/"), &run_id).is_err()
        || nonce.len() != 64
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(UNAVAILABLE.into());
    }
    let namespace_ref = session.identity_commitment(&json!([
        "mas-governed-evidence-namespace/1",
        run_id,
        nonce,
        fingerprint
    ]))?;
    let parent: Option<String> = match parent {
        Some(parent) => tx.query_row(
            "SELECT CASE WHEN length(CAST(id AS BLOB))<=128 THEN id END FROM steps WHERE id=? AND kind='request' AND target=?",
            params![parent,actor], |r| r.get(0)).optional().map_err(|_| UNAVAILABLE)?.flatten(),
        None => tx.query_row(
            "SELECT CASE WHEN length(CAST(id AS BLOB))<=128 THEN id END FROM steps WHERE parent='' AND key=? AND kind='request' AND actor='client' AND target=?",
            params![session.identity_commitment(&json!(["","root"]))?,actor], |r| r.get(0)).optional().map_err(|_| UNAVAILABLE)?.flatten(),
    };
    let parent = parent.filter(|id| task_id(id)).ok_or(UNAVAILABLE)?;
    let context = Context {
        namespace_ref,
        parent,
        run_id,
        nonce,
    };
    let result = read(&tx, &context)?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    drop(db);
    Ok(result)
}

pub(super) fn snapshot(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    parent: Option<&str>,
    key: &str,
) -> Result<Snapshot> {
    with_snapshot(directory, session, actor, parent, |tx, context| {
        let parent = &context.parent;
        let run_id = &context.run_id;
        let nonce = &context.nonce;
        let storage_key = session.identity_commitment(&json!([parent, key]))?;
        let row: Option<JournalRow> = tx.query_row(
        "SELECT CASE WHEN length(CAST(id AS BLOB))<=128 THEN id END,CASE WHEN length(CAST(state AS BLOB))<=32 THEN state END,CASE WHEN length(CAST(input AS BLOB))<=65536 THEN input END,CASE WHEN length(CAST(output AS BLOB))<=65536 THEN output END,CASE WHEN length(CAST(evidence AS BLOB))<=65536 THEN evidence END FROM steps WHERE parent=? AND key=? AND actor=? AND kind='effect' AND target=''",
        params![parent,storage_key,actor], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_| UNAVAILABLE)?;
        let effect = if let Some((id, state, descriptor, output, evidence)) = row {
            let id = id.ok_or(UNAVAILABLE)?;
            let expected_id = format!(
                "eff-{}",
                crate::digest(json!([nonce, run_id, parent, key]).to_string().as_bytes())
            );
            if id != expected_id {
                return Err(UNAVAILABLE.into());
            }
            let descriptor = parsed(descriptor)?.ok_or(UNAVAILABLE)?;
            let original: Option<String> = tx.query_row(
            "SELECT CASE WHEN length(CAST(original_commitment AS BLOB))<=128 THEN original_commitment END FROM governed_inputs WHERE id=?",
            [&id], |r| r.get(0)).optional().map_err(|_| UNAVAILABLE)?.flatten();
            if original.as_deref() != Some(session.commitment(&descriptor)?.as_str()) {
                return Err(UNAVAILABLE.into());
            }
            let state = state.ok_or(UNAVAILABLE)?;
            if ![
                "prepared",
                "executing",
                "unknown",
                "completed",
                "resolved",
                "blocked",
                "failed",
                "rejected",
            ]
            .contains(&state.as_str())
            {
                return Err(UNAVAILABLE.into());
            }
            let body: Option<Option<String>> = tx.query_row(
            "SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM governed_completions WHERE id=?",
            [&id], |r| r.get(0)).optional().map_err(|_| UNAVAILABLE)?;
            if body == Some(None) {
                return Err(UNAVAILABLE.into());
            }
            Some(Retained {
                id,
                state,
                descriptor,
                output: parsed(output)?,
                evidence: parsed(evidence)?,
                completion: parsed(body.flatten())?,
            })
        } else {
            None
        };
        let source_exposure = if session.is_v2() {
            if let Some(effect) = &effect {
                if matches!(effect.state.as_str(), "completed" | "resolved") {
                    exposure::load_step_source(tx, session, &effect.id, "effect-output")?
                } else {
                    exposure::Exposure::default()
                }
            } else {
                exposure::Exposure::default()
            }
        } else {
            exposure::Exposure::default()
        };
        let exposure = if session.is_v2() {
            exposure::load_invocation(tx, session, parent)?.union(&source_exposure)?
        } else {
            exposure::Exposure::default()
        };
        Ok(Snapshot {
            namespace_ref: context.namespace_ref.clone(),
            parent: parent.clone(),
            effect,
            exposure,
            source_exposure,
        })
    })
}

fn resolve(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    parent: Option<&str>,
    key: &str,
) -> Result<Value> {
    if !bounded_identifier(actor)
        || !bounded_identifier(key)
        || parent.is_some_and(|id| !task_id(id))
    {
        return Err(UNAVAILABLE.into());
    }
    let query = json!({"version":"mas-governed-effect-query/1","actor":actor,"parent_invocation":parent,"key":key});
    // Authorise both query processing and the reader operation before opening
    // the journal. Namespace identity precedes any effect-specific lookup.
    session.authorize_resolution(actor, &query, &query, true)?;
    let snapshot = snapshot(directory, session, actor, parent, key)?;
    let binding = snapshot.binding(parent, key);
    let exposure = &snapshot.exposure;
    let mut view = json!({"version":"mas-governed-effect-view/1","namespace_ref":snapshot.namespace_ref,
        "actor":actor,"parent_invocation":snapshot.parent,"key":key,"effect_id":null,"state":"not-recorded",
        "host_settlement":"unknown","external_execution":"unknown","payload_disposition":"unavailable",
        "descriptor":null,"output":null,"evidence":null});
    if let Some(effect) = snapshot.effect {
        view["effect_id"] = json!(effect.id);
        view["state"] = json!(effect.state);
        view["descriptor"] = effect.descriptor;
        if matches!(effect.state.as_str(), "completed" | "resolved") {
            let output = effect.output.ok_or(UNAVAILABLE)?;
            let evidence = effect.evidence.ok_or(UNAVAILABLE)?;
            let completion = effect.completion.ok_or(UNAVAILABLE)?;
            resolver_record::validate_completion(&effect.id, &output, &evidence, &completion)?;
            view["host_settlement"] = json!("completed");
            view["external_execution"] = completion["external_execution"].clone();
            view["payload_disposition"] = completion["payload_disposition"].clone();
            view["output"] = output;
            view["evidence"] = evidence;
        } else {
            if effect.completion.is_some() {
                return Err(UNAVAILABLE.into());
            }
            view["host_settlement"] = json!(match effect.state.as_str() {
                "prepared" | "executing" => "pending",
                "failed" | "rejected" => "failed",
                _ => "unknown",
            });
        }
    }
    let mut metadata = view.clone();
    for field in ["descriptor", "output", "evidence"] {
        metadata[field] = Value::Null;
    }
    if metadata["host_settlement"] == "completed" {
        metadata["payload_disposition"] = json!("withheld");
    }
    super::encoding::canonical(&metadata, 65_536)?;
    // Safe host facts remain separately governed when a retained payload no
    // longer satisfies its schema, inspection or disclosure policy. No read
    // changes either the stored completion or execution uncertainty.
    session.authorize_resolution_with_exposure(actor, &metadata, &metadata, false, exposure)?;
    if view["host_settlement"] != "completed" || view["payload_disposition"] != "available" {
        binding.verify(directory, session, actor)?;
        return Ok(metadata);
    }
    let available = (|| -> Result<()> {
        resolver_record::validate_payload(session, actor, &view)?;
        super::encoding::canonical(&view, 65_536)?;
        // Inspect the application descriptor, result and reported evidence.
        // Host-owned receipt metadata was validated separately; scanning its
        // nested contracts again would multiply the native text-unit budget.
        let payload = json!({"descriptor":view["descriptor"],"output":view["output"],
            "evidence":view["evidence"]["reported_evidence"]});
        session.authorize_resolution_payload_with_exposure(actor, &view, &payload, exposure)
    })();
    if available.is_ok() {
        session.check_current()?;
        binding.verify(directory, session, actor)?;
        Ok(view)
    } else {
        session.authorize_resolution_with_exposure(actor, &metadata, &metadata, false, exposure)?;
        binding.verify(directory, session, actor)?;
        Ok(metadata)
    }
}

/// Read one exact retained effect without taking the execution owner lock,
/// recovery, migration, protocol access or writes. All failures are content-free.
pub fn read_governed_effect(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    parent_invocation: &str,
    key: &str,
) -> Result<Value> {
    resolve(directory, session, actor, Some(parent_invocation), key).map_err(|_| UNAVAILABLE.into())
}

/// Resolve only the effect of this namespace's exact root invocation.
/// An authorised missing effect is "not-recorded", with unknown external
/// execution. It certifies neither non-execution nor permission to retry.
pub fn read_governed_root_effect(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    key: &str,
) -> Result<Value> {
    resolve(directory, session, actor, None, key).map_err(|_| UNAVAILABLE.into())
}
