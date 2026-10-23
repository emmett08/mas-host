//! Resolve retained source projections without acquiring execution authority.
use super::{exposure, resolver, resolver_record, GovernedSession};
use serde_json::{json, Value};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-evidence-unavailable";

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(super) fn read_source(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    key: &str,
) -> Result<Value> {
    if !session.is_v2() || !identifier(actor) || !identifier(key) {
        return Err(UNAVAILABLE.into());
    }
    let query = json!({"version":"mas-governed-source-query/1","actor":actor,"key":key});
    session.authorize_resolution(actor, &query, &query, true)?;
    let snapshot = resolver::snapshot(directory, session, actor, None, key)?;
    let binding = snapshot.binding(None, key);
    let mut view = json!({"version":"mas-governed-source-view/1","namespace_ref":snapshot.namespace_ref,
        "actor":actor,"parent_invocation":snapshot.parent,"key":key,"effect_id":null,"source_id":null,
        "state":"not-recorded","host_settlement":"unknown","external_execution":"unknown",
        "payload_disposition":"unavailable","source_metadata":null,"lineage":[],"projection":null,
        "raw_source_disposition":"not-retained"});
    let closure = &snapshot.exposure;
    let selected = &snapshot.source_exposure;
    let mut payload_view = None;
    if let Some(effect) = snapshot.effect {
        view["effect_id"] = json!(effect.id);
        view["state"] = json!(effect.state);
        if matches!(effect.state.as_str(), "completed" | "resolved") {
            let output = effect.output.ok_or(UNAVAILABLE)?;
            let evidence = effect.evidence.ok_or(UNAVAILABLE)?;
            let completion = effect.completion.ok_or(UNAVAILABLE)?;
            resolver_record::validate_completion(&effect.id, &output, &evidence, &completion)?;
            let mut sources = selected
                .sources
                .values()
                .filter(|source| source.step_id() == effect.id && source.kind() == "effect-output");
            let source = sources.next().ok_or(UNAVAILABLE)?;
            if sources.next().is_some() {
                return Err(UNAVAILABLE.into());
            }
            view["source_id"] = json!(source.id());
            view["source_metadata"] = source.metadata();
            if session.referenced_lineage_enabled() {
                view["version"] = json!("mas-governed-source-view/2");
                view["lineage_disposition"] = json!("referenced");
                view["lineage_reference"] = super::lineage_page::reference(session, selected)?;
            } else {
                view["lineage"] = json!(selected
                    .sources
                    .values()
                    .map(|source| source.metadata())
                    .collect::<Vec<_>>());
            }
            view["host_settlement"] = json!("completed");
            view["external_execution"] = completion["external_execution"].clone();
            view["payload_disposition"] = json!("withheld");
            if source.body["payload_available"] == true
                && completion["payload_disposition"] == "available"
            {
                payload_view = Some(json!({"actor":actor,"host_settlement":"completed",
                    "payload_disposition":"available","descriptor":effect.descriptor,
                    "output":output,"evidence":evidence}));
            }
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
    super::encoding::canonical(&view, 65_536)?;
    session.authorize_resolution_with_exposure(actor, &view, &view, false, closure)?;
    if let Some(payload) = payload_view {
        let available = (|| -> Result<Value> {
            resolver_record::validate_payload(session, actor, &payload)?;
            let mut available = view.clone();
            available["payload_disposition"] = json!("available");
            available["projection"] = payload["output"].clone();
            super::encoding::canonical(&available, 65_536)?;
            session.authorize_resolution_payload_with_exposure(
                actor,
                &available,
                &payload["output"],
                closure,
            )?;
            Ok(available)
        })();
        if let Ok(available) = available {
            session.check_current()?;
            binding.verify(directory, session, actor)?;
            return Ok(available);
        }
        session.authorize_resolution_with_exposure(actor, &view, &view, false, closure)?;
    }
    binding.verify(directory, session, actor)?;
    Ok(view)
}

/// Read only an owned namespace's exact root effect source. The projection is
/// retained output; discarded original bytes cannot be reconstructed or fetched.
pub fn read_governed_root_source(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    key: &str,
) -> Result<Value> {
    read_source(directory, session, actor, key).map_err(|_| UNAVAILABLE.into())
}

/// Check an existing task response against the root invocation's full exposure.
/// This helper reads a coherent snapshot and performs no recovery or dispatch.
pub fn read_governed_root_delivery(
    directory: &Path,
    session: &GovernedSession,
    actor: &str,
    payload: Option<&Value>,
    status: &Value,
) -> Result<()> {
    (|| -> Result<()> {
        if !session.is_v2() || !identifier(actor) {
            return Err(UNAVAILABLE.into());
        }
        session.check_current()?;
        session.check_status_value(actor, status)?;
        let (namespace, parent, closure) =
            resolver::with_snapshot(directory, session, actor, None, |db, context| {
                Ok((
                    context.namespace_ref.clone(),
                    context.parent.clone(),
                    exposure::load_invocation(db, session, &context.parent)?,
                ))
            })?;
        if let Some(payload) = payload {
            session.check_delivery_with_exposure(actor, payload, &closure)?;
        }
        // Inspection/authentication callbacks may overlap further acquisition.
        // Recheck the exact namespace and union newly exposed restrictions.
        // Release this second snapshot before any authority callback as well.
        let current = resolver::with_snapshot(directory, session, actor, None, |db, context| {
            if context.namespace_ref != namespace || context.parent != parent {
                return Err(UNAVAILABLE.into());
            }
            exposure::load_invocation(db, session, &context.parent)
        })?;
        let closure = closure.union(&current)?;
        if let Some(payload) = payload {
            // Rebind these same inspected bytes to any newly exposed policy.
            // The status value may be a smaller retention marker.
            session.authorize_status_with_exposure(actor, payload, &closure)?;
        }
        session.authorize_status_with_exposure(actor, status, &closure)?;
        session.check_current()?;
        // No callbacks follow this final identity check. A final authority
        // callback may itself have overlapped another acquisition.
        resolver::with_snapshot(directory, session, actor, None, |db, context| {
            if context.namespace_ref != namespace || context.parent != parent {
                return Err(UNAVAILABLE.into());
            }
            let current = exposure::load_invocation(db, session, &context.parent)?;
            if current.roots != closure.roots
                || current.pending != closure.pending
                || !current.sources.keys().eq(closure.sources.keys())
                || crate::now_ms()? >= current.expiry()
            {
                return Err(UNAVAILABLE.into());
            }
            Ok(())
        })
    })()
    .map_err(|_| UNAVAILABLE.into())
}
