//! # `synthetic_capture_source`
//!
//! ## Responsibility
//!
//! This module owns an explicit portable synthetic capture source for authored demos and deterministic
//! processing scenarios.
//!
//! ## Boundaries
//!
//! It is not a target-dependent fallback for unavailable device or file access. Application or graph
//! configuration selects it explicitly.

//! Deterministic synthetic source and its native acquisition provider.
//!
//! This is an explicit portable test/demo source selected through configuration,
//! not a target-dependent substitute for a concrete capture device.

mod implementation;
mod presentation;

pub use implementation::SyntheticCaptureSource;
pub use presentation::synthetic_presentation;
