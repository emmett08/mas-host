//! Read one owned task's root effect through the core's authorised snapshot reader.
use super::{retained, Caller, RpcError, RpcResult, Service};
use serde_json::Value;

pub(super) const PROFILE: &str = "urn:mas:host-governed-effect-read:1";

pub(super) fn unavailable() -> RpcError {
    RpcError(-32001, "effect_not_found")
}

impl Service {
    pub(super) fn retained_effect(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        let (id, key) = retained::request(params, unavailable)?;
        let session = principal.session.as_ref().ok_or_else(unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(principal, id)
            .ok_or_else(unavailable)?;
        let view =
            mas_host_core::governance::read_governed_root_effect(&directory, session, &actor, key)
                .map_err(|_| unavailable())?;
        principal.current().map_err(|_| unavailable())?;
        Ok(view)
    }
}
