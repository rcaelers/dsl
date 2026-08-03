use std::collections::HashMap;
use std::sync::Arc;

use logic_analyzer_graph_api::node::RuntimeBuilderOverride;
use logic_analyzer_graph_api::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use node_graph::api::{GraphState, NodeId};
use signal_processing::{
    AppManagerFactory, ArtifactRepository, ConfigurationBoundary, CooperativeAppManagerFactory,
    InlineWorkExecutor, MemoryArtifactRepository, PayloadRegistry, PersistentStoreConfig,
    WorkExecutor,
};

use super::errors::{ApplyError, CompileError};
use super::graph::{
    ApplySummary, BuilderRegistry, CompileCtx, CompiledGraph, DiscoveredCapturePresentation,
    DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration, LiveAnalysisSource,
    LiveCaptureDiscoveryError, LiveRun, SamplingOverlayCandidate, SourceProcessOverrides,
};
use super::source_preparation::SourcePreparation;
use super::{
    DerivedCacheClearStats, DerivedCacheClearTask, DerivedCacheEntrySnapshot, GraphWorkerClient,
    OutputSubscriptionPlan, SourcePreparationExecutor, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate, cache_policy, graph,
};

/// Stateful application-facing facade for graph discovery, compilation, and execution.
///
/// The compiler owns its inventory-derived runtime registry. Hosts supply graph documents and an
/// application-neutral output-subscription plan, then consume resolved results without
/// coordinating individual compiler functions or handling node builders directly.
pub struct GraphCompiler {
    builders: BuilderRegistry,
    output_subscriptions: OutputSubscriptionPlan,
    source_preparation: SourcePreparation,
    runtime_factory: Arc<dyn AppManagerFactory>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
    graph_worker_client: Option<Arc<GraphWorkerClient>>,
}

impl GraphCompiler {
    /// Creates a compiler with cooperative in-memory host services.
    pub fn new() -> Self {
        Self {
            builders: BuilderRegistry::standard(),
            output_subscriptions: OutputSubscriptionPlan::new(),
            source_preparation: SourcePreparation::new(),
            runtime_factory: Arc::new(CooperativeAppManagerFactory),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
            graph_worker_client: None,
        }
    }

    /// Constructs a compiler with host-selected finite-source preparation.
    ///
    /// # Parameters
    /// - `executor`: Host executor for finite-source preparation tasks.
    pub fn with_source_preparation_executor(executor: Box<dyn SourcePreparationExecutor>) -> Self {
        Self {
            builders: BuilderRegistry::standard(),
            output_subscriptions: OutputSubscriptionPlan::new(),
            source_preparation: SourcePreparation::with_executor(executor),
            runtime_factory: Arc::new(CooperativeAppManagerFactory),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
            graph_worker_client: None,
        }
    }

    /// Constructs a compiler with host-selected source and graph execution.
    pub fn with_execution(
        source_preparation_executor: Box<dyn SourcePreparationExecutor>,
        runtime_factory: Arc<dyn AppManagerFactory>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self::with_execution_and_builder_overrides(
            source_preparation_executor,
            runtime_factory,
            work_executor,
            Vec::new(),
        )
    }

    /// Constructs a compiler with host-selected execution and node factories.
    pub fn with_execution_and_builder_overrides(
        source_preparation_executor: Box<dyn SourcePreparationExecutor>,
        runtime_factory: Arc<dyn AppManagerFactory>,
        work_executor: Arc<dyn WorkExecutor>,
        builder_overrides: Vec<RuntimeBuilderOverride>,
    ) -> Self {
        Self {
            builders: BuilderRegistry::standard_with_overrides(builder_overrides),
            output_subscriptions: OutputSubscriptionPlan::new(),
            source_preparation: SourcePreparation::with_execution(
                source_preparation_executor,
                Arc::clone(&work_executor),
            ),
            runtime_factory,
            work_executor,
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
            graph_worker_client: None,
        }
    }

