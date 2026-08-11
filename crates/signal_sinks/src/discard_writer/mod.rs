//! # `discard_writer`
//!
//! ## Responsibility
//!
//! This module owns the explicit portable sink that consumes and discards a runtime stream.
//!
//! ## Boundaries
//!
//! It is an authored processing behavior, not an implicit platform-specific substitute for an
//! unavailable file writer or source.

//! Browser-safe discard writer processing nodes.
//!
//! These explicit sinks intentionally consume data without persistence. They are a
//! portable authored behavior, not an implicit replacement for unavailable I/O.

mod writers;

pub use writers::{DiscardTextWriter, DiscardWordWriter};
