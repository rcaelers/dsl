use std::collections::HashMap;
use std::sync::Arc;

use logic_analyzer_graph_plan::{
    DiscoveredCapturePresentation, ProcessingGraph, ProcessingGraphError,
};
use node_graph::api::NodeId;
use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};
use platform_runtime::{InlineWorkExecutor, WorkExecutor};
use signal_derived::PersistentStoreConfig;
use signal_runtime::{AppManagerFactory, ConfigurationBoundary, CooperativeAppManagerFactory};

use super::execution::{
    self, ApplySummary, GraphRunContext, LiveAnalysisSource, LiveRun, SourceProcessOverrides,
};
use super::source_preparation::SourcePreparation;
use super::{
    ApplyError, DerivedCacheClearStats, DerivedCacheClearTask, DerivedCacheEntrySnapshot,
    InlineSourcePreparationExecutor, SourcePreparationExecutor, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate, cache_policy,
};

/// Composes execution-lifetime graph services.
///
/// Lowering remains explicit: start, cache, and reconciliation methods consume a previously
/// produced [`ProcessingGraph`]. The runtime therefore cannot inspect or rewrite an editable graph
/// document while starting work.
pub struct GraphRuntime {
    source_preparation: SourcePreparation,
    runtime_factory: Arc<dyn AppManagerFactory>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

impl GraphRuntime {
    /// Creates a cooperative in-memory runtime.
    pub fn new() -> Self {
        Self::with_execution(
            Box::new(InlineSourcePreparationExecutor),
            Arc::new(CooperativeAppManagerFactory),
            Arc::new(InlineWorkExecutor),
        )
    }

    /// Creates a runtime with host-selected execution services.
    pub fn with_execution(
        source_preparation_executor: Box<dyn SourcePreparationExecutor>,
        runtime_factory: Arc<dyn AppManagerFactory>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            source_preparation: SourcePreparation::with_execution(
                source_preparation_executor,
                Arc::clone(&work_executor),
            ),
            runtime_factory,
            work_executor,
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }

    /// Supplies the repository used by preparation, runs, and cache policy.
    pub fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.source_preparation
            .set_artifact_repository(Arc::clone(&repository));
        self.artifact_repository = repository;
    }

    /// Synchronizes finite-source preparation from compiler discovery output.
    pub fn synchronize_prepared_capture(
        &mut self,
        discovered: Result<Option<DiscoveredCapturePresentation>, String>,
    ) -> SourcePreparationUpdate {
        match discovered {
            Ok(discovered) => self.source_preparation.synchronize(discovered),
            Err(error) => self.source_preparation.fail(error),
        }
    }

    /// Forgets the current source-preparation generation.
    pub fn reset_prepared_capture(&mut self) {
        self.source_preparation.reset();
    }

    /// Returns the current finite-source preparation phase.
    pub fn source_preparation_status(&self) -> SourcePreparationStatus {
        self.source_preparation.status()
    }

    /// Returns source-preparation state and progress.
    pub fn source_preparation_snapshot(&self) -> SourcePreparationSnapshot {
        self.source_preparation.snapshot()
    }

    /// Returns persistent cache configurations grouped by affected graph node.
    pub fn derived_cache_configs_by_node(
        &self,
        compiled: &ProcessingGraph,
    ) -> HashMap<NodeId, Vec<PersistentStoreConfig>> {
        execution::derived_cache_configs_by_node_with_subscriptions(
            compiled,
            &self.artifact_repository,
        )
    }

    /// Removes one persistent derived-data cache entry.
    pub fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, String> {
        cache_policy::clear_entry(config)
    }

    /// Starts host-scheduled cleanup of all persistent derived-data caches.
    pub fn start_clear_derived_caches(&self) -> Result<DerivedCacheClearTask, String> {
        cache_policy::start_clear_repository(&self.artifact_repository, &self.work_executor)
    }

    /// Immediately removes all persistent derived-data cache entries.
    pub fn clear_derived_caches(&self) -> Result<DerivedCacheClearStats, String> {
        cache_policy::clear_repository(&self.artifact_repository)
    }

    /// Inspects one persistent derived-data cache entry.
    pub fn inspect_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<DerivedCacheEntrySnapshot>, String> {
        cache_policy::inspect_entry(config)
    }

    /// Loads cached presentation data from an already-lowered plan.
    pub fn load_cached_data(
        &self,
        compiled: ProcessingGraph,
        context: &mut GraphRunContext,
    ) -> Result<bool, Vec<ProcessingGraphError>> {
        self.configure_context(context);
        execution::load_cached_data_with_subscriptions(compiled, context)
    }

    /// Materializes and starts an already-lowered plan.
    pub fn start(
        &self,
        compiled: ProcessingGraph,
        context: &mut GraphRunContext,
        source_overrides: SourceProcessOverrides,
    ) -> Result<LiveRun, Vec<ProcessingGraphError>> {
        self.configure_context(context);
        execution::start_app_run_with_source_overrides_and_subscriptions(
            compiled,
            context,
            source_overrides,
            self.runtime_factory.as_ref(),
        )
    }

    /// Starts live analysis from an already-lowered plan and provider-owned source process.
    pub fn start_live_analysis(
        &self,
        compiled: ProcessingGraph,
        context: &mut GraphRunContext,
        source: LiveAnalysisSource,
    ) -> Result<LiveRun, Vec<ProcessingGraphError>> {
        self.configure_context(context);
        execution::start_live_analysis_with_subscriptions(
            compiled,
            context,
            source,
            self.runtime_factory.as_ref(),
        )
    }

    /// Reconciles an active run against a newly lowered plan.
    pub fn apply(
        &self,
        run: &mut LiveRun,
        compiled: ProcessingGraph,
    ) -> Result<ApplySummary, ApplyError> {
        run.apply_compiled(compiled)
    }

    /// Applies hot changes from a newly lowered plan at an explicit boundary.
    pub fn apply_configuration_epoch(
        &self,
        run: &mut LiveRun,
        compiled: ProcessingGraph,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        run.apply_configuration_epoch_compiled(compiled, boundary)
    }

    fn configure_context(&self, context: &mut GraphRunContext) {
        context.set_work_executor(Arc::clone(&self.work_executor));
        context.set_artifact_repository(Arc::clone(&self.artifact_repository));
    }
}

impl Default for GraphRuntime {
    fn default() -> Self {
        Self::new()
    }
}
