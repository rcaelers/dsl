//! # `edge_detector`
//!
//! ## Responsibility
//!
//! This module owns edge detection, debounce, and pulse-qualification processing over sampled levels.
//!
//! ## Boundaries
//!
//! It does not own raw-capture indexing, viewer markers, graph controls, or trigger panel policy.

//! Signal edge detection and qualification.
//!
//! This node converts generic level streams into qualified edge events. Its graph
//! sockets and UI controls are defined by the owning graph-node feature.

mod implementation;

pub use implementation::{EdgeDetector, EdgeMode};
