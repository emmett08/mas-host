//! Enforce the bounded, trusted-local structured execution profile.
mod authority;
mod boundary;
mod encoding;
pub(crate) mod exposure;
mod inspection;
mod resolver;
mod resolver_record;
mod schema;
mod session;
mod source;
mod source_resolver;
pub(crate) mod storage;

pub use inspection::PrivacyInspector;
pub use resolver::{read_governed_effect, read_governed_root_effect};
pub use session::GovernedSession;
pub use source_resolver::{read_governed_root_delivery, read_governed_root_source};
pub(crate) use storage::{open_namespace, require_ordinary};

#[cfg(test)]
mod exposure_adversarial_tests;
#[cfg(test)]
mod inspection_adversarial_tests;
#[cfg(test)]
mod resolver_adversarial_tests;
#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod source_resolver_tests;
#[cfg(test)]
mod tests;
