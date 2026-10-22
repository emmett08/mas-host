//! Enforce the bounded, trusted-local structured execution profile.
mod authority;
mod boundary;
mod encoding;
mod inspection;
mod schema;
mod session;
pub(crate) mod storage;

pub use inspection::PrivacyInspector;
pub use session::GovernedSession;
pub(crate) use storage::{open_namespace, require_ordinary};

#[cfg(test)]
mod inspection_adversarial_tests;
#[cfg(test)]
mod tests;
