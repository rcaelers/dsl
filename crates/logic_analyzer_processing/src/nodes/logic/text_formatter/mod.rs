//! # `text_formatter`
//!
//! ## Responsibility
//!
//! This module owns timestamp-aware formatting of configured input values into a text level stream.
//!
//! ## Boundaries
//!
//! It does not own UI template editing, file destination selection, or output presentation.

//! Text-formatting processing node.
//!
//! It formats generic text stream values. UI templates and graph controls are
//! separate concrete-node concerns.

mod implementation;

pub use implementation::TextFormatter;
