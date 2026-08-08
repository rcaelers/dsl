//! # `logic_analyzer_graph_capabilities::node`
//!
//! ## Responsibility
//!
//! This namespace owns capability traits implemented by graph-node and payload plugins, including
//! runtime capabilities and capture feature factories. Graph-node and payload inventory descriptors
//! and catalog assembly live in `logic_analyzer_graph_registry`.
//!
//! ## Boundaries
//!
//! It is an extension contract, not a compiler, node bundle, UI service, or host adapter. Implementers
//! use `node_support` for all supporting values and do not depend on concrete compiler internals. Its
//! Host catalog paths and scanning remain outside this plugin contract.

//! Contracts implemented by graph nodes and compile-time plugins.
//!
//! Graph-node and payload inventory registrations live in `logic_analyzer_graph_registry`. This
//! namespace intentionally contains no registry assembly, compiler policy, built-in-node behavior,
//! host paths, or UI operations.

mod contracts;

pub use contracts::{
    CaptureGraphSourceError, CaptureGraphSourceFactory, CaptureSourceFeature,
    CaptureSourceFeatureError, GraphNodeCapabilityBundle, GraphNodeCapabilityOverride,
    GraphNodePresentation, GraphNodeSemantics, LiveCaptureFeature, LiveCaptureFeatureProvider,
    RuntimeMaterializer, TimelineFeature,
};
