//! Deterministic logic-analyzer graph-document semantic analysis and lowering.
//!
//! This crate lowers a generic [`node_graph::api`] document through a validated
//! `logic_analyzer_graph_registry::GraphRegistry` into an immutable execution plan. Concrete graph nodes and their
//! presentations live in `logic-analyzer-graph-nodes`; application composition and window
//! integration belong in `logic-analyzer-ui`.
//!
//! The public facade owns document validation, discovery, edits, lowering, and diagnostics. It
//! consumes generic registry contracts rather than concrete node names or protocols. Built-in
//! node definitions, widget presentation, and target-specific adapters are outside
//! this crate.

mod data_collector;
mod graph;
mod graph_lowerer;
mod payload_catalog;

#[cfg(test)]
mod architecture_tests;

pub use graph::{
    DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration,
    LiveCaptureDiscoveryError,
};
pub use graph_lowerer::GraphLowerer;
