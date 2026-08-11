//! Materialization and lifecycle services for compiled logic-analyzer graphs.
//!
//! The compiler owns document semantics and produces immutable execution plans. This crate owns
//! the resources whose lifetime begins when a plan is prepared or run: repositories, executors,
//! runtime managers, source preparation, and cache maintenance. Processing-plan contracts live in
//! `logic-analyzer-graph-plan`; worker composition lives above this crate.

mod runtime;

pub use runtime::{
    ApplyError, ApplySummary, CaptureWorkerSourcePreparationExecutor, DerivedCacheClearStats,
    DerivedCacheClearTask, DerivedCacheEntrySnapshot, DerivedCacheError, GraphRunContext,
    GraphRuntime, GraphRuntimeError, InlineSourcePreparationExecutor, LiveAnalysisSource, LiveRun,
    PreparedCapture, PreparedCaptureData, PreparingCapture, RunData, RunDiagnostic,
    RunDiagnosticRegistry, RunDiagnosticSeverity, SourceArtifactReadiness, SourceDataKind,
    SourcePreparationControl, SourcePreparationError, SourcePreparationExecutor,
    SourcePreparationProtocolError, SourcePreparationResult, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationTask, SourcePreparationTaskUpdate,
    SourcePreparationUpdate, SourcePreparationWork, SourceProcessOverrides, SourceReadiness,
    SourceReadinessRegistry, ThreadedSourcePreparationExecutor,
};
