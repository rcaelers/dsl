use std::collections::HashMap;
use std::path::Path;
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
    OutputSubscriptionPlan, SourcePreparationExecutor, SourcePreparationStatus,
    SourcePreparationUpdate, graph,
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
}

impl GraphCompiler {
    pub fn new() -> Self {
        Self {
            builders: BuilderRegistry::standard(),
            output_subscriptions: OutputSubscriptionPlan::new(),
            source_preparation: SourcePreparation::new(),
            runtime_factory: Arc::new(CooperativeAppManagerFactory),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }

    /// Constructs a compiler with host-selected finite-source preparation.
    pub fn with_source_preparation_executor(executor: Box<dyn SourcePreparationExecutor>) -> Self {
        Self {
            builders: BuilderRegistry::standard(),
            output_subscriptions: OutputSubscriptionPlan::new(),
            source_preparation: SourcePreparation::with_executor(executor),
            runtime_factory: Arc::new(CooperativeAppManagerFactory),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
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
        }
    }

    pub fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.artifact_repository = repository;
    }

    pub fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.output_subscriptions = subscriptions;
    }

    pub fn payloads(&self) -> &PayloadRegistry {
        self.builders.payloads()
    }

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

    pub fn source_preparation_status(&self) -> SourcePreparationStatus {
        self.source_preparation.status()
    }

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

    pub fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
        graph::discover_trigger_configuration(graph, &self.builders)
    }

    pub fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_live_capture_edit(graph, &self.builders, source_node, edit)
    }

    pub fn discover_timeline_markers(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, String> {
        graph::discover_timeline_markers(graph, &self.builders)
    }

    pub fn apply_timeline_marker_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_timeline_marker_edit(graph, &self.builders, owner_node, edit)
    }

    pub fn discover_timeline_marker_reference_bindings(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, String> {
        graph::discover_timeline_marker_reference_bindings(graph, &self.builders)
    }

    pub fn apply_timeline_marker_reference_binding_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_timeline_marker_reference_binding_edit(graph, &self.builders, owner_node, edit)
    }

    pub fn lower(&self, graph: &GraphState) -> Result<CompiledGraph, Vec<CompileError>> {
        graph::lower_with_subscriptions(graph, &self.builders, &self.output_subscriptions)
    }

    pub fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
        graph::sampling_overlay_candidates(graph, &self.builders, &self.output_subscriptions)
    }

    pub fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
        directory: &Path,
    ) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
        graph::derived_cache_configs_by_node_with_subscriptions(
            graph,
            &self.builders,
            &self.output_subscriptions,
            directory,
            &self.artifact_repository,
        )
    }

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

    pub fn apply_run(
        &self,
        run: &mut LiveRun,
        graph: &GraphState,
    ) -> Result<ApplySummary, ApplyError> {
        run.apply_with_subscriptions(graph, &self.builders, &self.output_subscriptions)
    }

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
