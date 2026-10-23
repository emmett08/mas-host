//! Append an exact-source human review to the existing execution journal.
use super::{exposure, resolver, GovernedSession};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-assessment-review-unavailable";

/// Record a separately authorised verdict without modifying inspection truth,
/// disclosure decisions, existing completion or execution permission. The stable
/// review key provides changed-verdict conflict detection and replay identity.
pub fn adjudicate_governed_root_assessment(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    key: &str,
    review: &Value,
) -> Result<Value> {
    (|| -> Result<Value> {
        crate::fields(review, &["review_key", "assessment_binding", "verdict", "reason"], &[])?;
        let review_key = review["review_key"].as_str().filter(|s| !s.is_empty() && s.len() <= 64
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))).ok_or(UNAVAILABLE)?;
        if !["false-positive", "confirmed-finding", "inconclusive"].contains(&review["verdict"].as_str().ok_or(UNAVAILABLE)?)
            || !["authorised-source-review", "domain-context", "insufficient-evidence"].contains(&review["reason"].as_str().ok_or(UNAVAILABLE)?)
            || (review["verdict"] == "inconclusive" && review["reason"] != "insufficient-evidence") {
            return Err(UNAVAILABLE.into());
        }
        session.authorize_assessment_adjudication(actor, review, &exposure::Exposure::default())?;
        let view = resolver::read_governed_root_assessment(directory, session, actor, key)?;
        let components = view["inspection"]["component_assessments"].as_array().ok_or(UNAVAILABLE)?;
        let component = components.iter().find(|item| item["binding"] == review["assessment_binding"]).ok_or(UNAVAILABLE)?;
        let snapshot = resolver::snapshot(directory, session, actor, None, key)?;
        let binding = snapshot.binding(None, key);
        let body = json!({"version":"mas-governed-assessment-review/1", "namespace_ref":view["namespace_ref"],
            "parent_invocation":view["parent_invocation"],"effect_id":view["effect_id"],"key":key,"actor":actor,
            "reviewer_ref":session.commitment(&json!(["reviewer/1",session.fingerprint()]))?,
            "review_key":review_key,"assessment_binding":component["binding"],"source_binding":component["record"]["source_binding"],
            "verdict":review["verdict"],"reason":review["reason"],"release_permission":"requires-fresh-policy-decision"});
        let id = session.commitment(&json!(["assessment-review-id/1",view["namespace_ref"],view["parent_invocation"],view["effect_id"],review_key]))?;
        let record = json!({"review_id":id,"binding":session.commitment(&body)?,"record":body});
        super::encoding::canonical(&record, 65_536)?;
        session.authorize_assessment_adjudication(actor, &record, &snapshot.exposure)?;
        session.authorize_assessment_resolution(actor, &record, &snapshot.exposure)?;
        session.check_current()?;
        binding.verify(directory, session, actor)?;
        super::lifecycle::with_admission(directory, session, || -> Result<Value> {
        let mut db = Connection::open_with_flags(directory.join("execution.sqlite"), OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|_|UNAVAILABLE)?;
        db.busy_timeout(Duration::from_secs(2)).map_err(|_|UNAVAILABLE)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|UNAVAILABLE)?;
        // Existing execution ownership and tables remain unchanged. This table
        // stores bounded immutable review metadata, never a new dispatch queue.
        tx.execute_batch("CREATE TABLE IF NOT EXISTS governed_assessment_reviews(id TEXT PRIMARY KEY,body TEXT NOT NULL)").map_err(|_|UNAVAILABLE)?;
        let prior: Option<String> = tx.query_row("SELECT body FROM governed_assessment_reviews WHERE id=?",[&id],|r|r.get(0)).optional().map_err(|_|UNAVAILABLE)?;
        if let Some(prior) = prior {
            if mas_protocol_codec::parse_json(&prior).map_err(|_|UNAVAILABLE)? != record { return Err(UNAVAILABLE.into()); }
        } else {
            let count:i64=tx.query_row("SELECT COUNT(*) FROM governed_assessment_reviews",[],|r|r.get(0)).map_err(|_|UNAVAILABLE)?;
            if count>=128 {return Err(UNAVAILABLE.into());}
            tx.execute("INSERT INTO governed_assessment_reviews VALUES(?,?)",params![id,record.to_string()]).map_err(|_|UNAVAILABLE)?;
        }
        // Revalidate current authority and complete exposure before commit.
        session.check_current()?;
        if session.is_v2() {
            let current = exposure::load_invocation(&tx, session, &snapshot.parent)?;
            if !current.pending.is_empty() || current.roots != snapshot.exposure.roots {
                return Err(UNAVAILABLE.into());
            }
        }
        tx.commit().map_err(|_|UNAVAILABLE)?;
        Ok(record)
        })
    })().map_err(|_|UNAVAILABLE.into())
}
