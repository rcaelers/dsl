use std::sync::Arc;

use logic_analyzer_graph_capabilities::node::GraphNodeCapabilityOverride;
use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use logic_analyzer_graph_compiler::{
    DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration, GraphLowerer,
    LiveCaptureOperationError, TimelineOperationError,
};
use logic_analyzer_graph_orchestration::{
    GraphWorkerClient, GraphWorkerFailure, GraphWorkerMessage,
};
use logic_analyzer_graph_plan::{
    CollectedOutputSubscription, CollectedTableSubscription, OutputSubscriptionPlan,
    ProcessingGraphError as CompileError, SamplingOverlayCandidate,
};
use logic_analyzer_graph_runtime::{
    ApplyError, ApplySummary, DerivedCacheClearStats, DerivedCacheClearTask,
    DerivedCacheEntrySnapshot, DerivedCacheError, GraphRunContext, GraphRuntime, GraphRuntimeError,
    LiveAnalysisSource, LiveRun, SourcePreparationExecutor, SourcePreparationStatus,
    SourcePreparationUpdate, SourceProcessOverrides, SourceReadinessRegistry,
};
use node_graph::{GraphState, NodeId};
use platform_runtime::WorkExecutor;
use signal_derived::{DerivedLanes, PersistentStoreConfig};
use signal_runtime::{AppManagerFactory, ConfigurationBoundary, DisconnectEvent};

use super::contract::{CachedDataLoader, GraphRun, GraphRunFailure};
use crate::live_capture::{CaptureAvailability, CaptureFeatureDiscovery};

/// UI-owned orchestration of document compilation and processing-graph execution.
pub(crate) struct UiGraphService {
    compiler: GraphLowerer,
    runtime: GraphRuntime,
    work_executor: Arc<dyn WorkExecutor>,
    graph_worker_client: Option<Arc<GraphWorkerClient>>,
}

fn present_runtime_errors(errors: Vec<GraphRuntimeError>) -> Vec<CompileError> {
    errors
        .into_iter()
        .map(|error| CompileError {
            node: error.node(),
            message: error.to_string(),
        })
        .collect()
}

impl UiGraphService {
    fn new(
        compiler: GraphLowerer,
        runtime: GraphRuntime,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            compiler,
            runtime,
            work_executor,
            graph_worker_client: None,
        }
    }

    fn lowerer(&self) -> &GraphLowerer {
        &self.compiler
    }

    pub(crate) fn start_revision_preparation(
        &self,
        revision: u64,
        graph: GraphState,
    ) -> Result<super::GraphRevisionPreparationTask, platform_runtime::WorkExecutorError> {
        let lowerer = self.compiler.clone();
        super::GraphRevisionPreparationTask::start(
            revision,
            Arc::clone(&self.work_executor),
            Box::new(move || lowerer.lower(&graph)),
        )
    }
}

impl GraphRun for LiveRun {
    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        LiveRun::persistent_cache_configs(self)
    }

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        LiveRun::sampling_overlays(self)
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

    fn pump_for(&mut self, budget: usize, max_duration: std::time::Duration) {
        LiveRun::pump_for(self, budget, max_duration);
    }

    fn wait(&mut self) {
        LiveRun::wait(self);
    }

    fn progress(&self) -> Vec<(NodeId, u64)> {
        LiveRun::progress(self)
    }

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        LiveRun::take_disconnected(self)
    }

    fn take_node_failures(&mut self) -> Vec<(Option<NodeId>, signal_runtime::NodeFailure)> {
        LiveRun::take_node_failures(self)
    }

    fn apply_processing_graph(
        &mut self,
        graph: logic_analyzer_graph_plan::ProcessingGraph,
        boundary: Option<ConfigurationBoundary>,
    ) -> Result<ApplySummary, ApplyError> {
        match boundary {
            Some(boundary) => self.apply_configuration_epoch_compiled(graph, boundary),
            None => self.apply_compiled(graph),
        }
    }
}

