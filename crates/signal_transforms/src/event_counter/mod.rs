//! # `event_counter`
//!
//! ## Responsibility
//!
//! This module owns counting timestamped events into a generic numeric level stream.
//!
//! ## Boundaries
//!
//! It does not own event-source policy, graph controls, or numeric presentation.

//! Event-counting processing node.
//!
//! It applies protocol-neutral counting semantics to event streams; capture
//! policy, UI editing, and graph composition are outside this module.

mod counter;

pub use counter::EventCounter;
