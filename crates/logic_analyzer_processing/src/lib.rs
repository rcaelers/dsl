//! Concrete, UI-independent logic-analyzer processing behavior.
//!
//! This crate owns capture formats and devices, protocol decoders, processing
//! nodes, and sinks. It translates its format and transport failures at the
//! [`signal_processing`] boundary; it does not own graph definitions, saved-node
//! migration, socket presentation, UI controls, or host selection.
//!
//! The root contains shared capture-source metadata and process-node construction.
//! Supported concrete node contracts are grouped under [`nodes`], while
//! protocol-neutral processing values are under [`types`]. Hosts inject file,
//! device, and execution capabilities through these contracts.

#[cfg(test)]
mod architecture_tests;
mod capture_source_metadata;
mod process_node_construction;

pub mod nodes;
mod support;
pub mod types;

pub use capture_source_metadata::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, CaptureSourceRuntimeCapabilities, CaptureSourceSignal,
};
pub use process_node_construction::ProcessNodeConstruction;
