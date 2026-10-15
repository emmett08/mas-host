//! Bounded recursive workflow composition over ordinary MAS requests.
//!
//! The plan module owns syntax and validation. The execution module implements
//! the Composite interpreter against a small Runtime Strategy; the SDK adapter
//! retains the host as the authority for admission, replay and concurrency.
mod execution;
mod plan;

pub use execution::{execute, execute_with, Runtime};
pub use plan::validate;

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = 65_536;
const MAX_NODES: usize = 4096;
const MAX_ITERATIONS: usize = 1024;
const MAX_EVALUATIONS: usize = 100_000;
const MAX_BATCH: usize = 8;

#[cfg(test)]
mod tests;