    /// Sets artifact repository.
    pub fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.source_preparation
            .set_artifact_repository(Arc::clone(&repository));
        self.artifact_repository = repository;
    }

    /// Sets output subscriptions.
    pub fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.output_subscriptions = subscriptions;
    }

    /// Returns the configured retained and visible output selection.
    pub fn output_subscriptions(&self) -> &OutputSubscriptionPlan {
        &self.output_subscriptions
    }

    /// Sets graph worker client.
    pub fn set_graph_worker_client(&mut self, client: Option<Arc<GraphWorkerClient>>) {
        self.graph_worker_client = client;
    }

    /// Returns the optional client used to delegate graph execution to a worker.
    pub fn graph_worker_client(&self) -> Option<Arc<GraphWorkerClient>> {
        self.graph_worker_client.clone()
    }

    /// Returns discovered payload registrations available to graph lowering.
    pub fn payloads(&self) -> &PayloadRegistry {
        self.builders.payloads()
    }

    /// Discovers the capture source's viewer presentation for the current graph.
    ///
    /// # Parameters
    /// - `graph`: Editor graph to inspect without executing it.
    pub fn discover_capture_presentation(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredCapturePresentation>, String> {
        graph::discover_capture_presentation_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
        )
    }

    /// Advances source-owned preload/index preparation and returns only state changes.
    pub fn synchronize_prepared_capture(&mut self, graph: &GraphState) -> SourcePreparationUpdate {
        let discovered = graph::discover_capture_presentation_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
        );
        match discovered {
            Ok(discovered) => self.source_preparation.synchronize(discovered),
            Err(error) => self.source_preparation.fail(error),
        }
    }

    /// Forgets source preparation state so the current graph is prepared again.
    pub fn reset_prepared_capture(&mut self) {
        self.source_preparation.reset();
    }

    /// Returns the current finite-source preparation lifecycle phase.
    pub fn source_preparation_status(&self) -> SourcePreparationStatus {
        self.source_preparation.status()
    }

    /// Returns the current finite-source preparation snapshot and progress.
    pub fn source_preparation_snapshot(&self) -> SourcePreparationSnapshot {
        self.source_preparation.snapshot()
    }

    /// Discovers the graph's single live-capture feature, if present.
    pub fn discover_live_capture_feature(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
        graph::discover_live_capture_feature_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
        )
    }

    /// Discovers validated trigger configuration from the live-capture source.
    pub fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
        graph::discover_trigger_configuration(graph, &self.builders)
    }

    /// Applies a trigger edit to the live-capture source's persisted state.
    ///
    /// # Parameters
    /// - `graph`: Graph containing the live-capture source.
    /// - `source_node`: Source node that owns the edit.
    /// - `edit`: Requested simple or advanced trigger change.
    pub fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_live_capture_edit(graph, &self.builders, source_node, edit)
    }

    /// Discovers markers contributed by concrete graph nodes.
    pub fn discover_timeline_markers(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, String> {
        graph::discover_timeline_markers(graph, &self.builders)
    }

    /// Applies a host marker edit to its owning node's persisted state.
    ///
    /// # Parameters
    /// - `graph`: Graph containing the marker owner.
    /// - `owner_node`: Node that owns the edited marker.
    /// - `edit`: Requested marker timestamp update.
    pub fn apply_timeline_marker_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_timeline_marker_edit(graph, &self.builders, owner_node, edit)
    }

    /// Discovers controls that reference host-owned timeline markers.
    ///
    /// # Parameters
    /// - `graph`: Graph to inspect for reference controls.
    pub fn discover_timeline_marker_reference_bindings(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, String> {
        graph::discover_timeline_marker_reference_bindings(graph, &self.builders)
    }

    /// Applies new host reference choices to a node-owned control.
    ///
    /// # Parameters
    /// - `graph`: Graph containing the control owner.
    /// - `owner_node`: Node that owns the edited control.
    /// - `edit`: New choices to synchronize into node state.
    pub fn apply_timeline_marker_reference_binding_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_timeline_marker_reference_binding_edit(graph, &self.builders, owner_node, edit)
    }

    /// Lowers an editor graph into a materializable runtime description.
    pub fn lower(&self, graph: &GraphState) -> Result<CompiledGraph, Vec<CompileError>> {
        graph::lower_with_subscriptions(graph, &self.builders, &self.output_subscriptions)
    }

    /// Resolves sampling overlays available for presentation in the graph.
    pub fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
        graph::sampling_overlay_candidates(graph, &self.builders, &self.output_subscriptions)
    }

    /// Returns persistent derived-data cache configurations grouped by source node.
    pub fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
    ) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
        graph::derived_cache_configs_by_node_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
            &self.artifact_repository,
        )
    }

    /// Clears derived cache entry.
    pub fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, String> {
        cache_policy::clear_entry(config)
    }

    /// Clears derived caches.
    pub fn clear_derived_caches(&self) -> Result<DerivedCacheClearStats, String> {
        cache_policy::clear_repository(&self.artifact_repository)
    }

    /// Starts host-scheduled cleanup of all persistent derived-data caches.
    pub fn start_clear_derived_caches(&self) -> Result<DerivedCacheClearTask, String> {
        cache_policy::start_clear_repository(&self.artifact_repository, &self.work_executor)
    }

    /// Inspects size and timestamp diagnostics for one derived-data cache entry.
    pub fn inspect_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<DerivedCacheEntrySnapshot>, String> {
        cache_policy::inspect_entry(config)
    }

    /// Lowers and starts an in-process graph run using the supplied context.
    ///
    /// # Parameters
    /// - `graph`: Editor graph to lower and materialize.
    /// - `ctx`: Run-scoped host services and timeline state.
    pub fn start_app_run(
        &self,
        graph: &GraphState,
        ctx: &mut CompileCtx,
    ) -> Result<LiveRun, Vec<CompileError>> {
        self.configure_context(ctx);
        graph::start_app_run(
            graph,
            &self.builders,
            &self.output_subscriptions,
            ctx,
            self.runtime_factory.as_ref(),
        )
    }

    /// Loads persistent derived lanes for presentation without executing producers or sinks.
    ///
    /// # Parameters
    /// - `graph`: Editor graph whose persistent derived lanes should be loaded.
    /// - `ctx`: Run-scoped host services and timeline state.
    pub fn load_cached_data(
        &self,
        graph: &GraphState,
        ctx: &mut CompileCtx,
    ) -> Result<bool, Vec<CompileError>> {
        self.configure_context(ctx);
        graph::load_cached_data_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
            ctx,
        )
    }

    /// Starts a graph run while replacing selected source nodes with supplied processes.
    ///
    /// # Parameters
    /// - `graph`: Editor graph to lower and materialize.
    /// - `ctx`: Run-scoped host services and timeline state.
    /// - `overrides`: Concrete source processes keyed by source node ID.
    pub fn start_app_run_with_source_overrides(
        &self,
        graph: &GraphState,
        ctx: &mut CompileCtx,
        overrides: SourceProcessOverrides,
    ) -> Result<LiveRun, Vec<CompileError>> {
        self.configure_context(ctx);
        graph::start_app_run_with_source_overrides_and_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
            ctx,
            overrides,
            self.runtime_factory.as_ref(),
        )
    }

    /// Starts live analysis using a provider-owned source process.
    ///
    /// # Parameters
    /// - `graph`: Editor graph to lower and materialize.
    /// - `ctx`: Run-scoped host services and timeline state.
    /// - `source`: Provider-owned live source substituted into the graph.
    pub fn start_live_analysis(
        &self,
        graph: &GraphState,
        ctx: &mut CompileCtx,
        source: LiveAnalysisSource,
    ) -> Result<LiveRun, Vec<CompileError>> {
        self.configure_context(ctx);
        graph::start_live_analysis_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
            ctx,
            source,
            self.runtime_factory.as_ref(),
        )
    }

    fn configure_context(&self, ctx: &mut CompileCtx) {
        ctx.set_work_executor(Arc::clone(&self.work_executor));
        ctx.set_artifact_repository(Arc::clone(&self.artifact_repository));
    }

    /// Applies compatible graph edits to an active run.
    ///
    /// # Parameters
    /// - `run`: Active run to reconfigure.
    /// - `graph`: Updated editor graph.
    pub fn apply_run(
        &self,
        run: &mut LiveRun,
        graph: &GraphState,
    ) -> Result<ApplySummary, ApplyError> {
        run.apply_with_subscriptions(graph, &self.builders, &self.output_subscriptions)
    }

    /// Applies compatible edits at an explicit application configuration boundary.
    ///
    /// # Parameters
    /// - `run`: Active run to reconfigure.
    /// - `graph`: Updated editor graph.
    /// - `boundary`: Application-defined boundary that permits configuration changes.
    pub fn apply_configuration_epoch(
        &self,
        run: &mut LiveRun,
        graph: &GraphState,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        run.apply_configuration_epoch(graph, &self.builders, &self.output_subscriptions, boundary)
    }
}

impl Default for GraphCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod graph_compiler_storage_tests {
    use std::sync::Arc;

    use logic_analyzer_graph_api::node_support::NodeBuildContext;
    use signal_processing::{ArtifactRepository, MemoryArtifactRepository};

    use super::{CompileCtx, GraphCompiler};

    #[test]
    fn host_repository_reaches_every_node_build_context() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let mut compiler = GraphCompiler::new();
        compiler.set_artifact_repository(Arc::clone(&repository));
        let mut context = CompileCtx::default();

        compiler.configure_context(&mut context);

        assert!(Arc::ptr_eq(&context.artifact_repository(), &repository));
    }
}
