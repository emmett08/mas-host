//! Privacy contract fixtures for bounded UTF-8 input, separate from execution authority.
//!
//! This crate provides N0 conformance and a deterministic synthetic scanner. A
//! protected profile always withholds until a qualified neural adapter and the
//! host's complete persistence/disclosure enforcement path are implemented.

mod assessment;
mod combined;
mod handling;
mod neural;
mod neural_pins;
mod record;
mod scanner;
mod source;
mod spans;

pub use assessment::{
    Assessment, ByteRange, Category, Completion, DetectorIdentity, Finding, Limits,
};
pub use combined::CombinedAssessment;
pub use handling::{
    decide, Decision, Disposition, ExecutionCertainty, HandlingContext, HandlingProfile,
    NotificationRoute, Reason,
};
pub use neural::{NeuralAssessment, NeuralFinding, OpenAiLabel};
pub use neural_pins::DetectorPins;
pub use scanner::scan_synthetic;
pub use source::Source;
pub use spans::merge_findings;

/// Stable, content-free failures suitable for a caller's own governed diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidIdentifier,
    InputTooLarge,
    UnsupportedFormat,
    InvalidRecord,
    UnsupportedVersion,
    DetectorMismatch,
    SourceMismatch,
    InvalidRange,
    IncompleteCoverage,
    LimitExceeded,
    EntropyUnavailable,
}

pub type Result<T> = std::result::Result<T, Error>;
