use logic_analyzer_graph_api::node::RuntimeBuilderOverride;
use logic_analyzer_graph_api::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use logic_analyzer_graph_compiler::{
    ApplyError, ApplySummary, CollectedOutputSubscription, CollectedTableSubscription, CompileCtx,
    CompileError, DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration, GraphCompiler,
    LiveAnalysisSource, LiveCaptureDiscoveryError, LiveRun, OutputSubscriptionPlan,
    SamplingOverlayCandidate, SourcePreparationExecutor, SourcePreparationSnapshot,
    SourcePreparationStatus, SourcePreparationUpdate, SourceProcessOverrides,
    SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use signal_processing::{
    AppManagerFactory, ConfigurationBoundary, DerivedLanes, DisconnectEvent, PersistentStoreConfig,
    WorkExecutor,
};

use super::contract::{GraphRun, GraphService};
use crate::live_capture::{CaptureAvailability, CaptureFeatureDiscovery};

impl GraphRun for LiveRun {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        LiveRun::persistent_cache_configs(self)
    }

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        LiveRun::sampling_overlays(self)
    }

    fn derived_lanes(&self) -> &DerivedLanes {
        LiveRun::derived_lanes(self)
    }

    fn output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        LiveRun::collected_output_subscriptions(self)
    }

    fn table_subscriptions(&self) -> &[CollectedTableSubscription] {
        LiveRun::collected_table_subscriptions(self)
    }

    fn source_readiness(&self) -> &SourceReadinessRegistry {
        LiveRun::source_readiness(self)
    }

    fn is_finished(&self) -> bool {
        LiveRun::is_finished(self)
    }

    fn is_stopping(&self) -> bool {
        LiveRun::is_stopping(self)
    }

    fn stop(&mut self) {
        LiveRun::stop(self);
    }

    fn pump(&mut self, budget: usize) {
        LiveRun::pump(self, budget);
    }

    fn progress(&self) -> Vec<(NodeId, u64)> {
        LiveRun::progress(self)
    }

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        LiveRun::take_disconnected(self)
    }
}

fn concrete_run(run: &mut dyn GraphRun) -> Result<&mut LiveRun, ApplyError> {
    run.as_any_mut()
        .downcast_mut::<LiveRun>()
        .ok_or_else(|| ApplyError::Apply("graph run was not created by GraphCompiler".into()))
}

impl CaptureFeatureDiscovery for GraphCompiler {
    fn discover_capture_availability(&self, graph: &GraphState) -> CaptureAvailability {
        match self.discover_live_capture_feature(graph) {
            Ok(Some(feature)) => CaptureAvailability::Available {
                source_node: feature.source_node(),
                source_title: feature.source_title().to_owned(),
                has_trigger_program: feature.has_trigger_program(),
                capabilities: feature.capabilities().clone(),
                session_plan: feature.session_plan().cloned().map(Box::new),
            },
            Ok(None) => CaptureAvailability::Unavailable {
                reason: "The graph has no live capture source".into(),
            },
            Err(error) => CaptureAvailability::Unavailable {
                reason: error.message,
            },
        }
    }
}

impl GraphService for GraphCompiler {
    fn set_artifact_repository(
        &mut self,
        repository: std::sync::Arc<dyn signal_processing::ArtifactRepository>,
    ) {
        GraphCompiler::set_artifact_repository(self, repository);
    }

    fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
    ) -> Result<std::collections::HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>>
    {
        GraphCompiler::derived_cache_configs_by_node(self, graph)
    }

    fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<logic_analyzer_graph_compiler::DerivedCacheClearStats, String> {
        GraphCompiler::clear_derived_cache_entry(self, config)
    }

    fn clear_derived_caches(
        &self,
    ) -> Result<logic_analyzer_graph_compiler::DerivedCacheClearStats, String> {
        GraphCompiler::clear_derived_caches(self)
    }

    fn inspect_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<logic_analyzer_graph_compiler::DerivedCacheEntrySnapshot>, String> {
        GraphCompiler::inspect_derived_cache_entry(self, config)
    }

    fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        GraphCompiler::set_output_subscriptions(self, subscriptions);
    }

    fn synchronize_prepared_capture(&mut self, graph: &GraphState) -> SourcePreparationUpdate {
        GraphCompiler::synchronize_prepared_capture(self, graph)
    }

    fn reset_prepared_capture(&mut self) {
        GraphCompiler::reset_prepared_capture(self);
    }

    fn source_preparation_status(&self) -> SourcePreparationStatus {
        GraphCompiler::source_preparation_status(self)
    }

    fn source_preparation_snapshot(&self) -> SourcePreparationSnapshot {
        GraphCompiler::source_preparation_snapshot(self)
    }

    fn discover_live_capture_feature(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
        GraphCompiler::discover_live_capture_feature(self, graph)
    }

    fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
        GraphCompiler::discover_trigger_configuration(self, graph)
    }

    fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String> {
        GraphCompiler::apply_live_capture_edit(self, graph, source_node, edit)
    }

    fn discover_timeline_markers(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, String> {
        GraphCompiler::discover_timeline_markers(self, graph)
    }

    fn apply_timeline_marker_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, String> {
        GraphCompiler::apply_timeline_marker_edit(self, graph, owner_node, edit)
    }

    fn discover_timeline_marker_reference_bindings(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, String> {
        GraphCompiler::discover_timeline_marker_reference_bindings(self, graph)
    }

    fn apply_timeline_marker_reference_binding_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, String> {
        GraphCompiler::apply_timeline_marker_reference_binding_edit(self, graph, owner_node, edit)
    }

    fn graph_contains_node(
        &self,
        graph: &GraphState,
        node: NodeId,
    ) -> Result<bool, Vec<CompileError>> {
        GraphCompiler::lower(self, graph).map(|compiled| {
            compiled
                .nodes
                .into_iter()
                .any(|compiled_node| compiled_node.id == node)
        })
    }

    fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
        GraphCompiler::sampling_overlay_candidates(self, graph)
    }

    fn load_cached_data(
        &self,
        graph: &GraphState,
        context: &mut CompileCtx,
    ) -> Result<bool, Vec<CompileError>> {
        GraphCompiler::load_cached_data(self, graph, context)
    }

    fn start_run(
        &self,
        graph: &GraphState,
        context: &mut CompileCtx,
        source_overrides: SourceProcessOverrides,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        GraphCompiler::start_app_run_with_source_overrides(self, graph, context, source_overrides)
            .map(|run| Box::new(run) as Box<dyn GraphRun>)
    }

    fn start_live_analysis(
        &self,
        graph: &GraphState,
        context: &mut CompileCtx,
        source: LiveAnalysisSource,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        GraphCompiler::start_live_analysis(self, graph, context, source)
            .map(|run| Box::new(run) as Box<dyn GraphRun>)
    }

    fn apply_run(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
    ) -> Result<ApplySummary, ApplyError> {
        GraphCompiler::apply_run(self, concrete_run(run)?, graph)
    }

    fn apply_configuration_epoch(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        GraphCompiler::apply_configuration_epoch(self, concrete_run(run)?, graph, boundary)
    }
}

pub(crate) fn standard_graph_service() -> Box<dyn GraphService> {
    Box::new(GraphCompiler::new())
}

pub(crate) fn graph_service_with_execution(
    source_preparation_executor: Box<dyn SourcePreparationExecutor>,
    runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
    work_executor: std::sync::Arc<dyn WorkExecutor>,
) -> Box<dyn GraphService> {
    graph_service_with_execution_and_builder_overrides(
        source_preparation_executor,
        runtime_factory,
        work_executor,
        Vec::new(),
    )
}

pub(crate) fn graph_service_with_execution_and_builder_overrides(
    source_preparation_executor: Box<dyn SourcePreparationExecutor>,
    runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
    work_executor: std::sync::Arc<dyn WorkExecutor>,
    builder_overrides: Vec<RuntimeBuilderOverride>,
) -> Box<dyn GraphService> {
    Box::new(GraphCompiler::with_execution_and_builder_overrides(
        source_preparation_executor,
        runtime_factory,
        work_executor,
        builder_overrides,
    ))
}
