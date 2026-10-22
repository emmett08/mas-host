//! Reuse the service-owned association before deriving any governed run path.
use super::{fields, Caller, RpcError, RpcResult, Service};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::path::PathBuf;

pub(super) fn request(value: &Value, unavailable: fn() -> RpcError) -> RpcResult<(&str, &str)> {
    fields(value, &["id", "key"], &[])?;
    let id = value["id"]
        .as_str()
        .filter(|id| {
            id.len() == 60
                && id.starts_with("a2a-")
                && id[4..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(unavailable)?;
    let key = value["key"]
        .as_str()
        .filter(|key| !key.is_empty() && key.len() <= 128 && !key.chars().any(char::is_control))
        .ok_or(RpcError(-32602, "invalid_parameters"))?;
    Ok((id, key))
}

impl Service {
    pub(super) fn owned_governed_task(
        &self,
        principal: &Caller,
        id: &str,
    ) -> Option<(PathBuf, String)> {
        let session = principal.session.as_ref()?;
        principal.current().ok()?;
        let (actor, fingerprint): (String, String) = {
            let db = self.db.lock().ok()?;
            db.query_row(
                "SELECT t.agent,g.fingerprint FROM tasks t JOIN governed_tasks g ON g.id=t.id WHERE t.id=? AND t.principal=?",
                params![id, principal.owner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()??
        };
        if !principal.agents.contains(&actor) || fingerprint != session.fingerprint() {
            return None;
        }
        Some((self.state.join("runs").join(id), actor))
    }
}
