//! Complete bounded DAG closures and sealed invocation manifests in one journal.
use super::{source::Source, GovernedSession};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 64;
type SourceRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
#[derive(Clone, Debug, Default)]
pub(crate) struct Exposure {
    pub(crate) sources: BTreeMap<String, Source>,
    pub(crate) roots: BTreeSet<String>,
    pub(crate) pending: BTreeSet<String>,
}
impl Exposure {
    pub(crate) fn union(&self, other: &Self) -> Result<Self> {
        let mut out = self.clone();
        out.pending.extend(other.pending.iter().cloned());
        for (id, source) in &other.sources {
            if out
                .sources
                .get(id)
                .is_some_and(|old| old.body != source.body)
            {
                return Err("governed-source-conflict".into());
            }
            out.sources.insert(id.clone(), source.clone());
        }
        if out.sources.len() > LIMIT {
            return Err("governed-exposure-limit".into());
        }
        out.frontier();
        out.validate()?;
        Ok(out)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.sources.len() > LIMIT
            || serde_json::to_vec(&self.sources.values().map(|s| &s.body).collect::<Vec<_>>())
                .map_err(|_| "governed-exposure-limit")?
                .len()
                > 65_536
        {
            return Err("governed-exposure-limit".into());
        }
        Ok(())
    }
    fn frontier(&mut self) {
        self.roots = self.sources.keys().cloned().collect();
        for source in self.sources.values() {
            for parent in source.refs("parents") {
                self.roots.remove(&parent);
            }
        }
    }
    pub(crate) fn expiry(&self) -> u64 {
        self.sources
            .values()
            .filter_map(|s| s.body["expires_at_ms"].as_u64())
            .min()
            .unwrap_or(u64::MAX)
    }
    pub(crate) fn with_source(&self, source: Source) -> Result<Self> {
        let mut addition = Self::default();
        addition.sources.insert(source.id().into(), source);
        self.union(&addition)
    }
}
fn bounded(db: &Connection, sql: &str, id: &str) -> Result<Option<String>> {
    let row: Option<Option<String>> = db
        .query_row(sql, [id], |r| r.get(0))
        .optional()
        .map_err(|_| "governed-lineage-unavailable")?;
    match row {
        Some(None) => Err("governed-lineage-limit".into()),
        Some(Some(s)) => Ok(Some(s)),
        None => Ok(None),
    }
}
fn source_record(db: &Connection, session: &GovernedSession, id: &str) -> Result<Source> {
    let body=bounded(db,"SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM governed_sources WHERE id=?",id)?.ok_or("governed-source-unavailable")?;
    let source = Source::parse(
        session,
        mas_protocol_codec::parse_json(&body).map_err(|_| "governed-source-invalid")?,
    )?;
    if source.id() != id {
        return Err("governed-source-binding-mismatch".into());
    }
    let selected: Option<SourceRow> = db.query_row(
        "SELECT CASE WHEN length(CAST(s.kind AS BLOB))<=32 THEN s.kind END,CASE WHEN length(CAST(s.actor AS BLOB))<=128 THEN s.actor END,CASE WHEN length(CAST(s.target AS BLOB))<=128 THEN s.target END,CASE WHEN length(CAST(CASE WHEN m.kind='request-input' THEN s.input ELSE s.output END AS BLOB))<=65536 THEN CASE WHEN m.kind='request-input' THEN s.input ELSE s.output END END FROM governed_source_steps m JOIN steps s ON s.id=m.step_id WHERE m.source_id=? AND m.step_id=?2 AND m.kind=?3",
        params![id,source.step_id(),source.kind()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|"governed-source-unavailable")?;
    let (kind, actor, target, payload) = selected.ok_or("governed-source-unavailable")?;
    let kind = kind.ok_or("governed-source-invalid")?;
    let expected_actor =
        if kind == "request" { target } else { actor }.ok_or("governed-source-invalid")?;
    if source.body["actor"] != expected_actor
        || !matches!(
            (kind.as_str(), source.kind()),
            ("request", "request-input" | "request-output") | ("effect", "effect-output")
        )
    {
        return Err("governed-source-binding-mismatch".into());
    }
    let payload = mas_protocol_codec::parse_json(&payload.ok_or("governed-source-unavailable")?)
        .map_err(|_| "governed-source-invalid")?;
    if source.body["payload_binding"] != session.commitment(&payload)? {
        return Err("governed-source-binding-mismatch".into());
    }
    Ok(source)
}
fn visit(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
    out: &mut Exposure,
    active: &mut BTreeSet<String>,
) -> Result<()> {
    if active.contains(id) {
        return Err("governed-source-cycle".into());
    }
    if out.sources.contains_key(id) {
        return Ok(());
    }
    if out.sources.len() + active.len() >= LIMIT {
        return Err("governed-exposure-limit".into());
    }
    active.insert(id.into());
    let source = source_record(db, session, id)?;
    for parent in source.refs("parents") {
        visit(db, session, &parent, out, active)?;
    }
    active.remove(id);
    out.sources.insert(id.into(), source);
    Ok(())
}
pub(crate) fn load_source(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
) -> Result<Exposure> {
    let mut out = Exposure::default();
    visit(db, session, id, &mut out, &mut BTreeSet::new())?;
    out.frontier();
    out.validate()?;
    Ok(out)
}
pub(crate) fn load_step_source(
    db: &Connection,
    session: &GovernedSession,
    step_id: &str,
    kind: &str,
) -> Result<Exposure> {
    let id: Option<String> = db
        .query_row(
            "SELECT CASE WHEN length(CAST(source_id AS BLOB))<=128 THEN source_id END FROM governed_source_steps WHERE step_id=? AND kind=?",
            params![step_id, kind],
            |r| r.get(0),
        )
        .map_err(|_| "governed-source-unavailable")?;
    let id = id.ok_or("governed-source-invalid")?;
    if id.len() > 128 {
        return Err("governed-source-invalid".into());
    }
    load_source(db, session, &id)
}
fn manifest(session: &GovernedSession, id: &str, kind: &str, exposure: &Exposure) -> Result<Value> {
    let mut body = json!({"version":"mas-governed-exposure/1","invocation_id":id,"kind":kind,"roots":exposure.roots,"source_count":exposure.sources.len(),"pending":exposure.pending,"closure_binding":session.commitment(&json!(exposure.sources.keys().collect::<Vec<_>>()))?});
    body["seal"] = json!(session.commitment(&body)?);
    Ok(body)
}
fn load_manifest(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
    table: &str,
    kind: &str,
) -> Result<Exposure> {
    let sql = format!(
        "SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM {table} WHERE {}=?",
        if table == "governed_contexts" {
            "step_id"
        } else {
            "invocation_id"
        }
    );
    let raw = bounded(db, &sql, id)?.ok_or("governed-exposure-unavailable")?;
    let body = mas_protocol_codec::parse_json(&raw).map_err(|_| "governed-exposure-invalid")?;
    crate::fields(
        &body,
        &[
            "version",
            "invocation_id",
            "kind",
            "roots",
            "source_count",
            "pending",
            "closure_binding",
            "seal",
        ],
        &[],
    )?;
    if body["version"] != "mas-governed-exposure/1"
        || body["invocation_id"] != id
        || body["kind"] != kind
    {
        return Err("governed-exposure-invalid".into());
    }
    let roots = body["roots"]
        .as_array()
        .filter(|r| r.len() <= LIMIT)
        .ok_or("governed-exposure-limit")?;
    let mut out = Exposure::default();
    for root in roots {
        visit(
            db,
            session,
            root.as_str().ok_or("governed-exposure-invalid")?,
            &mut out,
            &mut BTreeSet::new(),
        )?;
    }
    if kind == "invocation" {
        let pending = body["pending"]
            .as_array()
            .filter(|r| r.len() <= 64)
            .ok_or("governed-exposure-limit")?;
        for effect in pending {
            let effect = effect.as_str().ok_or("governed-exposure-invalid")?;
            let record =
                load_pending(db, session, effect)?.ok_or("governed-pending-unavailable")?;
            if record["invocation_id"] != id || !out.pending.insert(effect.into()) {
                return Err("governed-pending-invalid".into());
            }
        }
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM governed_pending WHERE invocation_id=?",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "governed-lineage-unavailable")?;
        if count != out.pending.len() as i64 {
            return Err("governed-pending-binding-mismatch".into());
        }
    }
    out.frontier();
    out.validate()?;
    if manifest(session, id, kind, &out)? != body {
        return Err("governed-exposure-binding-mismatch".into());
    }
    Ok(out)
}
pub(crate) fn load_invocation(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
) -> Result<Exposure> {
    let out = load_invocation_settling(db, session, id)?;
    if !out.pending.is_empty() {
        return Err("governed-acquisition-pending".into());
    }
    Ok(out)
}
pub(crate) fn load_invocation_settling(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
) -> Result<Exposure> {
    load_manifest(db, session, id, "governed_exposures", "invocation")
}
pub(crate) fn load_context(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
) -> Result<Exposure> {
    load_manifest(db, session, id, "governed_contexts", "context")
}
pub(crate) fn save_context(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
    exposure: &Exposure,
) -> Result<()> {
    let body = manifest(session, id, "context", exposure)?.to_string();
    let prior=bounded(db,"SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM governed_contexts WHERE step_id=?",id)?;
    if let Some(prior) = prior {
        if prior != body {
            return Err("governed-context-conflict".into());
        }
    } else {
        db.execute(
            "INSERT INTO governed_contexts VALUES(?,?)",
            params![id, body],
        )
        .map_err(|_| "governed-lineage-unavailable")?;
    }
    Ok(())
}
pub(crate) fn save_invocation(
    db: &Connection,
    session: &GovernedSession,
    id: &str,
    exposure: &Exposure,
) -> Result<()> {
    let body = manifest(session, id, "invocation", exposure)?.to_string();
    db.execute("INSERT INTO governed_exposures VALUES(?,?) ON CONFLICT(invocation_id) DO UPDATE SET body=excluded.body",params![id,body]).map_err(|_|"governed-lineage-unavailable")?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_source(
    db: &Connection,
    session: &GovernedSession,
    step_id: &str,
    kind: &str,
    actor: &str,
    payload: &Value,
    parents: &Exposure,
    additional: &BTreeSet<String>,
    available: bool,
    admitted_expiry: u64,
) -> Result<Exposure> {
    let prior: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM governed_source_steps WHERE step_id=? AND kind=?)",
            params![step_id, kind],
            |r| r.get(0),
        )
        .map_err(|_| "governed-lineage-unavailable")?;
    if prior {
        let retained = load_step_source(db, session, step_id, kind)?;
        let source = retained
            .sources
            .values()
            .find(|s| s.step_id() == step_id && s.kind() == kind)
            .ok_or("governed-source-unavailable")?;
        if source.body["payload_binding"] != session.commitment(payload)? {
            return Err("governed-source-conflict".into());
        }
        return Ok(retained);
    }
    let refs = session.baseline_refs().union(additional).cloned().collect();
    let stamp = if kind == "effect-output" {
        Some(load_pending(db, session, step_id)?.ok_or("governed-pending-unavailable")?)
    } else {
        None
    };
    let source = if let Some(stamp) = stamp {
        if stamp["policy_refs"] != json!(refs) {
            return Err("governed-source-policy-conflict".into());
        }
        Source::create_at(
            session,
            step_id,
            kind,
            actor,
            payload,
            &parents.roots,
            &refs,
            parents.expiry(),
            available,
            stamp["acquired_at_ms"].as_u64().unwrap(),
            stamp["expires_at_ms"].as_u64().unwrap(),
        )?
    } else {
        Source::create_at(
            session,
            step_id,
            kind,
            actor,
            payload,
            &parents.roots,
            &refs,
            parents.expiry(),
            available,
            crate::now_ms()?,
            admitted_expiry,
        )?
    };
    let out = parents.with_source(source.clone())?;
    db.execute(
        "INSERT INTO governed_sources VALUES(?,?)",
        params![source.id(), source.body.to_string()],
    )
    .map_err(|_| "governed-lineage-unavailable")?;
    db.execute(
        "INSERT INTO governed_source_steps VALUES(?,?,?)",
        params![step_id, kind, source.id()],
    )
    .map_err(|_| "governed-lineage-unavailable")?;
    Ok(out)
}
pub(crate) fn expose(
    db: &Connection,
    session: &GovernedSession,
    invocation: &str,
    addition: &Exposure,
) -> Result<Exposure> {
    let current = load_invocation_settling(db, session, invocation)?;
    let joined = current.union(addition)?;
    save_invocation(db, session, invocation, &joined)?;
    Ok(joined)
}

