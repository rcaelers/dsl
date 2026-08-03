//! # `trigger_counter`
//!
//! ## Responsibility
//!
//! This module owns counting trigger events into a generic numeric level stream.
//!
//! ## Boundaries
//!
//! It does not own trigger acquisition policy, graph controls, or numeric presentation.

//! Trigger-counting processing node.
//!
//! It applies protocol-neutral counting semantics to trigger streams; capture
//! policy, UI editing, and graph composition are outside this module.

mod implementation;

pub use implementation::TriggerCounter;