struct WorkerGraphRun {
    client: Arc<GraphWorkerClient>,
    sequence: u64,
    lanes: DerivedLanes,
    caches: Vec<PersistentStoreConfig>,
    sampling_overlays: Vec<SamplingOverlayCandidate>,
    output_subscriptions: Vec<CollectedOutputSubscription>,
    table_subscriptions: Vec<CollectedTableSubscription>,
    source_readiness: SourceReadinessRegistry,
    progress: Vec<(NodeId, u64)>,
    finished: bool,
    stopping: bool,
    needs_data_sync: bool,
    failure: Option<GraphRunFailure>,
}

impl WorkerGraphRun {
    fn start(
        service: &UiGraphService,
        graph: &GraphState,
        context: &mut GraphRunContext,
        client: Arc<GraphWorkerClient>,
    ) -> Result<Self, Vec<CompileError>> {
        let compiled = service.lowerer().lower(graph)?;
        let caches = service
            .runtime
            .derived_cache_configs_by_node(&compiled)
            .into_values()
            .flatten()
            .collect();
        let sampling_overlays = compiled.sampling_overlays.clone();
        context.set_sampling_overlays(sampling_overlays.clone());
        let sequence = client
            .start(
                graph.clone(),
                service.lowerer().output_subscriptions().clone(),
                context,
            )
            .map_err(|error| {
                vec![CompileError {
                    node: None,
                    message: error.to_string(),
                }]
            })?;
        Ok(Self {
            client,
            sequence,
            lanes: context.derived_lanes().clone(),
            caches,
            sampling_overlays,
            output_subscriptions: Vec::new(),
            table_subscriptions: Vec::new(),
            source_readiness: context.source_readiness().clone(),
            progress: Vec::new(),
            finished: false,
            stopping: false,
            needs_data_sync: false,
            failure: None,
        })
    }
}

impl GraphRun for WorkerGraphRun {
    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        self.caches.clone()
    }

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &self.sampling_overlays
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
        self.finished
    }

    fn is_stopping(&self) -> bool {
        self.stopping
    }

    fn stop(&mut self) {
        self.stopping = self.client.cancel(self.sequence);
    }

    fn pump(&mut self, _budget: usize) {
        for message in self.client.take_updates(self.sequence) {
            match message {
                GraphWorkerMessage::Started { .. } | GraphWorkerMessage::Artifacts { .. } => {}
                GraphWorkerMessage::Progress { nodes, .. } => self.progress = nodes,
                GraphWorkerMessage::Finished { .. } => {
                    self.finished = true;
                    self.needs_data_sync = true;
                }
                GraphWorkerMessage::Failed { error, .. } => {
                    self.finished = true;
                    self.failure = Some(graph_run_failure(error));
                }
                GraphWorkerMessage::Cancelled { .. } => {
                    self.finished = true;
                }
            }
        }
    }

    fn progress(&self) -> Vec<(NodeId, u64)> {
        self.progress.clone()
    }

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        Vec::new()
    }

    fn take_failure(&mut self) -> Option<GraphRunFailure> {
        self.failure.take()
    }

    fn synchronize_cached_data(
        &mut self,
        graph: &GraphState,
        load: &mut CachedDataLoader<'_>,
    ) -> Result<bool, Vec<CompileError>> {
        if !self.needs_data_sync {
            return Ok(false);
        }
        let mut context = GraphRunContext::default();
        context.set_derived_lanes(self.lanes.clone());
        let loaded = load(graph, &mut context)?;
        if loaded {
            self.output_subscriptions = context.collected_output_subscriptions().to_vec();
            self.table_subscriptions = context.collected_table_subscriptions().to_vec();
            self.sampling_overlays = context.take_sampling_overlays();
        }
        self.needs_data_sync = false;
        Ok(loaded)
    }
}

