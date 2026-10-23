//! Deliver one retained acquisition to an independently authenticated subscriber.
use super::{resolver, source_resolver, GovernedSession};
use serde_json::Value;
use std::path::Path;
type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-shared-source-unavailable";

/// Trusted service-reader seam. The producer session supplies the original
/// namespace seal; it never supplies the subscriber's authority. Both current
/// sessions and the complete original source restrictions must permit delivery.
/// This read neither subscribes an execution worker nor acquires fresh evidence.
pub fn read_governed_shared_source(
    directory: &Path,
    owner: &GovernedSession,
    reader: &GovernedSession,
    actor: &str,
    key: &str,
) -> Result<Value> {
    (|| -> Result<Value> {
        owner.compatible_source_reader(reader)?;
        // Current owner authority is a conservative additional condition, not
        // a substitute for the independent recipient's own grant and policies.
        let view = source_resolver::read_source(directory, owner, actor, key)?;
        let snapshot = resolver::snapshot(directory, owner, actor, None, key)?;
        let binding = snapshot.binding(None, key);
        let exposure = &snapshot.exposure;
        let mut metadata = view.clone();
        metadata["projection"] = Value::Null;
        if view["host_settlement"] == "completed" {
            metadata["payload_disposition"] = serde_json::json!("withheld");
        }
        reader.authorize_resolution_with_exposure(actor, &metadata, &metadata, false, exposure)?;
        let result = if view["payload_disposition"] == "available" {
            reader.authorize_resolution_payload_with_exposure(
                actor,
                &view,
                &view["projection"],
                exposure,
            )?;
            view
        } else {
            metadata
        };
        reader.check_current()?;
        owner.check_current()?;
        binding.verify(directory, owner, actor)?;
        Ok(result)
    })()
    .map_err(|_| UNAVAILABLE.into())
}

/// Admit publication of an exact retained source under the publisher's current
/// grant and every source restriction. Publication conveys a reference; readers
/// still need their own current permission for metadata and projection delivery.
pub fn authorize_governed_source_publication(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    key: &str,
    descriptor: &Value,
) -> Result<()> {
    (|| -> Result<()> {
        let view = source_resolver::read_source(directory, session, actor, key)?;
        if view["host_settlement"] != "completed" || !view["source_id"].is_string() {
            return Err("governed-publication-source-unavailable".into());
        }
        let snapshot = resolver::snapshot(directory, session, actor, None, key)?;
        let binding = snapshot.binding(None, key);
        session.authorize_exposure(
            actor,
            "publish-source",
            &session.0.principal,
            "client",
            descriptor,
            descriptor,
            &snapshot.exposure,
        )?;
        let inspection = session.inspect_disclosure_with_exposure(
            actor,
            descriptor,
            descriptor,
            &snapshot.exposure,
        )?;
        session.authorize_inspected_with_exposure(
            actor,
            "publish-source",
            &session.0.principal,
            "client",
            descriptor,
            descriptor,
            inspection,
            &snapshot.exposure,
        )?;
        session.check_current()?;
        binding.verify(directory, session, actor)?;
        Ok(())
    })()
    .map_err(|_| "governed-source-publication-unavailable".into())
}
