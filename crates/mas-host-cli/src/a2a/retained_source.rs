//! Resolve only an authenticated task's exact retained acquisition projection.
use super::{retained, Caller, RpcError, RpcResult, Service};
use serde_json::Value;

pub(super) const PROFILE: &str = "urn:mas:host-governed-source-read:1";

pub(super) fn unavailable() -> RpcError {
    RpcError(-32001, "source_not_found")
}

impl Service {
    pub(super) fn retained_source(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        let (id, key) = retained::request(params, unavailable)?;
        let session = principal.session.as_ref().ok_or_else(unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(principal, id)
            .ok_or_else(unavailable)?;
        let view =
            mas_host_core::governance::read_governed_root_source(&directory, session, &actor, key)
                .map_err(|_| unavailable())?;
        // The core reader checks this caller's exact authentication lease
        // before its final pure exposure check. Do not add a callback after it.
        Ok(view)
    }
}
