//! Logic-analyzer graph-to-runtime compiler and application-host services.
//!
//! This crate lowers a generic [`node_graph::api`] document through inventory-submitted node contracts
//! into the UI-independent [`signal_processing`] runtime. Concrete graph nodes and their
//! presentations live in `logic-analyzer-graph-nodes`; application composition and window
//! integration belong in `logic-analyzer-ui`.

mod cache_policy;
mod data_collector;
mod derived_cache_backend;
mod errors;
mod graph;
mod graph_compiler;
mod graph_node_registration;
mod output_subscription;
mod payload_registration;
mod run_data;
mod source_preparation;
mod source_preparation_contract;
mod source_preparation_executor;
mod worker_client;
mod worker_execution;
mod worker_execution_codec;

#[cfg(test)]
mod architecture_tests;

pub use cache_policy::{DerivedCacheClearStats, DerivedCacheEntrySnapshot};
pub(crate) use data_collector::{
    BUILDER_NAME as DATA_COLLECTOR_BUILDER, DataCollectorBuilder, OUTPUT_SUBSCRIPTION_BUILDER_NAME,
};
pub use errors::{ApplyError, CompileError};
pub use graph::{
    ApplySummary, CompileCtx, CompiledEdge, CompiledGraph, CompiledNode,
    DiscoveredCapturePresentation, DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration, LiveAnalysisSource,
    LiveCaptureDiscoveryError, LiveRun, ResolvedSamplingOverlay, SamplingOverlayCandidate,
    SourceProcessOverrides,
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
    PreparedCapture, PreparedCaptureData, PreparingCapture, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate,
};
pub use source_preparation_executor::{
    CaptureWorkerSourcePreparationExecutor, InlineSourcePreparationExecutor,
    SourcePreparationControl, SourcePreparationExecutor, SourcePreparationResult,
    SourcePreparationTask, SourcePreparationTaskUpdate, SourcePreparationWork,
};
pub use worker_client::GraphWorkerClient;
pub use worker_execution::{GraphWorkerMessage, GraphWorkerRequest, GraphWorkerRuntime};
pub use worker_execution_codec::{
    decode_graph_worker_messages, decode_graph_worker_request, encode_graph_worker_messages,
    encode_graph_worker_request,
};
