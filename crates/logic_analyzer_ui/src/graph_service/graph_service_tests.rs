use std::sync::{Arc, Mutex};

use logic_analyzer_graph_capabilities::node_support::LiveCaptureEdit;
use logic_analyzer_graph_compiler::{
    DiscoveredLiveCaptureFeature, DiscoveredTriggerConfiguration, LiveCaptureDiscoveryError,
};
use logic_analyzer_graph_plan::{
    CollectedOutputSubscription, CollectedTableSubscription, OutputSubscriptionPlan,
    ProcessingGraphError as CompileError, SamplingOverlayCandidate,
};
use logic_analyzer_graph_runtime::{
    ApplyError, ApplySummary, DerivedCacheClearStats, DerivedCacheEntrySnapshot, GraphRunContext,
    LiveAnalysisSource, SourcePreparationStatus, SourcePreparationUpdate, SourceProcessOverrides,
    SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use platform_artifacts::ArtifactRepository;
use signal_derived::PersistentStoreConfig;
use signal_runtime::{ConfigurationBoundary, DisconnectEvent};

use super::contract::{GraphRun, GraphService};
use crate::live_capture::{CaptureAvailability, CaptureFeatureDiscovery, capture_availability};

struct FakeGraphService {
    subscriptions: Arc<Mutex<Vec<(NodeId, usize)>>>,
    contains_node: bool,
}

#[derive(Default)]
struct FakeGraphRun {
    output_subscriptions: Vec<CollectedOutputSubscription>,
    table_subscriptions: Vec<CollectedTableSubscription>,
    source_readiness: SourceReadinessRegistry,
    stopping: bool,
}

impl GraphRun for FakeGraphRun {
    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        Vec::new()
    }

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &[]
    }

    fn output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.output_subscriptions
    }

    fn table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.table_subscriptions
    }

    fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn is_stopping(&self) -> bool {
        self.stopping
    }

    fn stop(&mut self) {
        self.stopping = true;
    }

    fn pump(&mut self, _budget: usize) {}

    fn progress(&self) -> Vec<(NodeId, u64)> {
        vec![(NodeId(11), 23)]
    }

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        Vec::new()
    }
}

impl CaptureFeatureDiscovery for FakeGraphService {
    fn discover_capture_availability(&self, _graph: &GraphState) -> CaptureAvailability {
        CaptureAvailability::Unavailable {
            reason: "fake graph has no capture source".into(),
        }
    }
}

impl GraphService for FakeGraphService {
    fn set_artifact_repository(&mut self, _repository: Arc<dyn ArtifactRepository>) {}

    fn derived_cache_configs_by_node(
        &self,
        _graph: &GraphState,
    ) -> Result<std::collections::HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>>
    {
        Ok(std::collections::HashMap::new())
    }

    fn clear_derived_cache_entry(
        &self,
        _config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, String> {
        Ok(DerivedCacheClearStats::default())
    }

    fn start_clear_derived_caches(
        &self,
    ) -> Result<logic_analyzer_graph_runtime::DerivedCacheClearTask, String> {
        logic_analyzer_graph_runtime::GraphRuntime::new().start_clear_derived_caches()
    }

    fn inspect_derived_cache_entry(
        &self,
        _config: &PersistentStoreConfig,
    ) -> Result<Option<DerivedCacheEntrySnapshot>, String> {
        Ok(None)
    }

    fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        *self.subscriptions.lock().unwrap() = subscriptions.outputs().collect();
    }

    fn synchronize_prepared_capture(&mut self, _graph: &GraphState) -> SourcePreparationUpdate {
        SourcePreparationUpdate::Unchanged
    }

    fn reset_prepared_capture(&mut self) {}

    fn source_preparation_status(&self) -> SourcePreparationStatus {
        SourcePreparationStatus::Ready
    }

    fn discover_live_capture_feature(
        &self,
        _graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
        Ok(None)
    }

    fn discover_trigger_configuration(
        &self,
        _graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
        Ok(None)
    }

    fn apply_live_capture_edit(
        &self,
        _graph: &GraphState,
        _source_node: NodeId,
        _edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    fn graph_contains_node(
        &self,
        _graph: &GraphState,
        _node: NodeId,
    ) -> Result<bool, Vec<CompileError>> {
        Ok(self.contains_node)
    }

    fn sampling_overlay_candidates(
        &self,
        _graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
        Ok(Vec::new())
    }

    fn load_cached_data(
        &self,
        _graph: &GraphState,
        _context: &mut GraphRunContext,
    ) -> Result<bool, Vec<CompileError>> {
        Ok(false)
    }

    fn start_run(
        &self,
        _graph: &GraphState,
        _context: &mut GraphRunContext,
        _source_overrides: SourceProcessOverrides,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        Ok(Box::new(FakeGraphRun::default()))
    }

    fn start_live_analysis(
        &self,
        _graph: &GraphState,
        _context: &mut GraphRunContext,
        _source: LiveAnalysisSource,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        Ok(Box::new(FakeGraphRun::default()))
    }

    fn apply_run(
        &self,
        _run: &mut dyn GraphRun,
        _graph: &GraphState,
    ) -> Result<ApplySummary, ApplyError> {
        Ok(ApplySummary::default())
    }

    fn apply_configuration_epoch(
        &self,
        _run: &mut dyn GraphRun,
        _graph: &GraphState,
        _boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        Ok(ApplySummary::default())
    }
}

#[test]
fn ui_graph_orchestration_accepts_a_local_service_fake() {
    let subscriptions = Arc::new(Mutex::new(Vec::new()));
    let mut service: Box<dyn GraphService> = Box::new(FakeGraphService {
        subscriptions: subscriptions.clone(),
        contains_node: true,
    });
    let plan: OutputSubscriptionPlan = [(NodeId(3), 2), (NodeId(7), 1)].into_iter().collect();

    service.set_output_subscriptions(plan);

    assert_eq!(
        *subscriptions.lock().unwrap(),
        vec![(NodeId(3), 2), (NodeId(7), 1)]
    );
    assert!(
        service
            .graph_contains_node(&GraphState::default(), NodeId(3))
            .unwrap()
    );
    assert!(matches!(
        service.source_preparation_status(),
        SourcePreparationStatus::Ready
    ));
    let mut run = service
        .start_run(
            &GraphState::default(),
            &mut GraphRunContext::default(),
            SourceProcessOverrides::new(),
        )
        .unwrap();
    assert_eq!(run.progress(), vec![(NodeId(11), 23)]);
    run.stop();
    assert!(run.is_stopping());
}

#[test]
fn capture_discovery_uses_the_same_substitutable_graph_service() {
    let service: Box<dyn GraphService> = Box::new(FakeGraphService {
        subscriptions: Arc::new(Mutex::new(Vec::new())),
        contains_node: false,
    });

    let availability = capture_availability(&GraphState::default(), service.as_ref(), None);

    assert_eq!(
        availability.reason(),
        Some("fake graph has no capture source")
    );
}
