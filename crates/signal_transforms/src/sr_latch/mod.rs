//! # `sr_latch`
//!
//! ## Responsibility
//!
//! This module owns set-reset latch behavior for generic level/event control streams.
//!
//! ## Boundaries
//!
//! It does not own graph socket configuration, widget behavior, or generic channel scheduling.

//! Set-reset latch processing node.
//!
//! It owns the UI-independent set/reset stream state machine; controls and graph
//! configuration remain in the graph-node feature.

mod latch;

pub use latch::SrLatch;
