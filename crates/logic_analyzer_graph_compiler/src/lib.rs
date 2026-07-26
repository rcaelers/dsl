//! Logic-analyzer graph-to-runtime compiler and application-host services.
//!
//! This crate lowers a generic [`node_graph::api`] document through inventory-submitted node contracts
//! into the UI-independent [`signal_processing`] runtime. Concrete graph nodes and their
//! presentations live in `logic-analyzer-graph-nodes`; application composition and window
//! integration belong in `logic-analyzer-ui`.

mod data_collector;
mod errors;
mod graph;
mod graph_compiler;
mod graph_node_registration;
mod output_subscription;
mod payload_registration;
mod run_data;
#[cfg(not(target_arch = "wasm32"))]
#[path = "source_preparation_native.rs"]
mod source_preparation;
#[cfg(target_arch = "wasm32")]
#[path = "source_preparation_wasm.rs"]
mod source_preparation;
mod source_preparation_contract;

#[cfg(test)]
mod architecture_tests;

#[cfg(not(target_arch = "wasm32"))]
#[path = "cache_platform_native.rs"]
mod cache_platform;
#[cfg(target_arch = "wasm32")]
#[path = "cache_platform_wasm.rs"]
mod cache_platform;

pub(crate) use data_collector::{
    BUILDER_NAME as DATA_COLLECTOR_BUILDER, DataCollectorBuilder, OUTPUT_SUBSCRIPTION_BUILDER_NAME,
};
pub use errors::{ApplyError, CompileError};
pub use graph::{
    ApplySummary, CompileCtx, CompiledEdge, CompiledGraph, CompiledNode,
    DiscoveredCapturePresentation, DiscoveredLiveCaptureFeature, DiscoveredTriggerConfiguration,
    LiveAnalysisSource, LiveCaptureDiscoveryError, LiveRun, ResolvedSamplingOverlay,
    ResolvedSamplingQualifier, SamplingOverlayCandidate, SourceProcessOverrides,
};
pub use graph_compiler::GraphCompiler;
pub(crate) use graph_node_registration::{
    standard_graph_node_builders, validate_graph_node_payload_requirements,
};
pub use output_subscription::{
    CollectedOutputLane, CollectedOutputSubscription, CollectedTableSubscription,
    OutputSubscriptionPlan,
};
pub(crate) use payload_registration::payload_registrations;
pub use run_data::{
    RunData, RunDiagnostic, RunDiagnosticRegistry, RunDiagnosticSeverity, SourceArtifactReadiness,
    SourceDataKind, SourceReadiness, SourceReadinessRegistry,
};
pub use source_preparation_contract::{
    PreparedCapture, PreparedCaptureData, SourcePreparationStatus, SourcePreparationUpdate,
};