/// Freeze acquisition authority before the adapter receives execute permission.
pub(crate) fn begin_pending(
    db: &Connection,
    session: &GovernedSession,
    invocation: &str,
    effect: &str,
    refs: &BTreeSet<String>,
    context: &Exposure,
    admitted_expiry: u64,
) -> Result<()> {
    if load_pending(db, session, effect)?.is_some() {
        return Err("governed-acquisition-already-pending".into());
    }
    let mut exposure = load_invocation(db, session, invocation)?;
    // Authority was checked before the write lock. Do not grant execute if
    // another host-owned exposure changed that exact admitted context meanwhile.
    if exposure.sources.keys().ne(context.sources.keys()) || exposure.roots != context.roots {
        return Err("governed-context-changed".into());
    }
    let refs: BTreeSet<_> = session.baseline_refs().union(refs).cloned().collect();
    let mut body = json!({"version":"mas-governed-pending-source/1","invocation_id":invocation,"effect_id":effect,"policy_refs":refs,"context_binding":session.commitment(&json!(context.sources.keys().collect::<Vec<_>>()))?,"acquired_at_ms":crate::now_ms()?,"expires_at_ms":admitted_expiry.min(context.expiry())});
    body["seal"] = json!(session.commitment(&body)?);
    db.execute(
        "INSERT INTO governed_pending VALUES(?,?,?)",
        params![invocation, effect, body.to_string()],
    )
    .map_err(|_| "governed-lineage-unavailable")?;
    exposure.pending.insert(effect.into());
    save_invocation(db, session, invocation, &exposure)?;
    Ok(())
}
pub(crate) fn load_pending(
    db: &Connection,
    session: &GovernedSession,
    effect: &str,
) -> Result<Option<Value>> {
    let Some(raw)=bounded(db,"SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM governed_pending WHERE effect_id=?",effect)? else {return Ok(None)};
    let body = mas_protocol_codec::parse_json(&raw).map_err(|_| "governed-pending-invalid")?;
    crate::fields(
        &body,
        &[
            "version",
            "invocation_id",
            "effect_id",
            "policy_refs",
            "context_binding",
            "acquired_at_ms",
            "expires_at_ms",
            "seal",
        ],
        &[],
    )?;
    let mut unsigned = body.clone();
    unsigned.as_object_mut().unwrap().remove("seal");
    if body["version"] != "mas-governed-pending-source/1"
        || body["effect_id"] != effect
        || body["seal"] != session.commitment(&unsigned)?
        || body["acquired_at_ms"].as_u64().is_none()
        || body["expires_at_ms"].as_u64().is_none()
    {
        return Err("governed-pending-invalid".into());
    }
    Ok(Some(body))
}
pub(crate) fn finish_pending(
    db: &Connection,
    session: &GovernedSession,
    invocation: &str,
    effect: &str,
) -> Result<()> {
    let mut exposure = load_invocation_settling(db, session, invocation)?;
    db.execute("DELETE FROM governed_pending WHERE effect_id=?", [effect])
        .map_err(|_| "governed-lineage-unavailable")?;
    exposure.pending.remove(effect);
    save_invocation(db, session, invocation, &exposure)
}