fn graph_run_failure(error: GraphWorkerFailure) -> GraphRunFailure {
    match error {
        GraphWorkerFailure::Busy => GraphRunFailure::Busy,
        GraphWorkerFailure::Compilation(message) => GraphRunFailure::Compilation(message),
        GraphWorkerFailure::Execution(message) => GraphRunFailure::Execution(message),
        GraphWorkerFailure::Node(message) => GraphRunFailure::Node(message),
        GraphWorkerFailure::Artifact(message) => GraphRunFailure::Artifact(message),
        GraphWorkerFailure::Cache(message) => GraphRunFailure::Cache(message),
        GraphWorkerFailure::Transport(error) => GraphRunFailure::Transport(error.to_string()),
    }
}

impl CaptureFeatureDiscovery for UiGraphService {
    fn discover_capture_availability(&self, graph: &GraphState) -> CaptureAvailability {
        match self.lowerer().discover_live_capture_feature(graph) {
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
                reason: error.to_string(),
            },
        }
    }
}

impl UiGraphService {
    pub(crate) fn set_artifact_repository(
        &mut self,
        repository: std::sync::Arc<dyn platform_artifacts::ArtifactRepository>,
    ) {
        self.runtime.set_artifact_repository(repository);
    }

    pub(crate) fn set_decoded_block_cache(
        &mut self,
        cache: signal_derived::DecodedBlockCacheHandle,
    ) {
        self.runtime.set_decoded_block_cache(cache);
    }

    pub(crate) fn set_graph_worker_client(&mut self, client: Option<Arc<GraphWorkerClient>>) {
        self.graph_worker_client = client;
    }

    pub(crate) fn derived_cache_configs_by_node(
        &self,
        graph: &GraphState,
    ) -> Result<std::collections::HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>>
    {
        let compiled = self.lowerer().lower(graph)?;
        Ok(self.runtime.derived_cache_configs_by_node(&compiled))
    }

    pub(crate) fn clear_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<DerivedCacheClearStats, DerivedCacheError> {
        self.runtime.clear_derived_cache_entry(config)
    }

    pub(crate) fn start_clear_derived_caches(
        &self,
    ) -> Result<DerivedCacheClearTask, DerivedCacheError> {
        self.runtime.start_clear_derived_caches()
    }

    pub(crate) fn inspect_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<DerivedCacheEntrySnapshot>, DerivedCacheError> {
        self.runtime.inspect_derived_cache_entry(config)
    }

    pub(crate) fn fingerprint_derived_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<[u8; 32]>, DerivedCacheError> {
        self.runtime.fingerprint_derived_cache_entry(config)
    }

