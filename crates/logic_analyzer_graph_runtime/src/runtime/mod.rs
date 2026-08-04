//! Graph execution owner.
//!
//! This module owns host-injected execution resources and the lifecycle of prepared and active
//! graph runs. Its execution entry points consume immutable graph-plan contracts and have no
//! compiler or registry dependency; document discovery and lowering never begin runtime work.

mod cache_policy;
mod data_collector;
mod derived_cache_backend;
mod errors;
mod execution;
mod run_data;
mod service;
mod source_preparation;
mod source_preparation_contract;
mod source_preparation_executor;

pub use cache_policy::{DerivedCacheClearStats, DerivedCacheClearTask, DerivedCacheEntrySnapshot};
pub use errors::ApplyError;
pub use execution::{
    ApplySummary, GraphRunContext, LiveAnalysisSource, LiveRun, SourceProcessOverrides,
};
pub use run_data::{
    RunData, RunDiagnostic, RunDiagnosticRegistry, RunDiagnosticSeverity, SourceArtifactReadiness,
    SourceDataKind, SourceReadiness, SourceReadinessRegistry,
};
pub use service::GraphRuntime;
pub use source_preparation_contract::{
    PreparedCapture, PreparedCaptureData, PreparingCapture, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate,
};
pub use source_preparation_executor::{
    CaptureWorkerSourcePreparationExecutor, InlineSourcePreparationExecutor,
    SourcePreparationControl, SourcePreparationExecutor, SourcePreparationResult,
    SourcePreparationTask, SourcePreparationTaskUpdate, SourcePreparationWork,
};
