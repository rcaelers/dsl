use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use logic_analyzer_graph_api::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use logic_analyzer_graph_compiler::{
    ApplyError, ApplySummary, CollectedOutputSubscription, CollectedTableSubscription, CompileCtx,
    CompileError, DerivedCacheClearStats, DerivedCacheEntrySnapshot, DiscoveredLiveCaptureFeature,
    DiscoveredTimelineMarker, DiscoveredTimelineMarkerReferenceBinding,
    DiscoveredTriggerConfiguration, LiveAnalysisSource, LiveCaptureDiscoveryError,
    OutputSubscriptionPlan, SamplingOverlayCandidate, SourcePreparationStatus,
    SourcePreparationUpdate, SourceProcessOverrides, SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use signal_processing::{
    ArtifactRepository, ConfigurationBoundary, DisconnectEvent, PersistentStoreConfig,
};

use crate::live_capture::CaptureFeatureDiscovery;

pub(crate) trait GraphRun {
    fn as_any_mut(&mut self) -> &mut dyn Any;

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

    fn progress(&self) -> Vec<(NodeId, u64)>;

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)>;

    fn take_failure(&mut self) -> Option<String> {
        None
    }
}

pub(crate) trait GraphService: CaptureFeatureDiscovery {
    fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>);

    fn set_graph_worker_client(
        &mut self,
        _client: Option<Arc<logic_analyzer_graph_compiler::GraphWorkerClient>>,
    ) {
    }

    fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
    ) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>>;

    fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, String>;

    #[allow(
        dead_code,
        reason = "web cache policy is active although its UI has no clear-all command yet"
    )]
    fn clear_derived_caches(&self) -> Result<DerivedCacheClearStats, String>;

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
        context: &mut CompileCtx,
    ) -> Result<bool, Vec<CompileError>>;

    fn start_run(
        &self,
        graph: &GraphState,
        context: &mut CompileCtx,
        source_overrides: SourceProcessOverrides,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>>;

    fn start_live_analysis(
        &self,
        graph: &GraphState,
        context: &mut CompileCtx,
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
