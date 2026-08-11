//! # `output_storage`
//!
//! ## Responsibility
//!
//! This internal module owns the processing-facing contract for opening and writing output files.
//!
//! ## Boundaries
//!
//! It names no native path, browser handle, dialog, or output-selection policy. Platform composition
//! implements the contract and concrete sink nodes consume it.

//! Output-file creation and persistence owned by concrete file sinks.
//!
//! Sinks use this narrow capability rather than selecting paths, dialogs, or host
//! implementations themselves.

mod contract;
#[cfg(test)]
mod testing;

pub(crate) use contract::UnavailableOutputStorage;
pub use contract::{OutputFile, OutputOrigin, OutputStorage};
#[cfg(test)]
pub(crate) use testing::TestOutputStorage;
