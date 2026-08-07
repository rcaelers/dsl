//! # `event_gate`
//!
//! ## Responsibility
//!
//! This module owns level-controlled gating of an event stream.
//!
//! ## Boundaries
//!
//! It does not own level/event port contracts, viewer presentation, or graph editing; it consumes the
//! generic runtime contracts supplied by `signal_capture_session`.

//! Signal-controlled timestamped-event gate.
//!
//! It gates generic timestamped events according to its input stream and configuration;
//! it does not own graph topology, controls, or viewer behavior.

mod implementation;

pub use implementation::{EventGate, GatePolarity};
