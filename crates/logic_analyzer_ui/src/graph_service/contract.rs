use std::any::Any;

use logic_analyzer_graph_api::node_support::LiveCaptureEdit;
use logic_analyzer_graph_compiler::{
    ApplyError, ApplySummary, CollectedOutputSubscription, CollectedTableSubscription, CompileCtx,
    CompileError, DiscoveredLiveCaptureFeature, DiscoveredTriggerConfiguration, LiveAnalysisSource,
    LiveCaptureDiscoveryError, OutputSubscriptionPlan, SamplingOverlayCandidate,
    SourcePreparationStatus, SourcePreparationUpdate, SourceProcessOverrides,
    SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use signal_processing::{
    ConfigurationBoundary, DerivedLanes, DisconnectEvent, PersistentStoreConfig,
};

use super::platform_contract::PlatformGraphService;
use crate::live_capture::CaptureFeatureDiscovery;

pub(crate) trait GraphRun {
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig>;

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate];

    fn derived_lanes(&self) -> &DerivedLanes;

    fn output_subscriptions(&self) -> &[CollectedOutputSubscription];

    fn table_subscriptions(&self) -> &[CollectedTableSubscription];

    fn source_readiness(&self) -> &SourceReadinessRegistry;

    fn is_finished(&self) -> bool;

    fn is_stopping(&self) -> bool;

    fn stop(&mut self);

    fn pump(&mut self, budget: usize);

    fn progress(&self) -> Vec<(NodeId, u64)>;

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)>;
}

pub(crate) trait GraphService: CaptureFeatureDiscovery + PlatformGraphService {
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

    fn graph_contains_node(
        &self,
        graph: &GraphState,
        node: NodeId,
    ) -> Result<bool, Vec<CompileError>>;

    fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>>;

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
}
