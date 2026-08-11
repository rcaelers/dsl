//! Generic graph document model and egui editor widget.
//!
//! [`api`] contains the compiler- and plugin-facing document and node-definition
//! contracts. The crate-root facade supplies editor composition. Concrete node
//! behavior, graph compiler policy, protocol semantics, and host adapters remain
//! outside this reusable widget crate.

#[cfg(test)]
mod architecture_tests;

pub mod api;
mod model;
mod runtime;
mod support;
mod widget;

pub use widget::{GraphSnapshotError, GraphUiPrefs, NodeContextAction, NodeGraphWidget};