    pub(crate) fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.compiler.set_output_subscriptions(subscriptions);
    }

    pub(crate) fn synchronize_prepared_capture(
        &mut self,
        graph: &GraphState,
    ) -> SourcePreparationUpdate {
        let discovered = self.lowerer().discover_capture_presentation(graph);
        self.runtime.synchronize_prepared_capture(discovered)
    }

    pub(crate) fn reset_prepared_capture(&mut self) {
        self.runtime.reset_prepared_capture();
    }

    pub(crate) fn source_preparation_status(&self) -> SourcePreparationStatus {
        self.runtime.source_preparation_status()
    }

    pub(crate) fn discover_live_capture_feature(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureOperationError> {
        self.lowerer().discover_live_capture_feature(graph)
    }

    pub(crate) fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureOperationError> {
        self.lowerer().discover_trigger_configuration(graph)
    }

    pub(crate) fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, LiveCaptureOperationError> {
        self.lowerer()
            .apply_live_capture_edit(graph, source_node, edit)
    }

    pub(crate) fn discover_timeline_markers(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, TimelineOperationError> {
        self.lowerer().discover_timeline_markers(graph)
    }

    pub(crate) fn apply_timeline_marker_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, TimelineOperationError> {
        self.lowerer()
            .apply_timeline_marker_edit(graph, owner_node, edit)
    }

    pub(crate) fn discover_timeline_marker_reference_bindings(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, TimelineOperationError> {
        self.lowerer()
            .discover_timeline_marker_reference_bindings(graph)
    }

    pub(crate) fn apply_timeline_marker_reference_binding_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, TimelineOperationError> {
        self.lowerer()
            .apply_timeline_marker_reference_binding_edit(graph, owner_node, edit)
    }

    pub(crate) fn graph_contains_node(
        &self,
        graph: &GraphState,
        node: NodeId,
    ) -> Result<bool, Vec<CompileError>> {
        self.lowerer().lower(graph).map(|compiled| {
            compiled
                .nodes
                .into_iter()
                .any(|compiled_node| compiled_node.id == node)
        })
    }

    pub(crate) fn load_prepared_cached_data(
        &self,
        compiled: logic_analyzer_graph_plan::ProcessingGraph,
        context: &mut GraphRunContext,
    ) -> Result<bool, Vec<CompileError>> {
        self.runtime
            .load_cached_data(compiled, context)
            .map_err(present_runtime_errors)
    }

    pub(crate) fn start_run(
        &self,
        graph: &GraphState,
        context: &mut GraphRunContext,
        source_overrides: SourceProcessOverrides,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        if source_overrides.is_empty()
            && let Some(client) = self.graph_worker_client.clone()
        {
            return WorkerGraphRun::start(self, graph, context, client)
                .map(|run| Box::new(run) as Box<dyn GraphRun>);
        }
        let compiled = self.lowerer().lower(graph)?;
        self.runtime
            .start(compiled, context, source_overrides)
            .map(|run| Box::new(run) as Box<dyn GraphRun>)
            .map_err(present_runtime_errors)
    }

    pub(crate) fn start_live_analysis(
        &self,
        graph: &GraphState,
        context: &mut GraphRunContext,
        source: LiveAnalysisSource,
    ) -> Result<Box<dyn GraphRun>, Vec<CompileError>> {
        let compiled = self.lowerer().lower(graph)?;
        self.runtime
            .start_live_analysis(compiled, context, source)
            .map(|run| Box::new(run) as Box<dyn GraphRun>)
            .map_err(present_runtime_errors)
    }

    pub(crate) fn apply_configuration_epoch(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        let compiled = self.lowerer().lower(graph).map_err(ApplyError::Compile)?;
        run.apply_processing_graph(compiled, Some(boundary))
    }

    pub(crate) fn synchronize_run_data(
        &self,
        run: &mut dyn GraphRun,
        graph: &GraphState,
    ) -> Result<bool, Vec<CompileError>> {
        run.synchronize_cached_data(graph, &mut |graph, context| {
            let compiled = self.lowerer().lower(graph)?;
            self.runtime
                .load_cached_data(compiled, context)
                .map_err(present_runtime_errors)
        })
    }
}

pub(crate) fn standard_graph_service() -> UiGraphService {
    UiGraphService::new(
        GraphLowerer::new(),
        GraphRuntime::new(),
        Arc::new(platform_runtime::InlineWorkExecutor),
    )
}

pub(crate) fn graph_service_with_execution(
    source_preparation_executor: Box<dyn SourcePreparationExecutor>,
    runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
    work_executor: std::sync::Arc<dyn WorkExecutor>,
) -> UiGraphService {
    graph_service_with_execution_and_capability_overrides(
        source_preparation_executor,
        runtime_factory,
        work_executor,
        Vec::new(),
    )
}

pub(crate) fn graph_service_with_execution_and_capability_overrides(
    source_preparation_executor: Box<dyn SourcePreparationExecutor>,
    runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
    work_executor: std::sync::Arc<dyn WorkExecutor>,
    capability_overrides: Vec<GraphNodeCapabilityOverride>,
) -> UiGraphService {
    UiGraphService::new(
        GraphLowerer::with_capability_overrides(capability_overrides),
        GraphRuntime::with_execution(
            source_preparation_executor,
            runtime_factory,
            Arc::clone(&work_executor),
        ),
        work_executor,
    )
}
