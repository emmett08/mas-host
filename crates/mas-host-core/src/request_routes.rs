//! Host task execution always needs its correlated Result or Failure route.
use super::{Inner, Result};

impl Inner {
    pub(crate) fn check_reply_route(&self, actor: &str, target: &str) -> Result<()> {
        // Forward admission remains the protocol's decision. A permitted
        // request with an impossible reply is rejected before host preparation
        // or worker execution, rather than after an irreversible effect.
        if self
            .request_routes
            .contains(&(actor.to_owned(), target.to_owned()))
            && !self
                .request_routes
                .contains(&(target.to_owned(), actor.to_owned()))
        {
            return Err("task reply route forbidden by frozen topology".into());
        }
        Ok(())
    }
}
