use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use logic_analyzer_graph_compiler::{
    DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration,
    LiveCaptureDiscoveryError,
};
use logic_analyzer_graph_orchestration::GraphWorkerClient;
use logic_analyzer_graph_plan::{
    CollectedOutputSubscription, CollectedTableSubscription, OutputSubscriptionPlan,
    ProcessingGraph, ProcessingGraphError as CompileError, SamplingOverlayCandidate,
};
use logic_analyzer_graph_runtime::{
    ApplyError, ApplySummary, DerivedCacheClearStats, DerivedCacheEntrySnapshot, GraphRunContext,
    LiveAnalysisSource, SourcePreparationStatus, SourcePreparationUpdate, SourceProcessOverrides,
    SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use platform_artifacts::ArtifactRepository;
use signal_derived::PersistentStoreConfig;
use signal_runtime::{ConfigurationBoundary, DisconnectEvent, NodeFailure};

use crate::live_capture::CaptureFeatureDiscovery;

pub(crate) type CachedDataLoader<'a> =
    dyn FnMut(&GraphState, &mut GraphRunContext) -> Result<bool, Vec<CompileError>> + 'a;

pub(crate) trait GraphRun {
    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig>;

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate];

    fn output_subscriptions(&self) -> &[CollectedOutputSubscription];

    fn table_subscriptions(&self) -> &[CollectedTableSubscription];

    fn source_readiness(&self) -> &SourceReadinessRegistry;

    fn is_finished(&self) -> bool;

    fn is_stopping(&self) -> bool;

    fn stop(&mut self);

    fn pump(&mut self, budget: usize);

    fn pump_for(&mut self, budget: usize, _max_duration: Duration) {
        self.pump(budget);
    }

    fn wait(&mut self) {
        while !self.is_finished() {
            self.pump_for(256, Duration::from_millis(4));
            std::thread::yield_now();
        }
    }

    fn progress(&self) -> Vec<(NodeId, u64)>;

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)>;

    fn take_node_failures(&mut self) -> Vec<(Option<NodeId>, NodeFailure)> {
        Vec::new()
    }

    fn take_failure(&mut self) -> Option<String> {
        None
    }

    fn apply_processing_graph(
        &mut self,
        _graph: ProcessingGraph,
        _boundary: Option<ConfigurationBoundary>,
    ) -> Result<ApplySummary, ApplyError> {
        Err(ApplyError::Apply(
            "this graph execution host does not support live plan updates".into(),
        ))
    }

    fn synchronize_cached_data(
        &mut self,
        _graph: &GraphState,
        _load: &mut CachedDataLoader<'_>,
    ) -> Result<bool, Vec<CompileError>> {
        Ok(false)
    }
}

pub(crate) trait GraphService: CaptureFeatureDiscovery {
    fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>);

    fn set_graph_worker_client(&mut self, _client: Option<Arc<GraphWorkerClient>>) {}

    fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
    ) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>>;

    fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, String>;

    fn start_clear_derived_caches(
        &self,
    ) -> Result<logic_analyzer_graph_runtime::DerivedCacheClearTask, String>;

    fn inspect_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<DerivedCacheEntrySnapshot>, String>;

    fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan);

    fn synchronize_prepared_capture(&mut self, graph: &GraphState) -> SourcePreparationUpdate;

    fn reset_prepared_capture(&mut self);

    fn source_preparation_status(&self) -> SourcePreparationStatus;

    fn discover_live_capture_feature(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError>;

    fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError>;

    fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String>;

    fn discover_timeline_markers(
        &self,
        _graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, String> {
        Ok(Vec::new())
    }

    fn apply_timeline_marker_edit(
        &self,
        _graph: &GraphState,
        _owner_node: NodeId,
        _edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, String> {
        Err("timeline-marker editing is unavailable".into())
    }

    fn discover_timeline_marker_reference_bindings(
        &self,
        _graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, String> {
        Ok(Vec::new())
    }

    fn apply_timeline_marker_reference_binding_edit(
        &self,
        _graph: &GraphState,
        _owner_node: NodeId,
        _edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, String> {
        Err("timeline-reference editing is unavailable".into())
    }

    fn graph_contains_node(
        &self,
        graph: &GraphState,
        node: NodeId,
    ) -> Result<bool, Vec<CompileError>>;

    fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>>;

    fn load_cached_data(
        &self,
        graph: &GraphState,
        context: &mut GraphRunContext,
    ) -> Result<bool, Vec<CompileError>>;

    fn start_run(
        &self,
        graph: &GraphState,
        context: &mut GraphRunContext,
        source_overrides: SourceProcessOverrides,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>>;

    fn start_live_analysis(
        &self,
        graph: &GraphState,
        context: &mut GraphRunContext,
        source: LiveAnalysisSource,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>>;

    fn apply_run(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
    ) -> Result<ApplySummary, ApplyError>;

    fn apply_configuration_epoch(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError>;

    fn synchronize_run_data(
        &self,
        _run: &mut dyn GraphRun,
        _graph: &GraphState,
    ) -> Result<bool, Vec<CompileError>> {
        Ok(false)
    }
}
