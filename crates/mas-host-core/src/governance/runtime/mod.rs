//! Enforce the bounded, trusted-local structured execution profile.
mod adjudication;
mod authority;
pub use adjudication::adjudicate_governed_root_assessment;
mod boundary;
mod credentials;
mod encoding;
pub(crate) mod exposure;
mod inspection;
pub(crate) mod lifecycle;
mod lineage_page;
mod quarantine;
mod resolver;
mod resolver_record;
mod revision;
mod schema;
mod session;
mod shared_source;
mod source;
mod source_resolver;
pub(crate) mod storage;

pub use inspection::PrivacyInspector;
pub use lifecycle::govern_namespace_lifecycle;
pub use lineage_page::read_governed_source_lineage_page;
pub use quarantine::{
    adjudicate_governed_source_quarantine, govern_source_quarantine,
    initialize_governed_source_quarantine, list_governed_source_quarantine,
    list_governed_source_quarantine_page, read_governed_source_quarantine,
};
pub use resolver::{
    read_governed_effect, read_governed_root_assessment, read_governed_root_effect,
};
pub use session::GovernedSession;
pub use shared_source::{authorize_governed_source_publication, read_governed_shared_source};
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

#[cfg(test)]
mod shared_source_tests;

#[cfg(test)]
mod referenced_lineage_tests;
