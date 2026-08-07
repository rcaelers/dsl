//! # `event_control`
//!
//! ## Responsibility
//!
//! This module owns event delay, holdoff, and rearm processing for event streams.
//!
//! ## Boundaries
//!
//! It does not define event socket presentation, document state migration, or runtime scheduling.

//! Event delay, holdoff, and explicit rearm control.
//!
//! The runtime behavior is protocol-neutral; product presentation and graph
//! lifecycle policy are owned by their respective generic and UI components.

mod implementation;

pub use implementation::EventControl;
