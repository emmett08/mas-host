//! Read exact component assessment metadata through the owned-task boundary.
use super::{retained, Caller, RpcError, RpcResult, Service};
use serde_json::Value;

fn unavailable() -> RpcError {
    RpcError(-32001, "assessment_not_found")
}

pub(super) const PROFILE: &str = "urn:mas:host-governed-assessment-review:1";

impl Service {
    pub(super) fn retained_assessment(
        &self,
        principal: &Caller,
        params: &Value,
    ) -> RpcResult<Value> {
        let (id, key) = retained::request(params, unavailable)?;
        let session = principal.session.as_ref().ok_or_else(unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(principal, id)
            .ok_or_else(unavailable)?;
        mas_host_core::governance::read_governed_root_assessment(&directory, session, &actor, key)
            .map_err(|_| unavailable())
    }
}

impl Service {
    pub(super) fn adjudicate_assessment(
        &self,
        principal: &Caller,
        params: &Value,
    ) -> RpcResult<Value> {
        super::fields(params, &["id", "key", "review"], &[]).map_err(|_| unavailable())?;
        let selector = serde_json::json!({"id":params["id"],"key":params["key"]});
        let (id, key) = retained::request(&selector, unavailable)?;
        let session = principal.session.as_ref().ok_or_else(unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(principal, id)
            .ok_or_else(unavailable)?;
        mas_host_core::governance::adjudicate_governed_root_assessment(
            &directory,
            session,
            &actor,
            key,
            &params["review"],
        )
        .map_err(|_| unavailable())
    }
}
