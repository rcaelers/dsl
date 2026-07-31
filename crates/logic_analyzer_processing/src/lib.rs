//! Concrete, UI-independent logic-analyzer processing nodes.

#[cfg(test)]
mod architecture_tests;
mod capture_source_metadata;
mod process_node_construction;

pub mod nodes;
#[cfg(not(target_arch = "wasm32"))]
mod support;
pub mod types;

pub use capture_source_metadata::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, CaptureSourceRuntimeCapabilities, CaptureSourceSignal,
};
pub use process_node_construction::ProcessNodeConstruction;
