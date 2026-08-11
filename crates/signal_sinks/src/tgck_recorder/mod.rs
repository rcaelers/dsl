//! # `tgck_recorder`
//!
//! ## Responsibility
//!
//! This module owns TGCK recorder output for its configured processing stream.
//!
//! ## Boundaries
//!
//! It owns recorder encoding, not graph-node state, file destination acquisition, or target-specific
//! storage implementation.

//! TGCK recorder processing node.
//!
//! It owns TGCK stream recording behavior and delegates output persistence through
//! its explicit storage contract; it does not select host APIs or UI policy.

mod recorder;

pub use recorder::TgckRecorder;
