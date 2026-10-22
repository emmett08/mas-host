//! Read one owned task's root effect through the core's authorised snapshot reader.
use super::{fields, Caller, RpcError, RpcResult, Service};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

pub(super) const PROFILE: &str = "urn:mas:host-governed-effect-read:1";

pub(super) fn unavailable() -> RpcError {
    RpcError(-32001, "effect_not_found")
}

fn task_id(value: &Value) -> RpcResult<&str> {
    value["id"]
        .as_str()
        .filter(|id| {
            id.len() == 60
                && id.starts_with("a2a-")
                && id[4..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(unavailable)
}

impl Service {
    pub(super) fn retained_effect(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["id", "key"], &[])?;
        let id = task_id(params)?;
        let key = params["key"]
            .as_str()
            .filter(|key| !key.is_empty() && key.len() <= 128 && !key.chars().any(char::is_control))
            .ok_or(RpcError(-32602, "invalid_parameters"))?;
        let session = principal.session.as_ref().ok_or_else(unavailable)?;
        principal.current().map_err(|_| unavailable())?;
        // Resolve task ownership and the exact admitted scope before deriving
        // any run path. The service record owns this association; the client
        // supplies neither a directory, actor nor an invocation identity.
        let (actor, fingerprint): (String, String) = {
            let db = self.db.lock().map_err(|_| unavailable())?;
            db.query_row(
                "SELECT t.agent,g.fingerprint FROM tasks t JOIN governed_tasks g ON g.id=t.id WHERE t.id=? AND t.principal=?",
                params![id, principal.owner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?
        };
        if !principal.agents.contains(&actor) || fingerprint != session.fingerprint() {
            return Err(unavailable());
        }
        let directory = self.state.join("runs").join(id);
        let view =
            mas_host_core::governance::read_governed_root_effect(&directory, session, &actor, key)
                .map_err(|_| unavailable())?;
        principal.current().map_err(|_| unavailable())?;
        Ok(view)
    }
}
