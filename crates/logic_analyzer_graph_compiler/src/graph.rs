//! Graph → Pipeline compiler (`docs/APP_DESIGN.md`).
//!
//! Two stages: `lower()` turns the UI graph into a pure, diffable
//! `CompiledGraph` IR (prune to sink-reachable nodes, follow reroutes,
//! validate, negotiate per-edge stream kinds); `start_live()` materializes
//! it into a running [`LiveRun`], the supervisor-driven live path used
//! by both the app and its own tests — nothing builds an offline `Pipeline`
//! from this IR anymore; that's what `examples/*.rs` do directly against
//! `signal_processing::Pipeline` for headless/scripted captures.
//!
//! Kind negotiation: each edge picks `offered ∩ accepted`, producer
//! preference order winning. That is what maps one UI `Signal` socket onto
//! the source's dual `d{i}`/`b{i}` ports; every `Words` socket carries the
//! same `Word` runtime type regardless of which decoder produced it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::{
    CaptureGraphSourceFactory, LiveCaptureFeature, RuntimeBuilder, RuntimeBuilderOverride,
};
use logic_analyzer_graph_api::node_support::{
    CaptureCacheIdentity, CapturePresentation, DefaultLanePresentationDescriptor, LiveCaptureEdit,
    NodeBuildContext, PortKind, ResolvedInput, ResolvedInputs, SimpleTriggerChannel,
    TimelineMarkerDescriptor, TimelineMarkerEdit, TimelineMarkerReference,
    TimelineMarkerReferenceBindingDescriptor, TimelineMarkerReferenceBindingEdit,
    TriggerConfigurationFeature,
};
use node_graph::api::{
    Connection, GraphState, Node, NodeId, NodeKind, Socket, SocketDirection, SocketId, SocketShape,
    VariadicInfo,
};
#[cfg(test)]
use signal_processing::PayloadRegistrationError;
use signal_processing::{
    AcquisitionContext, AcquisitionResult, AppManager, ArtifactRepository, CaptureChannelId,
    CaptureProviderCapabilities, CaptureSessionPlan, CaptureStartMode, CollectedLaneRequest,
    ConfigurationBoundary, DerivedDataRetention, DerivedLanes, DisconnectEvent, InlineWorkExecutor,
    InputSub, MemoryArtifactRepository, NodeConfig, NodeFailure, OverflowPolicy, PayloadRegistry,
    PersistentStoreConfig, PreparedAcquisition, ProcessNode, SampleBlock, SamplingPointStore,
    SimpleTriggerCondition, TriggerProgram, WorkExecutor,
};

use super::data_collector::DataCollectorBuilder;
use super::errors::{ApplyError, CompileError};
use super::{
    CollectedOutputLane, CollectedOutputSubscription, CollectedTableSubscription,
    OutputSubscriptionPlan, RunData, RunDiagnosticRegistry, SourceArtifactReadiness,
    SourceDataKind, SourceReadiness, SourceReadinessRegistry, cache_policy,
};

/// Shared resources handed to builders. A fresh `DerivedLanes` store per
/// run makes stale collected data vanish atomically on re-run.
pub struct CompileCtx {
    derived_lanes: DerivedLanes,
    /// Storage policy selected by the graph's source. Finite sources retain
    /// their complete timeline; continuous sources can explicitly choose a
    /// bounded rolling window.
    derived_data_retention: DerivedDataRetention,
    derived_word_caches: Vec<Option<PersistentStoreConfig>>,
    timeline_markers: HashMap<TimelineMarkerReference, signal_processing::TimelineMarker>,
    /// Clocked-node sampling overlays resolved during lowering. The host
    /// application independently chooses which candidates to display.
    sampling_overlays: Vec<SamplingOverlayCandidate>,
    sampling_points: HashMap<String, SamplingPointStore>,
    collected_output_subscriptions: Vec<CollectedOutputSubscription>,
    collected_table_subscriptions: Vec<CollectedTableSubscription>,
    diagnostics: RunDiagnosticRegistry,
    source_readiness: SourceReadinessRegistry,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

impl Default for CompileCtx {
    fn default() -> Self {
        Self {
            derived_lanes: DerivedLanes::default(),
            derived_data_retention: DerivedDataRetention::default(),
            derived_word_caches: Vec::new(),
            timeline_markers: HashMap::new(),
            sampling_overlays: Vec::new(),
            sampling_points: HashMap::new(),
            collected_output_subscriptions: Vec::new(),
            collected_table_subscriptions: Vec::new(),
            diagnostics: RunDiagnosticRegistry::default(),
            source_readiness: SourceReadinessRegistry::default(),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }
}

impl CompileCtx {
    /// Supplies the host-selected bounded work executor to node builders.
    pub fn set_work_executor(&mut self, executor: Arc<dyn WorkExecutor>) {
        self.work_executor = executor;
    }

    /// Supplies the host-selected repository used by concrete data stores.
    pub fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.artifact_repository = repository;
    }

    /// Returns the run's collected lanes for binding to host views and panels.
    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    /// Takes the sampling-overlay candidates discovered for this run.
    pub fn take_sampling_overlays(&mut self) -> Vec<SamplingOverlayCandidate> {
        std::mem::take(&mut self.sampling_overlays)
    }

    /// Supplies a shared lane catalog that a deferred execution host can populate later.
    pub fn set_derived_lanes(&mut self, lanes: DerivedLanes) {
        self.derived_lanes = lanes;
    }

    /// Supplies statically lowered overlay metadata before deferred execution starts.
    pub fn set_sampling_overlays(&mut self, overlays: Vec<SamplingOverlayCandidate>) {
        self.sampling_overlays = overlays;
    }

    /// Returns application-requested retained outputs and their resolved lane metadata.
    pub fn collected_output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.collected_output_subscriptions
    }

    pub fn collected_table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.collected_table_subscriptions
    }

    /// Returns all application-neutral data and readiness handles for this run.
    pub fn run_data(&self) -> RunData {
        RunData::new(
            self.derived_lanes.clone(),
            self.collected_output_subscriptions.clone(),
            self.collected_table_subscriptions.clone(),
            self.sampling_overlays.clone(),
            self.diagnostics.clone(),
            self.source_readiness.clone(),
        )
    }

    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }

    /// Supplies one host-owned timeline position to nodes materialized for
    /// this run. Values are snapshots; changing the host position takes
    /// effect on the next run.
    pub fn set_timeline_marker(
        &mut self,
        reference: TimelineMarkerReference,
        marker: signal_processing::TimelineMarker,
    ) {
        self.timeline_markers.insert(reference, marker);
    }

    pub(crate) fn timeline_markers(
        &self,
    ) -> impl Iterator<Item = (&TimelineMarkerReference, &signal_processing::TimelineMarker)> {
        self.timeline_markers.iter()
    }
}

impl NodeBuildContext for CompileCtx {
    fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    fn derived_data_retention(&self) -> DerivedDataRetention {
        self.derived_data_retention
    }

    fn derived_word_cache(&self, member: usize) -> Option<&PersistentStoreConfig> {
        self.derived_word_caches
            .get(member)
            .and_then(Option::as_ref)
    }

    fn sampling_points(&self, runtime_name: &str) -> Option<SamplingPointStore> {
        self.sampling_points.get(runtime_name).cloned()
    }

    fn work_executor(&self) -> Arc<dyn WorkExecutor> {
        Arc::clone(&self.work_executor)
    }

    fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.artifact_repository)
    }

    fn timeline_marker(
        &self,
        reference: TimelineMarkerReference,
    ) -> Option<signal_processing::TimelineMarker> {
        self.timeline_markers.get(&reference).copied()
    }
}

/// A fully resolved, selectable sampling overlay belonging to one graph node.
#[derive(Debug, Clone)]
pub struct SamplingOverlayCandidate {
    node_id: NodeId,
    node_title: String,
    overlay: ResolvedSamplingOverlay,
    cache_key: Option<[u8; 32]>,
    retained_word_lane: Option<RetainedWordSamplingLane>,
}

#[derive(Debug, Clone)]
struct RetainedWordSamplingLane {
    name: String,
    clock_high: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedSamplingOverlay {
    pub clock_channel: usize,
    pub sampled_channels: Vec<usize>,
    pub points: SamplingPointStore,
}

impl SamplingOverlayCandidate {
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn node_title(&self) -> &str {
        &self.node_title
    }

    pub fn overlay(&self) -> &ResolvedSamplingOverlay {
        &self.overlay
    }

    pub(crate) fn cache_key(&self) -> Option<[u8; 32]> {
        self.cache_key
    }

    pub(crate) fn set_cache_key(&mut self, cache_key: Option<[u8; 32]>) {
        self.cache_key = cache_key;
    }

    pub(crate) fn set_points(&mut self, points: SamplingPointStore) {
        self.overlay.points = points;
    }

    pub(crate) fn install_retained_word_provider(&mut self, lanes: DerivedLanes) -> bool {
        let Some(source) = &self.retained_word_lane else {
            return false;
        };
        self.overlay.points.set_retained_word_provider(
            lanes,
            &source.name,
            source.clock_high,
            self.overlay.sampled_channels.len(),
        );
        true
    }

    pub(crate) fn uses_retained_word_lane(&self) -> bool {
        self.retained_word_lane.is_some()
    }
}

// ── Builder trait & registry ─────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTriggerConfiguration {
    pub source_node: NodeId,
    pub source_title: String,
    pub feature: TriggerConfigurationFeature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTimelineMarker {
    pub owner_node: NodeId,
    pub owner_title: String,
    pub marker: TimelineMarkerDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTimelineMarkerReferenceBinding {
    pub owner_node: NodeId,
    pub owner_title: String,
    pub binding: TimelineMarkerReferenceBindingDescriptor,
}

pub struct DiscoveredLiveCaptureFeature {
    source_node: NodeId,
    source_title: String,
    visible_channels: Vec<usize>,
    feature: Box<dyn LiveCaptureFeature>,
}

impl DiscoveredLiveCaptureFeature {
    pub fn new(
        source_node: NodeId,
        source_title: impl Into<String>,
        feature: Box<dyn LiveCaptureFeature>,
    ) -> Self {
        let visible_channels = (0..feature.channels().len()).collect();
        Self::new_with_visible_channels(source_node, source_title, visible_channels, feature)
    }

    fn new_with_visible_channels(
        source_node: NodeId,
        source_title: impl Into<String>,
        visible_channels: Vec<usize>,
        feature: Box<dyn LiveCaptureFeature>,
    ) -> Self {
        Self {
            source_node,
            source_title: source_title.into(),
            visible_channels,
            feature,
        }
    }

    pub fn channels(&self) -> &[CaptureChannelId] {
        self.feature.channels()
    }

    pub fn source_node(&self) -> NodeId {
        self.source_node
    }

    pub fn source_title(&self) -> &str {
        &self.source_title
    }

    pub fn channel_names(&self) -> &[String] {
        self.feature.channel_names()
    }

    pub fn visible_channels(&self) -> &[usize] {
        &self.visible_channels
    }

    pub fn sample_rate_hz(&self) -> f64 {
        self.feature.sample_rate_hz()
    }

    pub fn capabilities(&self) -> &CaptureProviderCapabilities {
        self.feature.capabilities()
    }

    pub fn simple_trigger_channels(&self) -> &[SimpleTriggerChannel] {
        self.feature.simple_trigger_channels()
    }

    pub fn trigger_program(&self) -> Option<&TriggerProgram> {
        self.feature.trigger_program()
    }

    pub fn has_trigger_program(&self) -> bool {
        self.trigger_program().is_some() || self.has_simple_trigger()
    }

    pub fn session_plan(&self) -> Option<&CaptureSessionPlan> {
        self.feature.session_plan()
    }

    pub fn has_simple_trigger(&self) -> bool {
        self.simple_trigger_channels()
            .iter()
            .any(|channel| channel.enabled && channel.condition != SimpleTriggerCondition::Ignore)
    }

    pub fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory> {
        self.feature.graph_source_factory()
    }

    pub fn prepare(
        self,
        context: AcquisitionContext,
        mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        self.feature.prepare_with_mode(context, mode)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCaptureDiscoveryError {
    pub source_nodes: Vec<NodeId>,
    pub message: String,
}

pub struct DiscoveredCapturePresentation {
    pub identity: String,
    pub visible_channels: Vec<usize>,
    pub presentation: CapturePresentation,
}

pub(crate) struct BuilderRegistry {
    pub(crate) builders: HashMap<String, Box<dyn RuntimeBuilder>>,
    pub(crate) payloads: PayloadRegistry,
    pub(crate) payload_subscriptions: Vec<PayloadSubscription>,
}

#[derive(Clone)]
pub(crate) struct PayloadSubscription {
    pub(crate) kind: PortKind,
    pub(crate) diagnostic_name: String,
    pub(crate) presentation: DefaultLanePresentationDescriptor,
    pub(crate) persistent_cache: bool,
    pub(crate) configure_request: PayloadRequestConfigurator,
}

pub(crate) type PayloadRequestConfigurator = Arc<
    dyn Fn(
            CollectedLaneRequest,
            usize,
            &ResolvedInput,
            &dyn NodeBuildContext,
        ) -> CollectedLaneRequest
        + Send
        + Sync,
>;

impl BuilderRegistry {
    pub(crate) fn standard() -> Self {
        Self::standard_with_overrides(Vec::new())
    }

    pub(crate) fn standard_with_overrides(overrides: Vec<RuntimeBuilderOverride>) -> Self {
        let registry = Self::with_builders(super::standard_graph_node_builders(overrides));
        super::validate_graph_node_payload_requirements(&registry.payloads);
        registry
    }

    fn with_builders(builders: HashMap<String, Box<dyn RuntimeBuilder>>) -> Self {
        let mut registry = Self {
            builders,
            payloads: PayloadRegistry::new(),
            payload_subscriptions: Vec::new(),
        };
        for registration in super::payload_registrations() {
            super::payload_registration::apply_payload_registration(registration, &mut registry)
                .expect("payload inventory registration must be valid");
        }
        registry
    }

    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn isolated_test() -> Self {
        Self {
            builders: HashMap::new(),
            payloads: PayloadRegistry::new(),
            payload_subscriptions: Vec::new(),
        }
    }

    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn insert_test_builder(
        &mut self,
        name: impl Into<String>,
        builder: Box<dyn RuntimeBuilder>,
    ) {
        self.builders.insert(name.into(), builder);
    }

    /// Registers a payload that has explicit retained-data semantics.
    ///
    /// This also registers the payload with the generic runtime channel
    /// factory. The present registry records only the durable identity; a
    /// later adapter registration supplies its typed ingestion and query
    /// behavior.
    #[cfg(test)]
    pub(crate) fn register_payload<T: logic_analyzer_graph_api::node_support::PortValue>(
        &mut self,
        stable_id: impl Into<String>,
    ) -> Result<&mut Self, PayloadRegistrationError> {
        signal_processing::register_type::<T>();
        self.payloads.register::<T>(stable_id)?;
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn register_payload_subscription_with_request_configurator<
        T: logic_analyzer_graph_api::node_support::PortValue,
    >(
        &mut self,
        presentation: DefaultLanePresentationDescriptor,
        configure_request: PayloadRequestConfigurator,
        persistent_cache: bool,
    ) -> Result<&mut Self, PayloadRegistrationError> {
        let type_id = std::any::TypeId::of::<T>();
        let descriptor = self
            .payloads
            .descriptor_by_type_id(type_id)
            .ok_or_else(|| PayloadRegistrationError::PayloadNotRegistered {
                type_name: std::any::type_name::<T>().to_owned(),
            })?;
        if self.payloads.adapter_by_type_id(type_id).is_none() {
            return Err(PayloadRegistrationError::PayloadHasNoAdapter {
                stable_id: descriptor.stable_id().to_owned(),
            });
        }
        let kind = PortKind::of::<T>();
        if let Some(existing) = self
            .payload_subscriptions
            .iter_mut()
            .find(|existing| existing.kind == kind)
        {
            existing.presentation = presentation;
            existing.diagnostic_name = T::kind_name().to_owned();
            existing.persistent_cache = persistent_cache;
            existing.configure_request = configure_request;
        } else {
            self.payload_subscriptions.push(PayloadSubscription {
                kind,
                diagnostic_name: T::kind_name().to_owned(),
                presentation,
                persistent_cache,
                configure_request,
            });
        }
        Ok(self)
    }

    /// Registered retained-payload identities, keyed by runtime `TypeId` and
    /// durable plugin-owned identifiers.
    pub(crate) fn payloads(&self) -> &PayloadRegistry {
        &self.payloads
    }

    pub(crate) fn subscribable_payload_kinds(&self) -> Vec<PortKind> {
        self.payload_subscriptions
            .iter()
            .map(|payload| payload.kind)
            .collect()
    }

    fn payload_subscription_presentation(
        &self,
        kind: PortKind,
    ) -> Option<DefaultLanePresentationDescriptor> {
        self.payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .map(|payload| payload.presentation.clone())
    }

    pub(crate) fn payload_uses_persistent_cache(&self, kind: PortKind) -> bool {
        self.payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .is_some_and(|payload| payload.persistent_cache)
    }

    pub(crate) fn configure_collected_lane_request(
        &self,
        kind: PortKind,
        request: CollectedLaneRequest,
        member: usize,
        input: &ResolvedInput,
        ctx: &dyn NodeBuildContext,
    ) -> Result<(CollectedLaneRequest, &str), String> {
        let contract = self
            .payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .ok_or_else(|| format!("payload {kind:?} has no data-subscription contract"))?;
        Ok((
            (contract.configure_request)(request, member, input, ctx),
            &contract.diagnostic_name,
        ))
    }

    pub(crate) fn get(&self, def_name: &str) -> Option<&dyn RuntimeBuilder> {
        self.builders.get(def_name).map(|b| b.as_ref())
    }
}

fn capture_channel_selection(
    subscriptions: &OutputSubscriptionPlan,
    node_id: NodeId,
    node: &Node,
    builder: &dyn RuntimeBuilder,
) -> Vec<usize> {
    subscriptions
        .outputs()
        .filter(|(selected_node, _)| *selected_node == node_id)
        .filter_map(|(_, output)| {
            node.outputs
                .get(output)
                .and_then(|output| builder.viewer_channel_origin(output, &node.state))
        })
        .collect()
}

fn publish_materialized_source_readiness(
    compiled: &CompiledGraph,
    registry: &BuilderRegistry,
    readiness: &SourceReadinessRegistry,
) {
    for node in &compiled.nodes {
        let Some(lifecycle) = registry
            .get(&node.builder)
            .and_then(RuntimeBuilder::source_data_lifecycle)
        else {
            continue;
        };
        readiness.publish(SourceReadiness {
            source: node.id,
            kind: match lifecycle.kind {
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::File => {
                    SourceDataKind::File
                }
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::Live => {
                    SourceDataKind::Live
                }
            },
            preload: if lifecycle.preload {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            cache: if lifecycle.cache {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            index: if lifecycle.index {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            data: match lifecycle.kind {
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::File => {
                    SourceArtifactReadiness::Pending
                }
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::Live => {
                    SourceArtifactReadiness::Available
                }
            },
        });
    }
}

/// Discovers a concrete source's pre-run presentation through its builder contract.
pub(crate) fn discover_capture_presentation_with_subscriptions(
    graph: &GraphState,
    builders: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<Option<DiscoveredCapturePresentation>, String> {
    let mut candidates = Vec::new();
    for (&node_id, node) in &graph.nodes {
        if node.kind != NodeKind::Regular || node.muted {
            continue;
        }
        let Some(builder) = builders.get(node.def_name()) else {
            continue;
        };
        let Some(presentation) = builder.capture_presentation(&node.state)? else {
            continue;
        };
        let visible_channels = capture_channel_selection(subscriptions, node_id, node, builder);
        let identity_state = (&node.state, &visible_channels);
        let state = serde_json::to_vec(&identity_state).map_err(|error| error.to_string())?;
        candidates.push(DiscoveredCapturePresentation {
            identity: format!("{node_id:?}:{}", blake3::hash(&state).to_hex()),
            visible_channels,
            presentation,
        });
    }
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        count => Err(format!(
            "the graph has {count} enabled sources with pre-run capture presentations"
        )),
    }
}

#[cfg(test)]
fn discover_capture_presentation(
    graph: &GraphState,
    builders: &BuilderRegistry,
) -> Result<Option<DiscoveredCapturePresentation>, String> {
    let subscriptions = test_output_subscriptions(graph, builders);
    discover_capture_presentation_with_subscriptions(graph, builders, &subscriptions)
}

/// Resolves exactly one enabled live-capture feature without identifying a
/// concrete node type. Muted nodes do not participate in acquisition.
pub(crate) fn discover_live_capture_feature_with_subscriptions(
    graph: &GraphState,
    builders: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
    discover_live_capture_feature_from(graph, builders, subscriptions, |_| true)
}

#[cfg(test)]
fn discover_live_capture_feature(
    graph: &GraphState,
    builders: &BuilderRegistry,
) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
    let subscriptions = test_output_subscriptions(graph, builders);
    discover_live_capture_feature_with_subscriptions(graph, builders, &subscriptions)
}

/// Resolves exactly one enabled trigger-configuration feature without consulting acquisition
/// backends or identifying a concrete node type.
pub(crate) fn discover_trigger_configuration(
    graph: &GraphState,
    builders: &BuilderRegistry,
) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
    let mut candidates = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(builder) = builders.get(node.def_name()) else {
            continue;
        };
        match builder.trigger_configuration(&node.state) {
            Ok(Some(feature)) => candidates.push(DiscoveredTriggerConfiguration {
                source_node: node.id,
                source_title: node.title.clone(),
                feature,
            }),
            Ok(None) => {}
            Err(message) => {
                return Err(LiveCaptureDiscoveryError {
                    source_nodes: vec![node.id],
                    message: format!("{}: {message}", node.title),
                });
            }
        }
    }
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => Err(LiveCaptureDiscoveryError {
            source_nodes: candidates
                .iter()
                .map(|candidate| candidate.source_node)
                .collect(),
            message: "multiple enabled trigger configurations are present; keep one capture source enabled"
                .into(),
        }),
    }
}

/// Discovers every enabled marker through node-owned, protocol-neutral contracts.
pub(crate) fn discover_timeline_markers(
    graph: &GraphState,
    builders: &BuilderRegistry,
) -> Result<Vec<DiscoveredTimelineMarker>, String> {
    let mut discovered = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(builder) = builders.get(node.def_name()) else {
            continue;
        };
        let markers = builder
            .timeline_markers(&node.state)
            .map_err(|message| format!("{}: {message}", node.title))?;
        discovered.extend(markers.into_iter().map(|marker| DiscoveredTimelineMarker {
            owner_node: node.id,
            owner_title: node.title.clone(),
            marker,
        }));
    }
    discovered.sort_by(|left, right| {
        (left.owner_node.0, left.marker.id.as_str())
            .cmp(&(right.owner_node.0, right.marker.id.as_str()))
    });
    Ok(discovered)
}

/// Routes a marker edit to the concrete builder that owns it.
pub(crate) fn apply_timeline_marker_edit(
    graph: &GraphState,
    builders: &BuilderRegistry,
    owner_node: NodeId,
    edit: &TimelineMarkerEdit,
) -> Result<Value, String> {
    let node = graph
        .nodes
        .get(&owner_node)
        .ok_or_else(|| format!("timeline-marker node {owner_node:?} no longer exists"))?;
    let builder = builders
        .get(node.def_name())
        .ok_or_else(|| format!("no runtime builder is registered for {}", node.def_name()))?;
    builder
        .apply_timeline_marker_edit(&node.state, edit)?
        .ok_or_else(|| format!("{} does not support this timeline-marker edit", node.title))
}

/// Discovers controls which select a host-owned timeline position.
pub(crate) fn discover_timeline_marker_reference_bindings(
    graph: &GraphState,
    builders: &BuilderRegistry,
) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, String> {
    let mut discovered = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(builder) = builders.get(node.def_name()) else {
            continue;
        };
        let bindings = builder
            .timeline_marker_reference_bindings(&node.state)
            .map_err(|message| format!("{}: {message}", node.title))?;
        discovered.extend(bindings.into_iter().map(|binding| {
            DiscoveredTimelineMarkerReferenceBinding {
                owner_node: node.id,
                owner_title: node.title.clone(),
                binding,
            }
        }));
    }
    discovered.sort_by(|left, right| {
        (left.owner_node.0, left.binding.id.as_str())
            .cmp(&(right.owner_node.0, right.binding.id.as_str()))
    });
    Ok(discovered)
}

/// Routes a host-owned timeline-reference update to its concrete node.
pub(crate) fn apply_timeline_marker_reference_binding_edit(
    graph: &GraphState,
    builders: &BuilderRegistry,
    owner_node: NodeId,
    edit: &TimelineMarkerReferenceBindingEdit,
) -> Result<Value, String> {
    let node = graph
        .nodes
        .get(&owner_node)
        .ok_or_else(|| format!("timeline-reference node {owner_node:?} no longer exists"))?;
    let builder = builders
        .get(node.def_name())
        .ok_or_else(|| format!("no runtime builder is registered for {}", node.def_name()))?;
    builder
        .apply_timeline_marker_reference_binding_edit(&node.state, edit)?
        .ok_or_else(|| {
            format!(
                "{} does not support this timeline-reference edit",
                node.title
            )
        })
}

/// Routes a portable live-feature edit to the concrete builder that owns `source_node`.
pub(crate) fn apply_live_capture_edit(
    graph: &GraphState,
    builders: &BuilderRegistry,
    source_node: NodeId,
    edit: &LiveCaptureEdit,
) -> Result<Value, String> {
    let node = graph
        .nodes
        .get(&source_node)
        .ok_or_else(|| format!("live capture source {source_node:?} no longer exists"))?;
    let builder = builders
        .get(node.def_name())
        .ok_or_else(|| format!("no runtime builder is registered for {}", node.def_name()))?;
    builder
        .apply_live_capture_edit(&node.state, edit)?
        .ok_or_else(|| format!("{} does not support this live capture edit", node.title))
}

/// Resolves a live feature only from nodes retained by a successfully
/// compiled graph. This prevents a disconnected development or hardware node
/// from becoming the acquisition source for a different active time domain.
fn discover_live_capture_feature_from(
    graph: &GraphState,
    builders: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    include: impl Fn(&Node) -> bool,
) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
    let mut candidates = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted && include(node))
    {
        let Some(builder) = builders.get(node.def_name()) else {
            continue;
        };
        match builder.live_capture_feature(&node.state) {
            Ok(Some(feature)) => {
                let trigger_channels = feature.simple_trigger_channels();
                let trigger_ids: HashSet<_> = trigger_channels
                    .iter()
                    .map(|channel| &channel.channel_id)
                    .collect();
                let trigger_viewer_channels: HashSet<_> = trigger_channels
                    .iter()
                    .map(|channel| channel.viewer_channel)
                    .collect();
                let duplicate_trigger_channels = trigger_ids.len() != trigger_channels.len()
                    || trigger_viewer_channels.len() != trigger_channels.len();
                let invalid = if feature.channels().is_empty() {
                    Some("live capture exposes no channels")
                } else if feature.channel_names().len() != feature.channels().len() {
                    Some("live capture channel names do not match its channel table")
                } else if !feature.sample_rate_hz().is_finite() || feature.sample_rate_hz() <= 0.0 {
                    Some("live capture sample rate must be positive")
                } else if !feature
                    .capabilities()
                    .supports(feature.channels(), feature.sample_rate_hz())
                {
                    Some("live capture settings are not advertised by the provider")
                } else if feature
                    .simple_trigger_channels()
                    .iter()
                    .any(|channel| channel.viewer_channel >= feature.channels().len())
                {
                    Some("live capture trigger channel references an unknown viewer channel")
                } else if feature.session_plan().is_some_and(|plan| {
                    plan.channel_count != feature.channels().len() as u64
                        || plan.sample_rate_hz as f64 != feature.sample_rate_hz()
                }) {
                    Some("live capture session plan differs from its active channel/rate tuple")
                } else if feature.session_plan().is_some_and(|plan| {
                    plan.policy
                        .effective
                        .trigger_timeout
                        .is_some_and(|timeout| {
                            timeout.action == signal_processing::TriggerTimeoutAction::ForceTrigger
                                && !feature.capabilities().commands().force_trigger
                        })
                }) {
                    Some("live capture policy requests Force Trigger without advertising it")
                } else if duplicate_trigger_channels {
                    Some("live capture trigger channels must have unique identities and lanes")
                } else {
                    None
                };
                if let Some(message) = invalid {
                    return Err(LiveCaptureDiscoveryError {
                        source_nodes: vec![node.id],
                        message: format!("{}: {message}", node.title),
                    });
                }
                candidates.push(DiscoveredLiveCaptureFeature::new_with_visible_channels(
                    node.id,
                    node.title.clone(),
                    capture_channel_selection(subscriptions, node.id, node, builder),
                    feature,
                ));
            }
            Ok(None) => {}
            Err(message) => {
                return Err(LiveCaptureDiscoveryError {
                    source_nodes: vec![node.id],
                    message: format!("{}: {message}", node.title),
                });
            }
        }
    }

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => {
            let mut source_nodes: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.source_node)
                .collect();
            source_nodes.sort_unstable_by_key(|node| node.0);
            Err(LiveCaptureDiscoveryError {
                source_nodes,
                message: "the graph contains multiple live capture sources".into(),
            })
        }
    }
}

// ── IR ───────────────────────────────────────────────────────────────────────

/// Pure description — no threads, no channels. Cheap to rebuild on every
/// edit and cheap to diff (live reconfiguration).
#[derive(Debug, Clone, Default)]
pub struct CompiledGraph {
    pub nodes: Vec<CompiledNode>,
    pub edges: Vec<CompiledEdge>,
    pub derived_data_retention: DerivedDataRetention,
    pub sampling_overlays: Vec<SamplingOverlayCandidate>,
}

#[derive(Debug, Clone)]
pub struct CompiledNode {
    pub id: NodeId,
    /// `BuilderRegistry` key (the UI def name).
    pub builder: String,
    pub state: Value,
    /// Pipeline node name: `n{id}_{title_slug}`.
    pub runtime_name: String,
    pub data_collector: bool,
    pub resolved: ResolvedInputs,
    pub capture_cache_identity: CaptureCacheIdentity,
    pub derived_word_caches: Vec<Option<PersistentStoreConfig>>,
}

#[derive(Debug, Clone)]
pub struct CompiledEdge {
    pub from: (NodeId, String),
    pub to: (NodeId, String),
    pub buffer: usize,
    pub kind: PortKind,
}

fn runtime_name(node: &Node) -> String {
    let slug: String = node
        .title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("n{}_{}", node.id.0, slug.trim_matches('_'))
}

// ── Stage 1: lower ───────────────────────────────────────────────────────────

/// A UI wire with reroutes and muted nodes collapsed away: both endpoints on
/// live (non-reroute, non-muted) regular nodes.
struct Wire {
    from: SocketId,
    to: SocketId,
}

enum WireSource {
    Found(SocketId),
    /// Upstream is missing entirely (e.g. an unplugged reroute) — silently
    /// drop the wire, matching pre-existing reroute behavior.
    Dangling,
    /// A muted node's output has no viable pass-through: either its own
    /// sockets have no type-compatible input at all (a type-transforming
    /// node like a decoder, or a source with nothing to pass through in the
    /// first place), or the one that would match isn't connected.
    MutedBlocked {
        output: SocketId,
    },
}

/// Chases `from` back through any run of Reroute and muted nodes to the
/// effective producing socket. At a muted hop, follows
/// `Node::mute_pass_through_pairs` — the node's own declared input/output
/// type pairing, independent of whatever is wired downstream (mirrors
/// Blender: a muted node only usefully bypasses through a same-typed
/// input/output pair; a type-transforming node has none, so its output has
/// nothing to splice to and just drops).
fn resolve_wire_source(graph: &GraphState, from: SocketId, hops: &mut usize) -> WireSource {
    *hops += 1;
    if *hops > graph.connections.len() + graph.nodes.len() + 1 {
        return WireSource::Dangling; // cycle guard
    }
    let Some(node) = graph.nodes.get(&from.node) else {
        return WireSource::Dangling;
    };
    if node.kind == NodeKind::Reroute {
        return match graph.connections.iter().find(|c| c.to.node == from.node) {
            Some(upstream) => resolve_wire_source(graph, upstream.from, hops),
            None => WireSource::Dangling,
        };
    }
    if node.muted {
        let Some(&(_, in_idx)) = node
            .mute_pass_through_pairs()
            .iter()
            .find(|(out_idx, _)| *out_idx == from.index)
        else {
            return WireSource::MutedBlocked { output: from };
        };
        let paired_input = SocketId {
            node: from.node,
            index: in_idx,
            direction: SocketDirection::Input,
        };
        return match graph.connections.iter().find(|c| c.to == paired_input) {
            Some(upstream) => resolve_wire_source(graph, upstream.from, hops),
            None => WireSource::MutedBlocked { output: from },
        };
    }
    WireSource::Found(from)
}

fn resolve_reroute_edges(graph: &GraphState) -> (Vec<Wire>, Vec<CompileError>) {
    let mut wires = Vec::new();
    let mut errors = Vec::new();
    let mut blocked: HashSet<SocketId> = HashSet::new();
    for connection in &graph.connections {
        let Some(to_node) = graph.nodes.get(&connection.to.node) else {
            continue;
        };
        if to_node.kind == NodeKind::Reroute || to_node.muted {
            // Handled when the wire *leaving* it is resolved.
            continue;
        }
        let mut hops = 0usize;
        match resolve_wire_source(graph, connection.from, &mut hops) {
            WireSource::Found(from) => wires.push(Wire {
                from,
                to: connection.to,
            }),
            WireSource::Dangling => {}
            WireSource::MutedBlocked { output } => {
                if blocked.insert(output) {
                    let output_name = graph
                        .nodes
                        .get(&output.node)
                        .and_then(|n| n.outputs.get(output.index))
                        .map(|s| s.name.as_str())
                        .unwrap_or("?");
                    let to_label = graph
                        .nodes
                        .get(&connection.to.node)
                        .and_then(|n| n.inputs.get(connection.to.index).map(|s| (n, s)))
                        .map(|(n, s)| format!("{}.{}", n.title, s.name))
                        .unwrap_or_else(|| "?".to_string());
                    errors.push(CompileError::on(
                        output.node,
                        format!(
                            "Muted: '{output_name}' has no type-matching input to pass through — '{to_label}' loses its input"
                        ),
                    ));
                }
            }
        }
    }
    (wires, errors)
}

/// Position of a variadic member within its group (0-based); 0 for plain
/// sockets.
fn member_index(node: &Node, socket_index: usize) -> usize {
    let Some(socket) = node.inputs.get(socket_index) else {
        return 0;
    };
    if !socket.is_variadic_member() {
        return 0;
    }
    node.inputs[..socket_index]
        .iter()
        .filter(|other| other.def_index == socket.def_index && other.is_variadic_member())
        .count()
}

/// Compiler-synthesized collector identities are stable across repeated
/// lowering. Table data shares one collector; retained outputs use one
/// collector per producer so adding another producer never restarts an
/// existing cache.
const AUTO_DATA_COLLECTOR_NODE_ID: NodeId = NodeId(u32::MAX - 1);

fn auto_output_collector_node_id(producer: NodeId, graph: &GraphState) -> NodeId {
    let mut candidate = u32::MAX.wrapping_sub(2).wrapping_sub(producer.0);
    loop {
        let id = NodeId(candidate);
        if id != AUTO_DATA_COLLECTOR_NODE_ID && !graph.nodes.contains_key(&id) {
            return id;
        }
        candidate = candidate.wrapping_sub(1);
    }
}

/// Adds presentation-neutral retention and table collectors through the
/// same generic sink and edge-negotiation path as explicit graph sinks.
fn with_output_collectors(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> GraphState {
    let subscribable = registry.subscribable_payload_kinds();
    let mut watched: Vec<(SocketId, String)> = graph
        .nodes
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Regular)
        .flat_map(|(&id, node)| {
            let subscribable = &subscribable;
            node.outputs
                .iter()
                .enumerate()
                .filter(move |(index, output)| {
                    let Some(builder) = registry.get(node.def_name()) else {
                        return false;
                    };
                    let connected = graph.connections.iter().any(|connection| {
                        connection.from.node == id && connection.from.index == *index
                    });
                    let already_collected = graph.connections.iter().any(|connection| {
                        connection.from.node == id
                            && connection.from.index == *index
                            && graph
                                .nodes
                                .get(&connection.to.node)
                                .and_then(|target| registry.get(target.def_name()))
                                .is_some_and(|target| {
                                    target.is_data_collector() || target.is_data_subscription()
                                })
                    });
                    (connected || subscriptions.is_retained(id, *index))
                        && !already_collected
                        && builder.viewer_channel_origin(output, &node.state).is_none()
                        && builder
                            .offered_kinds(output, &node.state)
                            .into_iter()
                            .any(|kind| {
                                subscribable.contains(&kind)
                                    && builder.output_port(output, &node.state, kind).is_some()
                            })
                })
                .map(move |(index, output)| {
                    (
                        SocketId {
                            node: id,
                            index,
                            direction: SocketDirection::Output,
                        },
                        format!("{}.{}", node.title, output.name),
                    )
                })
        })
        .collect();
    // Concrete nodes order related presentation outputs in their socket
    // schema. Preserve that explicit order without interpreting labels.
    watched.sort_by_key(|(socket, _)| (socket.node.0, socket.index));

    let mut tabled: Vec<(SocketId, String)> = graph
        .nodes
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Regular)
        .flat_map(|(&id, node)| {
            let Some(builder) = registry.get(node.def_name()) else {
                return Vec::new().into_iter();
            };
            node.outputs
                .iter()
                .enumerate()
                .filter(|(index, output)| {
                    let retained = watched
                        .iter()
                        .any(|(socket, _)| socket.node == id && socket.index == *index);
                    let collected_by_explicit_sink = graph.connections.iter().any(|connection| {
                        connection.from.node == id
                            && connection.from.index == *index
                            && graph
                                .nodes
                                .get(&connection.to.node)
                                .and_then(|target| registry.get(target.def_name()))
                                .is_some_and(|builder| {
                                    builder.is_data_collector() || builder.is_data_subscription()
                                })
                    });
                    !retained
                        && !collected_by_explicit_sink
                        && builder.decoder_table_column(output, &node.state).is_some()
                        && builder
                            .offered_kinds(output, &node.state)
                            .into_iter()
                            .any(|kind| builder.output_port(output, &node.state, kind).is_some())
                })
                .map(move |(index, output)| {
                    (
                        SocketId {
                            node: id,
                            index,
                            direction: SocketDirection::Output,
                        },
                        format!("{}.{}", node.title, output.name),
                    )
                })
                .collect::<Vec<_>>()
                .into_iter()
        })
        .collect();
    tabled.sort_by_key(|(socket, _)| (socket.node.0, socket.index));

    let mut graph = graph.clone();
    let mut first = 0;
    while first < watched.len() {
        let producer = watched[first].0.node;
        let end = watched[first..]
            .iter()
            .position(|(socket, _)| socket.node != producer)
            .map_or(watched.len(), |offset| first + offset);
        let retained = &watched[first..end];
        let collector_id = auto_output_collector_node_id(producer, &graph);
        let inputs: Vec<Socket> = retained
            .iter()
            .map(|(_, label)| Socket {
                schema_id: String::new(),
                name: label.clone(),
                type_name: "Any".to_owned(),
                color: Default::default(),
                shape: SocketShape::Circle,
                allowed: Vec::new(),
                resolved_type: None,
                def_index: 0,
                variadic: Some(VariadicInfo {
                    base: "In".to_owned(),
                    max: retained.len(),
                    placeholder: false,
                }),
                visible: false,
                editor_visible: false,
                hidden: true,
                has_control: false,
                extensions: Default::default(),
            })
            .collect();
        let mut collector = Node::blank(
            collector_id,
            crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME,
            Default::default(),
        );
        collector.title = "Retained Output Collector".to_owned();
        collector.inputs = inputs;
        collector.state = Value::Null;
        graph.nodes.insert(collector_id, collector);
        graph
            .connections
            .extend(
                retained
                    .iter()
                    .enumerate()
                    .map(|(member, (from, _))| Connection {
                        from: *from,
                        to: SocketId {
                            node: collector_id,
                            index: member,
                            direction: SocketDirection::Input,
                        },
                    }),
            );
        first = end;
    }
    if !tabled.is_empty() {
        let inputs = tabled
            .iter()
            .map(|(_, label)| Socket {
                schema_id: String::new(),
                name: label.clone(),
                type_name: "Any".to_owned(),
                color: Default::default(),
                shape: SocketShape::Circle,
                allowed: Vec::new(),
                resolved_type: None,
                def_index: 0,
                variadic: Some(VariadicInfo {
                    base: "In".to_owned(),
                    max: tabled.len(),
                    placeholder: false,
                }),
                visible: false,
                editor_visible: false,
                hidden: true,
                has_control: false,
                extensions: Default::default(),
            })
            .collect();
        let mut collector = Node::blank(
            AUTO_DATA_COLLECTOR_NODE_ID,
            crate::DATA_COLLECTOR_BUILDER,
            Default::default(),
        );
        collector.title = "Derived Data Collector".to_owned();
        collector.inputs = inputs;
        collector.state = Value::Null;
        graph.nodes.insert(AUTO_DATA_COLLECTOR_NODE_ID, collector);
        graph
            .connections
            .extend(
                tabled
                    .into_iter()
                    .enumerate()
                    .map(|(member, (from, _))| Connection {
                        from,
                        to: SocketId {
                            node: AUTO_DATA_COLLECTOR_NODE_ID,
                            index: member,
                            direction: SocketDirection::Input,
                        },
                    }),
            );
    }
    graph
}

pub(crate) fn lower_with_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<CompiledGraph, Vec<CompileError>> {
    let augmented = with_output_collectors(graph, registry, subscriptions);
    let graph = &augmented;
    let (wires, mut errors) = resolve_reroute_edges(graph);

    // Prune: keep only what feeds a sink.
    let sinks: Vec<NodeId> = graph
        .nodes
        .values()
        .filter(|node| {
            node.kind == NodeKind::Regular
                && registry
                    .get(node.def_name())
                    .is_some_and(|builder| builder.is_sink() || builder.is_data_subscription())
        })
        .map(|node| node.id)
        .collect();
    if sinks.is_empty() {
        return Err(vec![CompileError::global(
            "Graph has no sink (add a File Writer)",
        )]);
    }
    let mut keep: HashSet<NodeId> = HashSet::new();
    let mut stack = sinks.clone();
    while let Some(id) = stack.pop() {
        if !keep.insert(id) {
            continue;
        }
        for wire in &wires {
            if wire.to.node == id && !keep.contains(&wire.from.node) {
                stack.push(wire.from.node);
            }
        }
    }
    let mut kept: Vec<NodeId> = keep.iter().copied().collect();
    kept.sort_by_key(|id| id.0);

    // Every kept node must have a runtime. At least one zero-input runtime
    // source is required, while at most one source may establish the capture
    // time domain; auxiliary sources carry values already on that timeline.
    let mut runtime_source_count = 0usize;
    let mut time_domain_source_count = 0usize;
    let mut derived_data_retention = DerivedDataRetention::Unlimited;
    for &id in &kept {
        let node = &graph.nodes[&id];
        match registry.get(node.def_name()) {
            None => errors.push(CompileError::on(
                id,
                format!("'{}' has no runtime implementation", node.def_name()),
            )),
            Some(builder) if builder.is_source() => {
                runtime_source_count += 1;
                if builder.is_time_domain_source() {
                    time_domain_source_count += 1;
                    derived_data_retention = builder.derived_data_retention(&node.state);
                }
            }
            Some(_) => {}
        }
    }
    if runtime_source_count == 0 {
        errors.push(CompileError::global("Graph has no data source"));
    } else if time_domain_source_count > 1 {
        for &id in &kept {
            let node = &graph.nodes[&id];
            if registry
                .get(node.def_name())
                .is_some_and(|builder| builder.is_time_domain_source())
            {
                errors.push(CompileError::on(
                    id,
                    "Multiple sources: a graph has exactly one time domain",
                ));
            }
        }
    }

    // Negotiate kinds and ports per edge.
    let mut resolved: HashMap<NodeId, ResolvedInputs> = HashMap::new();
    let mut edges: Vec<CompiledEdge> = Vec::new();
    let mut connected: HashMap<NodeId, HashSet<usize>> = HashMap::new();
    for wire in &wires {
        if !keep.contains(&wire.from.node) || !keep.contains(&wire.to.node) {
            continue;
        }
        let from_node = &graph.nodes[&wire.from.node];
        let to_node = &graph.nodes[&wire.to.node];
        let (Some(from_builder), Some(to_builder)) = (
            registry.get(from_node.def_name()),
            registry.get(to_node.def_name()),
        ) else {
            continue; // already reported above
        };
        let (Some(from_socket), Some(to_socket)) = (
            from_node.outputs.get(wire.from.index),
            to_node.inputs.get(wire.to.index),
        ) else {
            errors.push(CompileError::on(wire.to.node, "Dangling connection"));
            continue;
        };

        connected
            .entry(wire.to.node)
            .or_default()
            .insert(wire.to.index);

        let offered = from_builder.offered_kinds(from_socket, &from_node.state);
        let data_subscription = to_builder.is_data_subscription();
        let registered_collection = data_subscription || to_builder.is_data_collector();
        let accepted = if registered_collection {
            registry.subscribable_payload_kinds()
        } else {
            to_builder.accepted_kinds(to_socket, &to_node.state)
        };
        let Some(kind) = offered.iter().copied().find(|k| accepted.contains(k)) else {
            let message = if registered_collection {
                format!(
                    "collected-data input cannot retain '{}' because its payload has no registered subscription contract",
                    from_socket.name
                )
            } else {
                format!(
                    "'{}' cannot consume what '{}' produces on '{}'",
                    to_socket.name, from_node.title, from_socket.name
                )
            };
            errors.push(CompileError::on(wire.to.node, message));
            continue;
        };
        let offered_contracts =
            from_builder.offered_connection_contracts(from_socket, &from_node.state);
        let accepted_contracts =
            to_builder.accepted_connection_contracts(to_socket, &to_node.state);
        if !connection_contracts_overlap(&offered_contracts, &accepted_contracts) {
            errors.push(CompileError::on(
                wire.to.node,
                format!(
                    "'{}' accepts contracts [{}], but '{}' offers [{}]",
                    to_socket.name,
                    accepted_contracts.join(", "),
                    from_socket.name,
                    offered_contracts.join(", ")
                ),
            ));
            continue;
        }

        let Some(out_port) = from_builder.output_port(from_socket, &from_node.state, kind) else {
            errors.push(CompileError::on(
                wire.from.node,
                format!("No runtime port for output '{}'", from_socket.name),
            ));
            continue;
        };
        let member = member_index(to_node, wire.to.index);
        let Some(in_port) = to_builder.input_port(to_socket, member, &to_node.state, kind) else {
            errors.push(CompileError::on(
                wire.to.node,
                format!("No runtime port for input '{}'", to_socket.name),
            ));
            continue;
        };

        resolved.entry(wire.to.node).or_default().insert(
            to_socket.def_index,
            member,
            ResolvedInput {
                kind,
                source: format!("{}.{}", from_node.title, from_socket.name),
                source_node: wire.from.node,
                source_output: wire.from.index,
                source_node_title: from_node.title.clone(),
                word_display_format: from_builder
                    .word_display_format(from_socket, &from_node.state),
                lane_presentation: from_builder.lane_presentation(from_socket, &from_node.state),
                default_lane_presentation: registered_collection
                    .then(|| registry.payload_subscription_presentation(kind))
                    .flatten(),
                decoder_table_column: from_builder
                    .decoder_table_column(from_socket, &from_node.state),
                capture_channel: from_builder.viewer_channel_origin(from_socket, &from_node.state),
            },
        );
        edges.push(CompiledEdge {
            from: (wire.from.node, out_port),
            to: (wire.to.node, in_port),
            buffer: to_builder
                .input_buffer_override(to_socket, &to_node.state)
                .unwrap_or_else(|| kind.buffer_size(from_builder.is_source())),
            kind,
        });
    }

    // Required inputs.
    for &id in &kept {
        let node = &graph.nodes[&id];
        let Some(builder) = registry.get(node.def_name()) else {
            continue;
        };
        let node_connected = connected.get(&id);
        for (index, socket) in node.inputs.iter().enumerate() {
            // Control-bearing sockets go through `input_required` like any
            // other: most are self-supplying config (their builders return
            // false), but one can be conditionally required — the writer's
            // Filename picker is required exactly while its value is empty.
            if !socket.visible {
                continue;
            }
            if socket.is_variadic_placeholder() {
                let has_member = node
                    .inputs
                    .iter()
                    .any(|s| s.def_index == socket.def_index && s.is_variadic_member());
                if !has_member && builder.input_required(socket, &node.state) {
                    errors.push(CompileError::on(
                        id,
                        format!("Input '{}' needs at least one connection", socket.name),
                    ));
                }
            } else if !socket.is_variadic_member()
                && !node_connected.is_some_and(|set| set.contains(&index))
                && builder.input_required(socket, &node.state)
            {
                errors.push(CompileError::on(
                    id,
                    format!("Input '{}' is not connected", socket.name),
                ));
            }
        }
    }

    // Cycle check (a cycle would deadlock the pipeline).
    if has_cycle(&kept, &edges) {
        errors.push(CompileError::global("Graph contains a cycle"));
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let nodes: Vec<CompiledNode> = kept
        .iter()
        .map(|&id| {
            let node = &graph.nodes[&id];
            let resolved = resolved.remove(&id).unwrap_or_default();
            let builder = registry
                .get(node.def_name())
                .expect("retained node has a registered builder");
            CompiledNode {
                id,
                builder: node.def_name().to_owned(),
                state: node.state.clone(),
                runtime_name: runtime_name(node),
                data_collector: builder.is_data_collector() || builder.is_data_subscription(),
                capture_cache_identity: builder.capture_cache_identity(&node.state, &resolved),
                resolved,
                derived_word_caches: Vec::new(),
            }
        })
        .collect();
    let sampling_overlays = nodes
        .iter()
        .filter_map(|compiled_node| {
            let builder = registry.get(&compiled_node.builder)?;
            let descriptor = builder.sampling_overlay(&compiled_node.state)?;
            let clock_channel = compiled_node
                .resolved
                .get(descriptor.clock_input, 0)?
                .capture_channel?;
            let sampled_channels = descriptor
                .sampled_input_groups
                .iter()
                .flat_map(|def_index| compiled_node.resolved.members(*def_index))
                .filter_map(|(_, input)| input.capture_channel)
                .fold(Vec::new(), |mut channels, channel| {
                    if !channels.contains(&channel) {
                        channels.push(channel);
                    }
                    channels
                });
            if sampled_channels.is_empty() {
                return None;
            }
            let retained_word_lane = descriptor.retained_word_source.and_then(|source| {
                retained_word_sampling_lane_name(&nodes, registry, compiled_node.id, source.output)
                    .map(|name| RetainedWordSamplingLane {
                        name,
                        clock_high: source.clock_high,
                    })
            });
            Some(SamplingOverlayCandidate {
                node_id: compiled_node.id,
                node_title: graph.nodes[&compiled_node.id].title.clone(),
                overlay: ResolvedSamplingOverlay {
                    clock_channel,
                    sampled_channels,
                    points: SamplingPointStore::default(),
                },
                cache_key: None,
                retained_word_lane,
            })
        })
        .collect();
    let compiled = CompiledGraph {
        nodes,
        edges,
        derived_data_retention,
        sampling_overlays,
    };
    let mut compiled = compiled;
    cache_policy::assign_derived_word_caches(&mut compiled, registry);
    cache_policy::assign_sampling_point_caches(&mut compiled);
    Ok(compiled)
}

fn retained_word_sampling_lane_name(
    nodes: &[CompiledNode],
    registry: &BuilderRegistry,
    source_node: NodeId,
    source_output: usize,
) -> Option<String> {
    nodes
        .iter()
        .filter(|node| node.data_collector)
        .find_map(|node| {
            let builder = registry.get(&node.builder)?;
            let names = builder.collected_lane_names(&node.state, &node.resolved);
            node.resolved
                .members(0)
                .into_iter()
                .find_map(|(member, input)| {
                    (input.source_node == source_node && input.source_output == source_output)
                        .then(|| {
                            names
                                .iter()
                                .find(|(candidate, _)| *candidate == member)
                                .map(|(_, name)| name.clone())
                        })
                        .flatten()
                })
        })
}

#[cfg(test)]
pub(crate) fn test_output_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
) -> OutputSubscriptionPlan {
    graph
        .nodes
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Regular)
        .flat_map(|(&node_id, node)| {
            let Some(builder) = registry.get(node.def_name()) else {
                return Vec::new().into_iter();
            };
            node.outputs
                .iter()
                .enumerate()
                .filter_map(move |(output_index, output)| {
                    let logic_analyzer_graph_api::node_support::ViewerOutputControl::Selectable {
                        default_selected,
                        ..
                    } = builder.viewer_output_control(output, &node.state)?
                    else {
                        return None;
                    };
                    output
                        .extensions
                        .get("show_in_view")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(default_selected)
                        .then_some((node_id, output_index))
                })
                .collect::<Vec<_>>()
                .into_iter()
        })
        .collect()
}

#[cfg(test)]
fn lower(
    graph: &GraphState,
    registry: &BuilderRegistry,
) -> Result<CompiledGraph, Vec<CompileError>> {
    let subscriptions = test_output_subscriptions(graph, registry);
    lower_with_subscriptions(graph, registry, &subscriptions)
}

fn connection_contracts_overlap(offered: &[String], accepted: &[String]) -> bool {
    offered.is_empty()
        || accepted.is_empty()
        || offered.iter().any(|contract| accepted.contains(contract))
}

/// Resolves the clocked-node sampling presentations available for the
/// current graph without starting its runtime. Hosts use this to populate
/// presentation controls before the user runs the pipeline.
pub(crate) fn sampling_overlay_candidates(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
    lower_with_subscriptions(graph, registry, subscriptions)
        .map(|compiled| compiled.sampling_overlays)
}

fn sampling_point_map(compiled: &CompiledGraph) -> HashMap<String, SamplingPointStore> {
    compiled
        .sampling_overlays
        .iter()
        .map(|candidate| {
            (
                compiled_node(compiled, candidate.node_id)
                    .runtime_name
                    .clone(),
                candidate.overlay.points.clone(),
            )
        })
        .collect()
}

fn reuse_sampling_points(previous: &CompiledGraph, next: &mut CompiledGraph) {
    for candidate in &mut next.sampling_overlays {
        let Some(previous_candidate) = previous
            .sampling_overlays
            .iter()
            .find(|previous| previous.node_id == candidate.node_id)
        else {
            continue;
        };
        candidate.overlay.points = previous_candidate.overlay.points.clone();
    }
}

fn has_cycle(nodes: &[NodeId], edges: &[CompiledEdge]) -> bool {
    let mut indegree: HashMap<NodeId, usize> = nodes.iter().map(|&id| (id, 0)).collect();
    for edge in edges {
        *indegree.entry(edge.to.0).or_default() += 1;
    }
    let mut queue: Vec<NodeId> = indegree
        .iter()
        .filter(|entry| *entry.1 == 0)
        .map(|(&id, _)| id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop() {
        visited += 1;
        for edge in edges.iter().filter(|e| e.from.0 == id) {
            let d = indegree.get_mut(&edge.to.0).expect("edge endpoints kept");
            *d -= 1;
            if *d == 0 {
                queue.push(edge.to.0);
            }
        }
    }
    visited != nodes.len()
}

// ── Live pipeline ───────────────────────────────────────────────────────

/// Producers-before-consumers order; `lower` already rejected cycles.
fn topo_order(compiled: &CompiledGraph) -> Vec<NodeId> {
    let mut indegree: HashMap<NodeId, usize> =
        compiled.nodes.iter().map(|node| (node.id, 0)).collect();
    for edge in &compiled.edges {
        *indegree.entry(edge.to.0).or_default() += 1;
    }
    let mut queue: Vec<NodeId> = compiled
        .nodes
        .iter()
        .map(|node| node.id)
        .filter(|id| indegree[id] == 0)
        .collect();
    queue.sort_by_key(|id| id.0);
    let mut order = Vec::with_capacity(compiled.nodes.len());
    while let Some(id) = queue.pop() {
        order.push(id);
        for edge in compiled.edges.iter().filter(|edge| edge.from.0 == id) {
            let degree = indegree.get_mut(&edge.to.0).expect("kept node");
            *degree -= 1;
            if *degree == 0 {
                queue.push(edge.to.0);
            }
        }
    }
    order
}

pub(crate) fn compiled_node(compiled: &CompiledGraph, id: NodeId) -> &CompiledNode {
    compiled
        .nodes
        .iter()
        .find(|node| node.id == id)
        .expect("node in compiled graph")
}

fn materialize_compiled_node(
    node: &CompiledNode,
    builder: &dyn RuntimeBuilder,
    runtime_name: &str,
    registry: &BuilderRegistry,
    ctx: &mut CompileCtx,
) -> Result<Box<dyn ProcessNode>, String> {
    if builder.is_data_subscription() || builder.is_data_collector() {
        return DataCollectorBuilder::build_with_lane_names(
            runtime_name,
            &node.resolved,
            &builder.collected_lane_names(&node.state, &node.resolved),
            registry,
            ctx,
        );
    }
    builder.build(runtime_name, &node.state, &node.resolved, ctx)
}

fn collected_table_subscriptions(
    compiled: &CompiledGraph,
    registry: &BuilderRegistry,
) -> Vec<CollectedTableSubscription> {
    compiled
        .nodes
        .iter()
        .filter(|node| node.data_collector)
        .filter_map(|node| {
            let builder = registry.get(&node.builder)?;
            let lanes = builder
                .collected_lane_names(&node.state, &node.resolved)
                .into_iter()
                .filter_map(|(member, lane_name)| {
                    let input = node.resolved.get(0, member)?.clone();
                    input
                        .decoder_table_column
                        .is_some()
                        .then_some(CollectedOutputLane {
                            member,
                            lane_name,
                            source_label: input.source_node_title.clone(),
                            input,
                        })
                })
                .collect::<Vec<_>>();
            (!lanes.is_empty()).then_some(CollectedTableSubscription {
                collector: node.id,
                lanes,
            })
        })
        .collect()
}

fn collected_output_subscriptions(
    compiled: &CompiledGraph,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Vec<CollectedOutputSubscription> {
    compiled
        .nodes
        .iter()
        .filter_map(|node| {
            let builder = registry.get(&node.builder)?;
            node.data_collector
                .then(|| {
                    let lanes: Vec<CollectedOutputLane> = builder
                        .collected_lane_names(&node.state, &node.resolved)
                        .into_iter()
                        .filter_map(|(member, lane_name)| {
                            node.resolved.get(0, member).cloned().and_then(|input| {
                                subscriptions
                                    .contains(input.source_node, input.source_output)
                                    .then(|| CollectedOutputLane {
                                        source_label: builder.collected_source_label(
                                            &node.state,
                                            &input.source_node_title,
                                        ),
                                        member,
                                        lane_name,
                                        input,
                                    })
                            })
                        })
                        .collect();
                    (!lanes.is_empty()).then_some(CollectedOutputSubscription {
                        runtime_name: node.runtime_name.clone(),
                        lanes,
                    })
                })
                .flatten()
        })
        .collect()
}

pub(crate) fn derived_cache_configs_by_node_with_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    repository: &Arc<dyn ArtifactRepository>,
) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
    cache_policy::cache_configs_by_node(graph, registry, subscriptions, repository)
}

#[cfg(test)]
fn derived_cache_configs_by_node(
    graph: &GraphState,
    registry: &BuilderRegistry,
) -> Result<HashMap<NodeId, Vec<PersistentStoreConfig>>, Vec<CompileError>> {
    let subscriptions = test_output_subscriptions(graph, registry);
    let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
    derived_cache_configs_by_node_with_subscriptions(graph, registry, &subscriptions, &repository)
}

/// Input subscriptions for `id`, matched to the built node's input schema.
fn input_subs(
    compiled: &CompiledGraph,
    id: NodeId,
    built: &dyn ProcessNode,
    names: &HashMap<NodeId, String>,
) -> Result<Vec<Option<InputSub>>, String> {
    built
        .input_schema()
        .iter()
        .map(|schema| {
            let edge = compiled
                .edges
                .iter()
                .find(|edge| edge.to.0 == id && edge.to.1 == schema.name);
            match edge {
                None => Ok(None),
                Some(edge) => {
                    let from_node = names
                        .get(&edge.from.0)
                        .ok_or_else(|| format!("producer n{} not materialized", edge.from.0.0))?;
                    Ok(Some(InputSub {
                        from_node: from_node.clone(),
                        from_port: edge.from.1.clone(),
                        buffer: edge.buffer,
                        policy: OverflowPolicy::Block,
                    }))
                }
            }
        })
        .collect()
}

/// One live edit, in application order (removals reverse-topological,
/// additions topological, then hot configs and in-place restarts).
#[derive(Debug)]
enum LiveEdit {
    Remove(NodeId),
    Add(NodeId),
    Configure(NodeId, NodeConfig),
    Restart(NodeId),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApplySummary {
    pub added: usize,
    pub removed: usize,
    pub configured: usize,
    pub restarted: usize,
}

impl ApplySummary {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Wiring signature of a node's inputs, for diffing.
fn wiring_of(compiled: &CompiledGraph, id: NodeId) -> BTreeSet<(String, u32, String, usize)> {
    compiled
        .edges
        .iter()
        .filter(|edge| edge.to.0 == id)
        .map(|edge| {
            (
                edge.to.1.clone(),
                edge.from.0.0,
                edge.from.1.clone(),
                edge.buffer,
            )
        })
        .collect()
}

/// Classifies the difference between the running IR and the edited one
/// (the edit classes of `docs/APP_DESIGN.md`). Returns the edit list, or
/// the reason a full restart is needed.
fn diff(
    old: &CompiledGraph,
    new: &CompiledGraph,
    registry: &BuilderRegistry,
) -> Result<Vec<LiveEdit>, String> {
    let old_ids: HashSet<NodeId> = old.nodes.iter().map(|node| node.id).collect();
    let new_ids: HashSet<NodeId> = new.nodes.iter().map(|node| node.id).collect();
    let is_source = |compiled: &CompiledGraph, id: NodeId| {
        registry
            .get(&compiled_node(compiled, id).builder)
            .is_some_and(|builder| builder.is_time_domain_source())
    };

    let mut edits: Vec<LiveEdit> = Vec::new();

    // Removals, consumers before producers.
    let mut removals: Vec<NodeId> = topo_order(old)
        .into_iter()
        .rev()
        .filter(|id| !new_ids.contains(id))
        .collect();
    for &id in &removals {
        if is_source(old, id) {
            return Err("the source node was removed".into());
        }
    }
    edits.extend(removals.drain(..).map(LiveEdit::Remove));

    // Additions, producers before consumers.
    for id in topo_order(new) {
        if old_ids.contains(&id) {
            continue;
        }
        for edge in new.edges.iter().filter(|edge| edge.to.0 == id) {
            if edge.kind == PortKind::of::<SampleBlock>() {
                return Err(
                    "new node consumes block channels; block subscriptions cannot join mid-stream"
                        .to_string(),
                );
            }
            if is_source(new, edge.from.0) {
                return Err(
                    "new connection directly to the source; source destinations are fixed at start"
                        .into(),
                );
            }
        }
        edits.push(LiveEdit::Add(id));
    }

    // Changed nodes: hot config, or restart in place.
    for id in topo_order(new) {
        if !old_ids.contains(&id) {
            continue;
        }
        let old_node = compiled_node(old, id);
        let new_node = compiled_node(new, id);
        let wiring_changed = wiring_of(old, id) != wiring_of(new, id);
        let builder = registry
            .get(&new_node.builder)
            .ok_or_else(|| format!("no builder for '{}'", new_node.builder))?;
        let state_changed =
            builder.execution_state(&old_node.state) != builder.execution_state(&new_node.state);
        if !wiring_changed && !state_changed {
            continue;
        }
        if is_source(new, id) {
            return Err("the source node changed".into());
        }
        if !wiring_changed
            && state_changed
            && let Some(config) = builder.hot_config(&new_node.state)
        {
            edits.push(LiveEdit::Configure(id, config));
            continue;
        }
        // Restart in place: the node re-subscribes to its producers, which
        // is invisible to block streams and to source ports (their worker
        // threads snapshot destinations at start).
        for edge in new.edges.iter().filter(|edge| edge.to.0 == id) {
            if edge.kind == PortKind::of::<SampleBlock>() {
                return Err(format!(
                    "'{}' consumes block channels and cannot restart mid-stream",
                    new_node.runtime_name
                ));
            }
            if is_source(new, edge.from.0) {
                return Err(format!(
                    "'{}' is fed directly by the source and cannot restart mid-stream",
                    new_node.runtime_name
                ));
            }
        }
        edits.push(LiveEdit::Restart(id));
    }

    Ok(edits)
}

/// A pipeline running under the live supervisor: editable while it runs.
pub struct LiveRun {
    manager: AppManager,
    compiled: CompiledGraph,
    /// Supervisor key per UI node — assigned at add time and stable across
    /// title renames and in-place restarts.
    names: HashMap<NodeId, String>,
    lanes: DerivedLanes,
    collected_output_subscriptions: Vec<CollectedOutputSubscription>,
    collected_table_subscriptions: Vec<CollectedTableSubscription>,
    diagnostics: RunDiagnosticRegistry,
    source_readiness: SourceReadinessRegistry,
    /// Set by [`Self::stop`]: the wind-down has been signalled but node
    /// threads may still be finishing their current `work()` call.
    stop_requested: bool,
    cache_pruned: bool,
    timeline_markers: HashMap<TimelineMarkerReference, signal_processing::TimelineMarker>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

/// One provider-owned source process used only while a live capture follows
/// its authoritative store.
pub struct LiveAnalysisSource {
    pub source_node: NodeId,
    pub process: Box<dyn ProcessNode>,
}

/// Explicit source-node replacements used when materializing a graph.
///
/// The compiler validates every node ID against the lowered graph and never
/// interprets the source process or discovers a provider. Live capture and
/// finalized replay therefore share one substitution mechanism.
pub type SourceProcessOverrides = HashMap<NodeId, Box<dyn ProcessNode>>;

/// Lowers and materializes `graph` under a host-selected [`AppManager`].
fn start_live_with_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<CompileError>> {
    start_live_inner(
        graph,
        registry,
        subscriptions,
        ctx,
        SourceProcessOverrides::new(),
        runtime_factory,
    )
}

/// Publishes valid persistent derived lanes without executing the processing graph.
pub(crate) fn load_cached_data_with_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
) -> Result<bool, Vec<CompileError>> {
    let mut compiled = lower_with_subscriptions(graph, registry, subscriptions)?;
    cache_policy::configure_repository(&mut compiled, &ctx.artifact_repository);
    let sampling_cache_loaded = cache_policy::open_sampling_point_stores(
        &mut compiled,
        &ctx.derived_lanes,
        &ctx.artifact_repository,
    );
    let preview = cache_policy::prepare_cached_preview(&compiled);
    if preview.is_none() && !sampling_cache_loaded {
        cache_policy::schedule_maintenance(&compiled, &ctx.artifact_repository, &ctx.work_executor);
        return Ok(false);
    }

    ctx.derived_data_retention = compiled.derived_data_retention;
    ctx.sampling_overlays
        .clone_from(&compiled.sampling_overlays);
    ctx.sampling_points = sampling_point_map(&compiled);
    ctx.collected_output_subscriptions = preview
        .as_ref()
        .map(|preview| collected_output_subscriptions(preview, registry, subscriptions))
        .unwrap_or_default();
    ctx.collected_table_subscriptions = preview
        .as_ref()
        .map(|preview| collected_table_subscriptions(preview, registry))
        .unwrap_or_default();

    if let Some(preview) = &preview {
        for node in &preview.nodes {
            let builder = registry.get(&node.builder).ok_or_else(|| {
                vec![CompileError::on(
                    node.id,
                    format!("unknown builder '{}'", node.builder),
                )]
            })?;
            ctx.derived_word_caches
                .clone_from(&node.derived_word_caches);
            materialize_compiled_node(node, builder, &node.runtime_name, registry, ctx)
                .map_err(|message| vec![CompileError::on(node.id, message)])?;
        }
    }
    cache_policy::schedule_maintenance(&compiled, &ctx.artifact_repository, &ctx.work_executor);
    Ok(true)
}

/// Starts the fixed compiled graph with its live-capable source replaced by
/// the process that follows the capture store. All other nodes use the same
/// lowering and materialization path as an ordinary run.
pub(crate) fn start_live_analysis_with_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
    source: LiveAnalysisSource,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<CompileError>> {
    let mut overrides = SourceProcessOverrides::new();
    overrides.insert(source.source_node, source.process);
    start_live_inner(
        graph,
        registry,
        subscriptions,
        ctx,
        overrides,
        runtime_factory,
    )
}

fn start_live_inner(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
    mut source_overrides: SourceProcessOverrides,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<CompileError>> {
    let mut compiled = lower_with_subscriptions(graph, registry, subscriptions)?;
    cache_policy::configure_repository(&mut compiled, &ctx.artifact_repository);
    let (execution, cache_pruned) = cache_policy::prepare_execution(&compiled, registry);
    cache_policy::prepare_sampling_point_stores(
        &mut compiled,
        &execution,
        &ctx.derived_lanes,
        &ctx.artifact_repository,
        &ctx.work_executor,
    );
    ctx.derived_data_retention = compiled.derived_data_retention;
    ctx.sampling_overlays
        .clone_from(&compiled.sampling_overlays);
    ctx.sampling_points = sampling_point_map(&compiled);
    ctx.collected_output_subscriptions =
        collected_output_subscriptions(&compiled, registry, subscriptions);
    ctx.collected_table_subscriptions = collected_table_subscriptions(&compiled, registry);
    let mut manager = runtime_factory.create();
    let mut names: HashMap<NodeId, String> = HashMap::new();

    for source_node in source_overrides.keys().copied() {
        let Some(node) = execution.nodes.iter().find(|node| node.id == source_node) else {
            return Err(vec![CompileError::on(
                source_node,
                "source override is not retained by the compiled graph",
            )]);
        };
        let is_source = registry
            .get(&node.builder)
            .is_some_and(RuntimeBuilder::is_time_domain_source);
        if !is_source {
            return Err(vec![CompileError::on(
                source_node,
                "source override does not target a source node",
            )]);
        }
    }

    for id in topo_order(&execution) {
        let node = compiled_node(&execution, id);
        let builder = registry.get(&node.builder).ok_or_else(|| {
            vec![CompileError::on(
                id,
                format!("unknown builder '{}'", node.builder),
            )]
        })?;
        ctx.derived_word_caches
            .clone_from(&node.derived_word_caches);
        let process = if let Some(process) = source_overrides.remove(&id) {
            process
        } else {
            materialize_compiled_node(node, builder, &node.runtime_name, registry, ctx)
                .map_err(|message| vec![CompileError::on(id, message)])?
        };
        let inputs = input_subs(&execution, id, process.as_ref(), &names)
            .map_err(|message| vec![CompileError::on(id, message)])?;
        manager
            .add_node_deferred(signal_processing::NodeSpec {
                name: node.runtime_name.clone(),
                node: process,
                inputs,
            })
            .map_err(|message| vec![CompileError::on(id, message)])?;
        names.insert(id, node.runtime_name.clone());
    }
    // All initial subscriptions exist; only now may threads start (a
    // self-threading source snapshots its subscriber lists on first work()).
    manager
        .start_all_deferred()
        .map_err(|message| vec![CompileError::global(message)])?;
    publish_materialized_source_readiness(&compiled, registry, &ctx.source_readiness);

    Ok(LiveRun {
        manager,
        compiled,
        names,
        lanes: ctx.derived_lanes.clone(),
        collected_output_subscriptions: ctx.collected_output_subscriptions.clone(),
        collected_table_subscriptions: ctx.collected_table_subscriptions.clone(),
        diagnostics: ctx.diagnostics.clone(),
        source_readiness: ctx.source_readiness.clone(),
        stop_requested: false,
        cache_pruned,
        timeline_markers: ctx.timeline_markers.clone(),
        work_executor: Arc::clone(&ctx.work_executor),
        artifact_repository: Arc::clone(&ctx.artifact_repository),
    })
}

impl LiveRun {
    pub fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &self.compiled.sampling_overlays
    }

    pub fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        self.compiled
            .nodes
            .iter()
            .flat_map(|node| node.derived_word_caches.iter().flatten().cloned())
            .collect()
    }

    pub fn collected_output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.collected_output_subscriptions
    }

    pub fn collected_table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.collected_table_subscriptions
    }

    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.lanes
    }

    /// Returns a coherent application-neutral snapshot of this live run.
    pub fn run_data(&self) -> RunData {
        RunData::new(
            self.lanes.clone(),
            self.collected_output_subscriptions.clone(),
            self.collected_table_subscriptions.clone(),
            self.compiled.sampling_overlays.clone(),
            self.diagnostics.clone(),
            self.source_readiness.clone(),
        )
    }

    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }

    /// Diffs the edited graph against what is running and applies the
    /// difference live. On any error the running pipeline is untouched
    /// (edits either fail up front in `diff`, or — for build failures midway
    /// — leave already-applied edits in place and report).
    pub(crate) fn apply_with_subscriptions(
        &mut self,
        graph: &GraphState,
        registry: &BuilderRegistry,
        subscriptions: &OutputSubscriptionPlan,
    ) -> Result<ApplySummary, ApplyError> {
        let mut new = lower_with_subscriptions(graph, registry, subscriptions)
            .map_err(ApplyError::Compile)?;
        reuse_sampling_points(&self.compiled, &mut new);
        cache_policy::configure_repository(&mut new, &self.artifact_repository);
        let edits = diff(&self.compiled, &new, registry).map_err(ApplyError::NeedsFullRestart)?;
        if edits.is_empty() {
            self.collected_output_subscriptions =
                collected_output_subscriptions(&new, registry, subscriptions);
            self.collected_table_subscriptions = collected_table_subscriptions(&new, registry);
            self.compiled = new;
            return Ok(ApplySummary::default());
        }
        if self.cache_pruned {
            return Err(ApplyError::NeedsFullRestart(
                "the running graph reused persistent derived data; stop and rerun to apply edits"
                    .to_string(),
            ));
        }

        let mut ctx = CompileCtx {
            derived_lanes: self.lanes.clone(),
            derived_data_retention: new.derived_data_retention,
            derived_word_caches: Vec::new(),
            sampling_overlays: new.sampling_overlays.clone(),
            sampling_points: sampling_point_map(&new),
            collected_output_subscriptions: collected_output_subscriptions(
                &new,
                registry,
                subscriptions,
            ),
            collected_table_subscriptions: collected_table_subscriptions(&new, registry),
            diagnostics: self.diagnostics.clone(),
            source_readiness: self.source_readiness.clone(),
            timeline_markers: self.timeline_markers.clone(),
            work_executor: Arc::clone(&self.work_executor),
            artifact_repository: Arc::clone(&self.artifact_repository),
        };
        let mut summary = ApplySummary::default();
        for edit in edits {
            match edit {
                LiveEdit::Remove(id) => {
                    if let Some(name) = self.names.remove(&id) {
                        self.manager.remove_node(&name).map_err(ApplyError::Apply)?;
                    }
                    summary.removed += 1;
                }
                LiveEdit::Add(id) => {
                    let node = compiled_node(&new, id);
                    let builder = registry.get(&node.builder).ok_or_else(|| {
                        ApplyError::Apply(format!("no builder '{}'", node.builder))
                    })?;
                    ctx.derived_word_caches
                        .clone_from(&node.derived_word_caches);
                    let process = materialize_compiled_node(
                        node,
                        builder,
                        &node.runtime_name,
                        registry,
                        &mut ctx,
                    )
                    .map_err(ApplyError::Apply)?;
                    let inputs = input_subs(&new, id, process.as_ref(), &self.names)
                        .map_err(ApplyError::Apply)?;
                    self.manager
                        .add_node(signal_processing::NodeSpec {
                            name: node.runtime_name.clone(),
                            node: process,
                            inputs,
                        })
                        .map_err(ApplyError::Apply)?;
                    self.names.insert(id, node.runtime_name.clone());
                    summary.added += 1;
                }
                LiveEdit::Configure(id, config) => {
                    let name = self
                        .names
                        .get(&id)
                        .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0)))?;
                    self.manager
                        .reconfigure(name, config)
                        .map_err(ApplyError::Apply)?;
                    summary.configured += 1;
                }
                LiveEdit::Restart(id) => {
                    let node = compiled_node(&new, id);
                    let name = self
                        .names
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0)))?;
                    let builder = registry.get(&node.builder).ok_or_else(|| {
                        ApplyError::Apply(format!("no builder '{}'", node.builder))
                    })?;
                    ctx.derived_word_caches
                        .clone_from(&node.derived_word_caches);
                    let process =
                        materialize_compiled_node(node, builder, &name, registry, &mut ctx)
                            .map_err(ApplyError::Apply)?;
                    let inputs = input_subs(&new, id, process.as_ref(), &self.names)
                        .map_err(ApplyError::Apply)?;
                    self.manager
                        .restart_node(&name, process, inputs)
                        .map_err(ApplyError::Apply)?;
                    summary.restarted += 1;
                }
            }
        }
        self.collected_output_subscriptions =
            collected_output_subscriptions(&new, registry, subscriptions);
        self.collected_table_subscriptions = collected_table_subscriptions(&new, registry);
        self.compiled = new;
        Ok(summary)
    }

    /// Applies the subset of an edited capture graph that can preserve an
    /// explicit future-only boundary. Phase 13.1 deliberately accepts only
    /// builder-declared hot configuration; structural changes and restarts
    /// remain in the edited graph for the next capture or ordinary Run.
    pub(crate) fn apply_configuration_epoch(
        &mut self,
        graph: &GraphState,
        registry: &BuilderRegistry,
        subscriptions: &OutputSubscriptionPlan,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        let mut new = lower_with_subscriptions(graph, registry, subscriptions)
            .map_err(ApplyError::Compile)?;
        reuse_sampling_points(&self.compiled, &mut new);
        cache_policy::configure_repository(&mut new, &self.artifact_repository);
        let edits = diff(&self.compiled, &new, registry).map_err(ApplyError::NeedsFullRestart)?;
        if edits.is_empty() {
            self.collected_output_subscriptions =
                collected_output_subscriptions(&new, registry, subscriptions);
            self.collected_table_subscriptions = collected_table_subscriptions(&new, registry);
            self.compiled = new;
            return Ok(ApplySummary::default());
        }
        if self.cache_pruned {
            return Err(ApplyError::NeedsFullRestart(
                "the running graph reused persistent derived data; the edit is deferred to the next capture"
                    .to_string(),
            ));
        }
        if let Some(edit) = edits
            .iter()
            .find(|edit| !matches!(edit, LiveEdit::Configure(_, _)))
        {
            let reason = match edit {
                LiveEdit::Add(_) => "node additions",
                LiveEdit::Remove(_) => "node removals",
                LiveEdit::Restart(_) => "node restarts or wiring changes",
                LiveEdit::Configure(_, _) => unreachable!(),
            };
            return Err(ApplyError::NeedsFullRestart(format!(
                "{reason} are deferred to the next capture"
            )));
        }

        // Resolve every target before sending any control message so a
        // missing running node cannot leave a partially scheduled epoch.
        let scheduled: Vec<_> = edits
            .into_iter()
            .map(|edit| match edit {
                LiveEdit::Configure(id, config) => self
                    .names
                    .get(&id)
                    .cloned()
                    .map(|name| (name, config))
                    .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0))),
                _ => unreachable!(),
            })
            .collect::<Result<_, _>>()?;
        let configured = scheduled.len();
        for (name, config) in scheduled {
            self.manager
                .reconfigure_at(&name, config, boundary)
                .map_err(ApplyError::Apply)?;
        }
        self.collected_output_subscriptions =
            collected_output_subscriptions(&new, registry, subscriptions);
        self.collected_table_subscriptions = collected_table_subscriptions(&new, registry);
        self.compiled = new;
        Ok(ApplySummary {
            configured,
            ..ApplySummary::default()
        })
    }

    pub fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    /// Signals the wind-down and returns immediately — never joins node
    /// threads, so it is safe to call from the frame loop (a node may be
    /// mid-`work()` for a while yet; see `PipelineManager::request_stop`).
    /// [`Self::is_finished`] flips once every thread has exited.
    pub fn stop(&mut self) {
        self.stop_requested = true;
        self.manager.request_stop();
    }

    /// True from [`Self::stop`] until the run is dropped — used by the
    /// toolbar to show "Stopping…" while threads finish their current
    /// `work()` call.
    pub fn is_stopping(&self) -> bool {
        self.stop_requested
    }

    /// Drives up to `budget` `work()` calls forward. A no-op on the
    /// threaded native manager (its nodes run themselves); on wasm's
    /// cooperative manager this is what actually advances the run, so the
    /// UI frame loop must call it every frame regardless of target.
    pub fn pump(&mut self, budget: usize) {
        self.manager.pump(budget);
    }

    /// Drives cooperative work without monopolizing an interactive host event loop.
    pub fn pump_for(&mut self, budget: usize, max_duration: std::time::Duration) {
        self.manager.pump_for(budget, max_duration);
    }

    /// Blocks until the run completes naturally (tests / headless).
    pub fn wait(&mut self) {
        self.manager.wait();
    }

    /// Items produced per UI node (sum of `work()` returns), for header
    /// progress display.
    pub fn progress(&self) -> Vec<(NodeId, u64)> {
        let by_name: HashMap<String, u64> = self.manager.progress().into_iter().collect();
        self.names
            .iter()
            .filter_map(|(id, name)| by_name.get(name).map(|items| (*id, *items)))
            .collect()
    }

    /// Consumers dropped by backpressure policy since the last call, mapped
    /// back to UI nodes where possible.
    pub fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        self.manager
            .take_disconnected()
            .into_iter()
            .map(|event| {
                let id = event.consumer.as_ref().and_then(|consumer| {
                    self.names
                        .iter()
                        .find(|(_, name)| *name == consumer)
                        .map(|(id, _)| *id)
                });
                (id, event)
            })
            .collect()
    }

    /// Terminal node failures since the last call, mapped back to UI nodes.
    pub fn take_node_failures(&mut self) -> Vec<(Option<NodeId>, NodeFailure)> {
        self.manager
            .take_failures()
            .into_iter()
            .map(|failure| {
                let id = self
                    .names
                    .iter()
                    .find(|(_, name)| **name == failure.node)
                    .map(|(id, _)| *id);
                (id, failure)
            })
            .collect()
    }
}

pub(crate) fn start_app_run(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<CompileError>> {
    start_live_with_subscriptions(graph, registry, subscriptions, ctx, runtime_factory)
}

/// Starts an ordinary application run while replacing explicitly identified
/// source nodes. Finalized-session replay uses this entry point so lowering
/// cannot invoke the captured provider's discovery or build paths.
pub(crate) fn start_app_run_with_source_overrides_and_subscriptions(
    graph: &GraphState,
    registry: &BuilderRegistry,
    subscriptions: &OutputSubscriptionPlan,
    ctx: &mut CompileCtx,
    overrides: SourceProcessOverrides,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<CompileError>> {
    start_live_inner(
        graph,
        registry,
        subscriptions,
        ctx,
        overrides,
        runtime_factory,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use node_graph::api::{
        AnySocket, BoolSocket, GraphDocumentBuilder, InputDef, IntSocket, NodeDef,
        NodeTypeRegistry, OutputDef,
    };
    use signal_processing::{
        AcquisitionContext, AcquisitionError, AcquisitionResult, CaptureAnalysisChannel,
        CaptureAnalysisSource, CaptureChannelId, CaptureDataDelivery, CaptureProviderCapabilities,
        CaptureSettingCombination, CaptureStoreCursor, CollectedLaneSnapshotRequest, ConfigValue,
        IndexedAnnotationWriter, LiveStoreConfig, PreparedAcquisition, Sample, SamplingPoint,
        TextSample, Trigger, TriggerEditorSchema, TriggerIdentifier, TriggerLogicOperator,
        TriggerPredicate, TriggerStage, Word, WordLaneSnapshot,
    };

    use super::*;
    use crate::derived_cache_backend::{DerivedCacheBackend, DerivedCacheLookup};

    const CONTRACT_SOURCE: &str = "Contract Source";
    const CONTRACT_LIVE_SOURCE: &str = "Contract Live Source";
    const CONTRACT_CONFIGURABLE_SOURCE: &str = "Contract Configurable Source";
    const CONTRACT_TRANSFORM: &str = "Contract Transform";
    const CONTRACT_CONVERSION: &str = "Contract Conversion";
    const CONTRACT_SINK: &str = "Contract Sink";

    #[derive(Default)]
    struct TestDerivedCacheBackend {
        lookups: HashMap<[u8; 32], DerivedCacheLookup>,
    }

    impl TestDerivedCacheBackend {
        fn with_lookup(mut self, key: [u8; 32], lookup: DerivedCacheLookup) -> Self {
            self.lookups.insert(key, lookup);
            self
        }
    }

    impl DerivedCacheBackend for TestDerivedCacheBackend {
        fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup {
            self.lookups
                .get(&config.cache_key)
                .copied()
                .unwrap_or(DerivedCacheLookup::Miss)
        }
    }

    struct ContractSourceDefinition;

    impl NodeDef for ContractSourceDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_SOURCE
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            Vec::new()
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<AnySocket>("Out")]
        }

        fn state() -> Self::State {
            Value::Null
        }
    }

    struct ContractTransformDefinition;

    struct ContractConfigurableSourceDefinition;

    impl NodeDef for ContractConfigurableSourceDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_CONFIGURABLE_SOURCE
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            vec![InputDef::new::<AnySocket>("Configuration")]
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<AnySocket>("Out")]
        }

        fn state() -> Self::State {
            serde_json::json!({ "configured": false })
        }
    }

    struct ContractLiveSourceDefinition;

    impl NodeDef for ContractLiveSourceDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_LIVE_SOURCE
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            Vec::new()
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<AnySocket>("Out")]
        }

        fn state() -> Self::State {
            Value::Null
        }
    }

    impl NodeDef for ContractTransformDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_TRANSFORM
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            vec![InputDef::new::<AnySocket>("In")]
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<AnySocket>("Out")]
        }

        fn state() -> Self::State {
            Value::Null
        }
    }

    struct ContractSinkDefinition;

    struct ContractConversionDefinition;

    impl NodeDef for ContractConversionDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_CONVERSION
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            vec![InputDef::new::<BoolSocket>("In")]
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<IntSocket>("Out")]
        }

        fn state() -> Self::State {
            Value::Null
        }
    }

    impl NodeDef for ContractSinkDefinition {
        type State = Value;

        fn name() -> &'static str {
            CONTRACT_SINK
        }

        fn category() -> &'static str {
            "Compiler Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            vec![InputDef::new::<AnySocket>("In")]
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            Vec::new()
        }

        fn state() -> Self::State {
            Value::Null
        }
    }

    struct ContractBuilder {
        source: bool,
        sink: bool,
        accepted: Option<PortKind>,
        offered: Option<PortKind>,
        retention: DerivedDataRetention,
        hot_config: bool,
        presentation: bool,
    }

    struct ConfigurableSourceBuilder;

    impl RuntimeBuilder for ConfigurableSourceBuilder {
        fn is_source(&self) -> bool {
            true
        }

        fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            vec![PortKind::of::<TextSample>()]
        }

        fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            vec![PortKind::of::<Sample>()]
        }

        fn input_port(
            &self,
            _socket: &Socket,
            _member_index: usize,
            _state: &Value,
            kind: PortKind,
        ) -> Option<String> {
            (kind == PortKind::of::<TextSample>()).then(|| "configuration".to_owned())
        }

        fn output_port(&self, _socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
            (kind == PortKind::of::<Sample>()).then(|| "out".to_owned())
        }

        fn input_required(&self, _socket: &Socket, state: &Value) -> bool {
            !state
                .get("configured")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        }
    }

    impl ContractBuilder {
        fn source(kind: PortKind) -> Self {
            Self {
                source: true,
                sink: false,
                accepted: None,
                offered: Some(kind),
                retention: DerivedDataRetention::Unlimited,
                hot_config: false,
                presentation: false,
            }
        }

        fn finite_source(kind: PortKind) -> Self {
            Self {
                retention: DerivedDataRetention::MaxEntries(
                    signal_processing::DEFAULT_DERIVED_DATA_MAX_ENTRIES,
                ),
                ..Self::source(kind)
            }
        }

        fn transform(accepted: PortKind, offered: PortKind) -> Self {
            Self {
                source: false,
                sink: false,
                accepted: Some(accepted),
                offered: Some(offered),
                retention: DerivedDataRetention::Unlimited,
                hot_config: false,
                presentation: false,
            }
        }

        fn hot_transform(kind: PortKind) -> Self {
            Self {
                hot_config: true,
                ..Self::transform(kind, kind)
            }
        }

        fn presenting_transform(kind: PortKind) -> Self {
            Self {
                presentation: true,
                ..Self::transform(kind, kind)
            }
        }

        fn sink(kind: PortKind) -> Self {
            Self {
                source: false,
                sink: true,
                accepted: Some(kind),
                offered: None,
                retention: DerivedDataRetention::Unlimited,
                hot_config: false,
                presentation: false,
            }
        }
    }

    impl RuntimeBuilder for ContractBuilder {
        fn execution_state(&self, state: &Value) -> Value {
            let mut execution = state.clone();
            if self.presentation
                && let Some(fields) = execution.as_object_mut()
            {
                fields.remove("display_format");
            }
            execution
        }

        fn is_source(&self) -> bool {
            self.source
        }

        fn is_sink(&self) -> bool {
            self.sink
        }

        fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
            self.retention
        }

        fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            self.accepted.into_iter().collect()
        }

        fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            self.offered.into_iter().collect()
        }

        fn input_port(
            &self,
            _socket: &Socket,
            _member_index: usize,
            _state: &Value,
            kind: PortKind,
        ) -> Option<String> {
            (self.accepted == Some(kind)).then(|| "in".to_owned())
        }

        fn output_port(&self, _socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
            (self.offered == Some(kind)).then(|| "out".to_owned())
        }

        fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
            self.accepted.is_some()
        }

        fn lane_presentation(
            &self,
            _socket: &Socket,
            _state: &Value,
        ) -> Option<logic_analyzer_graph_api::node_support::LanePresentationDescriptor> {
            self.presentation.then(|| {
                logic_analyzer_graph_api::node_support::LanePresentationDescriptor::new(
                    "frames",
                    "frame",
                    0,
                    1.0,
                    logic_analyzer_graph_api::node_support::LaneBadgeDescriptor::new(
                        "F",
                        [255, 255, 255],
                    ),
                    "org.logicconduit.compiler-test.frame-renderer/v1",
                )
            })
        }

        fn hot_config(&self, state: &Value) -> Option<NodeConfig> {
            if !self.hot_config {
                return None;
            }
            let mut config = NodeConfig::new();
            config.insert(
                "value".to_owned(),
                ConfigValue::U64(state.get("value")?.as_u64()?),
            );
            Some(config)
        }
    }

    fn contract_pipeline(
        source_kind: PortKind,
        transform_input_kind: PortKind,
        transform_output_kind: PortKind,
        sink_kind: PortKind,
        type_preserving_transform: bool,
    ) -> (
        GraphDocumentBuilder,
        BuilderRegistry,
        NodeId,
        NodeId,
        NodeId,
    ) {
        let mut node_types = NodeTypeRegistry::new();
        node_types.register::<ContractSourceDefinition>();
        node_types.register::<ContractLiveSourceDefinition>();
        node_types.register::<ContractConfigurableSourceDefinition>();
        node_types.register::<ContractTransformDefinition>();
        node_types.register::<ContractConversionDefinition>();
        node_types.register::<ContractSinkDefinition>();

        let mut document = GraphDocumentBuilder::new(node_types);
        let source = document.add_node(CONTRACT_SOURCE).unwrap();
        let transform_name = if type_preserving_transform {
            CONTRACT_TRANSFORM
        } else {
            CONTRACT_CONVERSION
        };
        let transform = document.add_node(transform_name).unwrap();
        let sink = document.add_node(CONTRACT_SINK).unwrap();
        connect_named(&mut document, (source, "Out"), (transform, "In"));
        connect_named(&mut document, (transform, "Out"), (sink, "In"));

        let mut builders = BuilderRegistry::isolated_test();
        builders.insert_test_builder(
            CONTRACT_SOURCE,
            Box::new(ContractBuilder::source(source_kind)),
        );
        builders.insert_test_builder(
            transform_name,
            Box::new(ContractBuilder::transform(
                transform_input_kind,
                transform_output_kind,
            )),
        );
        builders.insert_test_builder(CONTRACT_SINK, Box::new(ContractBuilder::sink(sink_kind)));
        (document, builders, source, transform, sink)
    }

    fn configurable_source_pipeline() -> (GraphDocumentBuilder, BuilderRegistry, NodeId) {
        let mut node_types = NodeTypeRegistry::new();
        node_types.register::<ContractConfigurableSourceDefinition>();
        node_types.register::<ContractSinkDefinition>();
        let mut document = GraphDocumentBuilder::new(node_types);
        let source = document.add_node(CONTRACT_CONFIGURABLE_SOURCE).unwrap();
        let sink = document.add_node(CONTRACT_SINK).unwrap();
        connect_named(&mut document, (source, "Out"), (sink, "In"));

        let mut builders = BuilderRegistry::isolated_test();
        builders.insert_test_builder(
            CONTRACT_CONFIGURABLE_SOURCE,
            Box::new(ConfigurableSourceBuilder),
        );
        builders.insert_test_builder(
            CONTRACT_SINK,
            Box::new(ContractBuilder::sink(PortKind::of::<Sample>())),
        );
        (document, builders, source)
    }

    fn connect_named(
        document: &mut GraphDocumentBuilder,
        from: (NodeId, &str),
        to: (NodeId, &str),
    ) {
        let output = output_index(document, from.0, from.1);
        let input = input_index(document, to.0, to.1);
        document.graph_mut().add_connection(
            SocketId {
                node: from.0,
                index: output,
                direction: SocketDirection::Output,
            },
            SocketId {
                node: to.0,
                index: input,
                direction: SocketDirection::Input,
            },
        );
    }

    fn set_viewer_selection(
        document: &mut GraphDocumentBuilder,
        node: NodeId,
        output: usize,
        selected: bool,
    ) {
        document.graph_mut().nodes.get_mut(&node).unwrap().outputs[output]
            .extensions
            .insert("show_in_view".to_owned(), serde_json::json!(selected));
    }

    fn discover_compiled_live_capture_feature(
        graph: &GraphState,
        compiled: &CompiledGraph,
        builders: &BuilderRegistry,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
        let retained: HashSet<_> = compiled.nodes.iter().map(|node| node.id).collect();
        let subscriptions = test_output_subscriptions(graph, builders);
        discover_live_capture_feature_from(graph, builders, &subscriptions, |node| {
            retained.contains(&node.id)
        })
    }
    #[test]
    fn opaque_connection_contracts_require_an_intersection_when_both_ends_declare_them() {
        assert!(connection_contracts_overlap(&[], &["spi".into()]));
        assert!(connection_contracts_overlap(&["spi".into()], &[]));
        assert!(connection_contracts_overlap(
            &["spi".into(), "uart".into()],
            &["spi".into()]
        ));
        assert!(!connection_contracts_overlap(
            &["uart".into()],
            &["spi".into()]
        ));
    }

    #[test]
    fn payload_subscription_registration_requires_an_adapter_and_records_its_contract() {
        let mut registry = BuilderRegistry::isolated_test();
        registry
            .register_payload::<Sample>("org.logicconduit.compiler-test.sample/v1")
            .unwrap();
        let presentation = DefaultLanePresentationDescriptor::new(
            logic_analyzer_graph_api::node_support::LaneBadgeDescriptor::new("T", [255, 255, 255]),
            "org.logicconduit.compiler-test.renderer/v1",
        );

        assert!(matches!(
            registry.register_payload_subscription_with_request_configurator::<Sample>(
                presentation.clone(),
                Arc::new(|request, _, _, _| request),
                true,
            ),
            Err(PayloadRegistrationError::PayloadHasNoAdapter { .. })
        ));
        registry
            .payloads
            .register_adapter::<Sample>(signal_processing::digital_payload_adapter())
            .unwrap();
        registry
            .register_payload_subscription_with_request_configurator::<Sample>(
                presentation.clone(),
                Arc::new(|request, _, _, _| request),
                true,
            )
            .unwrap();

        assert_eq!(
            registry.subscribable_payload_kinds(),
            [PortKind::of::<Sample>()]
        );
        assert_eq!(
            registry.payload_subscription_presentation(PortKind::of::<Sample>()),
            Some(presentation)
        );
        assert!(registry.payload_uses_persistent_cache(PortKind::of::<Sample>()));
    }

    struct BufferedPluginBuilder;

    struct TriggerOnlyPluginBuilder;

    struct BufferedPluginGraphSourceFactory {
        channels: Arc<[CaptureChannelId]>,
    }

    struct BufferedPluginFeature {
        channels: Arc<[CaptureChannelId]>,
        channel_names: Arc<[String]>,
        capabilities: CaptureProviderCapabilities,
    }

    impl CaptureGraphSourceFactory for BufferedPluginGraphSourceFactory {
        fn create(
            &self,
            cursor: Box<dyn CaptureStoreCursor>,
        ) -> Result<Box<dyn ProcessNode>, String> {
            let channels = self
                .channels
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, channel)| {
                    CaptureAnalysisChannel::separate(
                        channel,
                        format!("ch{index}"),
                        format!("block{index}"),
                    )
                })
                .collect();
            CaptureAnalysisSource::new("buffered-plugin-analysis", cursor, 2_000_000.0, channels)
                .map(|source| Box::new(source) as Box<dyn ProcessNode>)
        }
    }

    impl LiveCaptureFeature for BufferedPluginFeature {
        fn channels(&self) -> &[CaptureChannelId] {
            &self.channels
        }

        fn channel_names(&self) -> &[String] {
            &self.channel_names
        }

        fn sample_rate_hz(&self) -> f64 {
            2_000_000.0
        }

        fn capabilities(&self) -> &CaptureProviderCapabilities {
            &self.capabilities
        }

        fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory> {
            Arc::new(BufferedPluginGraphSourceFactory {
                channels: Arc::clone(&self.channels),
            })
        }

        fn prepare(
            self: Box<Self>,
            _context: AcquisitionContext,
        ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
            Err(AcquisitionError::UnsupportedOperation(
                "capability-only compiler test feature".into(),
            ))
        }
    }

    impl RuntimeBuilder for BufferedPluginBuilder {
        fn is_source(&self) -> bool {
            true
        }

        fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            Vec::new()
        }

        fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            vec![PortKind::of::<Sample>()]
        }

        fn input_port(
            &self,
            _socket: &Socket,
            _member_index: usize,
            _state: &Value,
            _kind: PortKind,
        ) -> Option<String> {
            None
        }

        fn output_port(&self, _socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
            (kind == PortKind::of::<Sample>()).then(|| "out".to_owned())
        }

        fn viewer_channel_origin(&self, _socket: &Socket, _state: &Value) -> Option<usize> {
            Some(0)
        }

        fn capture_presentation(
            &self,
            _state: &Value,
        ) -> Result<Option<CapturePresentation>, String> {
            Ok(Some(CapturePresentation::Channels(vec![(
                0,
                "Channel 0".to_owned(),
            )])))
        }

        fn live_capture_feature(
            &self,
            _state: &Value,
        ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
            let channels: Arc<[CaptureChannelId]> = vec![
                CaptureChannelId::new("pod-a:3"),
                CaptureChannelId::new("pod-q:41"),
                CaptureChannelId::new("aux-bank:9"),
            ]
            .into();
            let capabilities = CaptureProviderCapabilities::new(
                CaptureDataDelivery::BufferedUpload,
                Arc::from([
                    CaptureSettingCombination::new(Arc::clone(&channels), Arc::from([2_000_000]))?,
                    CaptureSettingCombination::new(Arc::clone(&channels), Arc::from([1_000_000]))?,
                ]),
                false,
            )?;
            Ok(Some(Box::new(BufferedPluginFeature {
                channel_names: vec!["Pod A 3".into(), "Pod Q 41".into(), "Aux 9".into()].into(),
                channels,
                capabilities,
            })))
        }

        fn apply_live_capture_edit(
            &self,
            state: &Value,
            edit: &LiveCaptureEdit,
        ) -> Result<Option<Value>, String> {
            match edit {
                LiveCaptureEdit::SetTriggerProgram { program } => Ok(Some(serde_json::json!({
                    "previous_state": state,
                    "received_program": program,
                }))),
                LiveCaptureEdit::SetSimpleTrigger { .. } => Ok(None),
            }
        }

        fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
            false
        }

        fn build(
            &self,
            _name: &str,
            _state: &Value,
            _resolved: &ResolvedInputs,
            _ctx: &mut dyn NodeBuildContext,
        ) -> Result<Box<dyn ProcessNode>, String> {
            Err("capability-only compiler test builder".to_owned())
        }
    }

    impl RuntimeBuilder for TriggerOnlyPluginBuilder {
        fn is_source(&self) -> bool {
            true
        }

        fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            Vec::new()
        }

        fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
            vec![PortKind::of::<Sample>()]
        }

        fn input_port(
            &self,
            _socket: &Socket,
            _member_index: usize,
            _state: &Value,
            _kind: PortKind,
        ) -> Option<String> {
            None
        }

        fn output_port(&self, _socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
            (kind == PortKind::of::<Sample>()).then(|| "out".to_owned())
        }

        fn viewer_channel_origin(&self, _socket: &Socket, _state: &Value) -> Option<usize> {
            Some(0)
        }

        fn live_capture_feature(
            &self,
            _state: &Value,
        ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
            panic!("trigger configuration discovery consulted the acquisition feature")
        }

        fn trigger_configuration(
            &self,
            _state: &Value,
        ) -> Result<Option<TriggerConfigurationFeature>, String> {
            let schema = TriggerEditorSchema::new(
                TriggerIdentifier::new("plugin.trigger-only").unwrap(),
                1,
                1,
                2,
                vec![TriggerLogicOperator::And],
            )
            .unwrap()
            .with_digital_conditions(vec![SimpleTriggerCondition::High])
            .unwrap();
            TriggerConfigurationFeature::new(
                schema,
                None,
                vec![SimpleTriggerChannel {
                    channel_id: CaptureChannelId::new("plugin-bank:23"),
                    viewer_channel: 0,
                    name: "Plugin 23".into(),
                    enabled: true,
                    condition: SimpleTriggerCondition::Ignore,
                }],
            )
            .map(Some)
        }

        fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
            false
        }

        fn build(
            &self,
            _name: &str,
            _state: &Value,
            _resolved: &ResolvedInputs,
            _ctx: &mut dyn NodeBuildContext,
        ) -> Result<Box<dyn ProcessNode>, String> {
            Err("trigger-only compiler test builder".to_owned())
        }
    }

    #[test]
    fn trigger_configuration_discovery_does_not_require_acquisition() {
        let (document, mut builders, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders.builders.insert(
            CONTRACT_SOURCE.to_owned(),
            Box::new(TriggerOnlyPluginBuilder),
        );

        let configuration = discover_trigger_configuration(document.graph(), &builders)
            .unwrap()
            .unwrap();

        assert_eq!(configuration.source_node, source);
        assert_eq!(
            configuration.feature.schema().id().as_str(),
            "plugin.trigger-only"
        );
        assert_eq!(
            configuration.feature.channels()[0].channel_id.as_str(),
            "plugin-bank:23"
        );
    }

    #[test]
    fn discovery_rejects_multiple_live_capture_features() {
        let (mut document, mut builders, _source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders.builders.insert(
            CONTRACT_LIVE_SOURCE.to_owned(),
            Box::new(BufferedPluginBuilder),
        );
        let first = document.add_node(CONTRACT_LIVE_SOURCE).unwrap();
        let second = document.add_node(CONTRACT_LIVE_SOURCE).unwrap();

        let error = discover_live_capture_feature(document.graph(), &builders)
            .err()
            .unwrap();
        assert_eq!(error.source_nodes, [first, second]);
        assert!(error.message.contains("multiple"));
    }

    #[test]
    fn compiled_discovery_ignores_a_disconnected_live_feature() {
        let (mut document, mut builders, _source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders.builders.insert(
            CONTRACT_LIVE_SOURCE.to_owned(),
            Box::new(BufferedPluginBuilder),
        );
        document.add_node(CONTRACT_LIVE_SOURCE).unwrap();
        let compiled = lower(document.graph(), &builders).unwrap();

        assert!(
            discover_compiled_live_capture_feature(document.graph(), &compiled, &builders)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unchanged_live_lowering_reuses_runtime_sampling_points() {
        let points = SamplingPointStore::default();
        points
            .record(SamplingPoint::new(100, true, vec![false]))
            .unwrap();
        let old = CompiledGraph {
            sampling_overlays: vec![SamplingOverlayCandidate {
                node_id: NodeId(7),
                node_title: "Contract sampler".to_owned(),
                overlay: ResolvedSamplingOverlay {
                    clock_channel: 0,
                    sampled_channels: vec![1],
                    points,
                },
                cache_key: None,
                retained_word_lane: None,
            }],
            ..CompiledGraph::default()
        };

        let mut new = CompiledGraph {
            sampling_overlays: vec![SamplingOverlayCandidate {
                node_id: NodeId(7),
                node_title: "Contract sampler".to_owned(),
                overlay: ResolvedSamplingOverlay {
                    clock_channel: 0,
                    sampled_channels: vec![1],
                    points: SamplingPointStore::default(),
                },
                cache_key: None,
                retained_word_lane: None,
            }],
            ..CompiledGraph::default()
        };
        reuse_sampling_points(&old, &mut new);
        assert_eq!(
            new.sampling_overlays[0]
                .overlay
                .points
                .points_in_range(0, u64::MAX),
            [SamplingPoint::new(100, true, vec![false])]
        );
    }

    /// A lone source node with no explicit sink, used to verify that source
    /// presentation remains independent of selectable waveform presentations.
    fn source_only_contract() -> (GraphDocumentBuilder, BuilderRegistry) {
        let mut node_types = NodeTypeRegistry::new();
        node_types.register::<ContractSourceDefinition>();
        let mut document = GraphDocumentBuilder::new(node_types);
        document
            .add_node(CONTRACT_SOURCE)
            .expect("contract source is registered");
        let mut builders = BuilderRegistry::isolated_test();
        builders.insert_test_builder(CONTRACT_SOURCE, Box::new(BufferedPluginBuilder));
        (document, builders)
    }

    fn hide_first_output(document: &mut GraphDocumentBuilder) -> NodeId {
        let id = *document.graph().nodes.keys().next().unwrap();
        set_viewer_selection(document, id, 0, false);
        id
    }

    fn selectable_output_contract() -> (GraphDocumentBuilder, BuilderRegistry, NodeId) {
        selectable_output_contract_for::<Sample>(
            "org.logicconduit.compiler-test.sample/v1",
            signal_processing::digital_payload_adapter(),
            false,
        )
    }

    fn selectable_word_output_contract() -> (GraphDocumentBuilder, BuilderRegistry, NodeId) {
        selectable_output_contract_for::<Word>(
            "org.logicconduit.compiler-test.word/v1",
            signal_processing::word_payload_adapter(),
            true,
        )
    }

    fn selectable_output_contract_for<T: logic_analyzer_graph_api::node_support::PortValue>(
        stable_id: &str,
        adapter: Arc<dyn signal_processing::PayloadAdapter>,
        persistent_cache: bool,
    ) -> (GraphDocumentBuilder, BuilderRegistry, NodeId) {
        let kind = PortKind::of::<T>();
        let (mut document, mut registry, _source, transform, _sink) =
            contract_pipeline(kind, kind, kind, kind, true);
        registry.register_payload::<T>(stable_id).unwrap();
        registry.payloads.register_adapter::<T>(adapter).unwrap();
        let configure_request: PayloadRequestConfigurator = if persistent_cache {
            Arc::new(|request, member, _input, context| {
                let store_config = context.derived_word_cache(member).map_or_else(
                    LiveStoreConfig::default,
                    |persistent| LiveStoreConfig {
                        persistence: Some(persistent.clone()),
                        ..LiveStoreConfig::default()
                    },
                );
                request.with_options(signal_processing::CollectedWordLaneOptions::new(
                    store_config
                        .with_work_executor(context.work_executor())
                        .with_artifact_repository(context.artifact_repository()),
                    None,
                ))
            })
        } else {
            Arc::new(|request, _member, _input, _context| request)
        };
        registry
            .register_payload_subscription_with_request_configurator::<T>(
                DefaultLanePresentationDescriptor::new(
                    logic_analyzer_graph_api::node_support::LaneBadgeDescriptor::new(
                        "T",
                        [255, 255, 255],
                    ),
                    "org.logicconduit.compiler-test.renderer/v1",
                ),
                configure_request,
                persistent_cache,
            )
            .unwrap();
        registry.insert_test_builder(
            crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME,
            Box::new(DataCollectorBuilder::output_subscription()),
        );
        set_viewer_selection(&mut document, transform, 0, true);
        (document, registry, transform)
    }

    fn first_watched_selectable_output(
        document: &GraphDocumentBuilder,
        registry: &BuilderRegistry,
    ) -> (NodeId, usize) {
        test_output_subscriptions(document.graph(), registry)
            .outputs()
            .find(|(node_id, output)| {
                let node = &document.graph().nodes[node_id];
                registry.get(node.def_name()).is_some_and(|builder| {
                    builder
                        .viewer_channel_origin(&node.outputs[*output], &node.state)
                        .is_none()
                })
            })
            .expect("test graph has a watched selectable output")
    }

    #[test]
    fn unwatched_source_has_no_sink() {
        let (mut document, builders) = source_only_contract();
        hide_first_output(&mut document);
        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(errors.iter().any(|e| e.message.contains("no sink")));
    }

    #[test]
    fn raw_source_output_is_visible_by_default_without_becoming_a_derived_sink() {
        let (document, builders) = source_only_contract();
        let source = document.graph().nodes.values().next().unwrap();
        assert!(
            test_output_subscriptions(document.graph(), &builders)
                .outputs()
                .any(|(node, _)| node == source.id)
        );

        let presentation = discover_capture_presentation(document.graph(), &builders)
            .unwrap()
            .expect("source provides a capture presentation");
        assert_eq!(presentation.visible_channels, [0]);

        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(errors.iter().any(|error| error.message.contains("no sink")));
    }

    #[test]
    fn hiding_an_output_keeps_its_connected_data_cached() {
        let (mut document, registry, node_id) = selectable_output_contract();
        let (_, output_index) = first_watched_selectable_output(&document, &registry);
        let watched = lower(document.graph(), &registry).unwrap();
        assert!(watched.nodes.iter().any(|node| {
            node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME && node.data_collector
        }));

        set_viewer_selection(&mut document, node_id, output_index, false);
        let compiled = lower(document.graph(), &registry).unwrap();
        assert!(
            compiled
                .nodes
                .iter()
                .any(|node| node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
        );
        assert!(diff(&watched, &compiled, &registry).unwrap().is_empty());
        let subscriptions = test_output_subscriptions(document.graph(), &registry);
        assert!(collected_output_subscriptions(&compiled, &registry, &subscriptions).is_empty());
    }

    #[test]
    fn retained_output_visibility_is_a_metadata_only_change() {
        let (mut document, registry, producer) = selectable_output_contract();
        let sink = node_by_def(&document, CONTRACT_SINK);
        document.graph_mut().remove_node(sink);

        let mut visible = OutputSubscriptionPlan::new();
        visible.subscribe(producer, 0);
        let compiled_visible =
            lower_with_subscriptions(document.graph(), &registry, &visible).unwrap();
        assert_eq!(
            collected_output_subscriptions(&compiled_visible, &registry, &visible).len(),
            1
        );

        let mut hidden = OutputSubscriptionPlan::new();
        hidden.retain(producer, 0);
        let compiled_hidden =
            lower_with_subscriptions(document.graph(), &registry, &hidden).unwrap();

        assert!(
            diff(&compiled_visible, &compiled_hidden, &registry)
                .unwrap()
                .is_empty()
        );
        assert!(collected_output_subscriptions(&compiled_hidden, &registry, &hidden).is_empty());
    }

    #[test]
    fn output_subscription_collector_id_is_stable_across_relowers() {
        let (document, registry, _selected) = selectable_output_contract();

        let first = lower(document.graph(), &registry).unwrap();
        let second = lower(document.graph(), &registry).unwrap();
        let viewer_id = |compiled: &CompiledGraph| {
            compiled
                .nodes
                .iter()
                .find(|n| n.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
                .unwrap()
                .id
        };
        assert_eq!(viewer_id(&first), viewer_id(&second));
    }

    #[test]
    fn output_subscription_collector_is_a_generic_runtime_sink() {
        let (document, registry, _selected) = selectable_output_contract();
        let builder = registry
            .get(crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
            .unwrap();
        assert!(builder.is_sink());
        assert!(builder.is_data_collector());
        assert!(builder.is_data_subscription());

        let compiled = lower(document.graph(), &registry).unwrap();
        let viewer = compiled
            .nodes
            .iter()
            .find(|node| node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
            .unwrap();
        assert!(viewer.data_collector, "lowering must plan retained storage");

        let mut ctx = CompileCtx::default();
        let process =
            materialize_compiled_node(viewer, builder, &viewer.runtime_name, &registry, &mut ctx)
                .unwrap();
        assert_eq!(process.num_inputs(), viewer.resolved.member_count(0));
        assert_eq!(process.num_outputs(), 0);
    }

    fn persistent_word_keys(compiled: &CompiledGraph) -> Vec<[u8; 32]> {
        compiled
            .nodes
            .iter()
            .flat_map(|node| node.derived_word_caches.iter().flatten())
            .map(|config| config.cache_key)
            .collect()
    }

    #[test]
    fn persistent_derived_lane_key_is_stable_but_producer_configuration_invalidates_it() {
        let (mut document, registry, producer) = selectable_word_output_contract();
        let first = lower(document.graph(), &registry).unwrap();
        let repeated = lower(document.graph(), &registry).unwrap();
        let first_keys = persistent_word_keys(&first);
        assert!(!first_keys.is_empty());
        assert_eq!(first_keys, persistent_word_keys(&repeated));

        document.set_node_state(producer, serde_json::json!({ "revision": 2 }));
        let changed = lower(document.graph(), &registry).unwrap();
        assert_ne!(first_keys, persistent_word_keys(&changed));
    }

    #[test]
    fn cache_inventory_maps_a_lane_to_its_collector_and_upstream_nodes() {
        let (document, registry, producer) = selectable_word_output_contract();
        let compiled = lower(document.graph(), &registry).unwrap();
        let collector = compiled
            .nodes
            .iter()
            .find(|node| node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
            .unwrap();
        let expected: Vec<_> = collector
            .derived_word_caches
            .iter()
            .flatten()
            .map(|config| config.cache_key)
            .collect();

        let inventory = derived_cache_configs_by_node(document.graph(), &registry).unwrap();
        let actual = inventory[&collector.id]
            .iter()
            .map(|config| config.cache_key)
            .collect::<Vec<_>>();
        assert!(!expected.is_empty());
        assert!(expected.iter().all(|key| actual.contains(key)));
        let producer_keys = inventory[&producer]
            .iter()
            .map(|config| config.cache_key)
            .collect::<Vec<_>>();
        assert!(expected.iter().all(|key| producer_keys.contains(key)));
    }

    #[test]
    fn persistent_derived_lane_key_includes_variadic_member_order() {
        let (document, registry, _producer) = selectable_word_output_contract();
        let compiled = lower(document.graph(), &registry).unwrap();
        let collector = compiled
            .nodes
            .iter()
            .find(|node| node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME)
            .unwrap();
        let edge = compiled
            .edges
            .iter()
            .find(|edge| edge.to.0 == collector.id && edge.kind == PortKind::of::<Word>())
            .unwrap();
        assert_ne!(
            cache_policy::persistent_lane_key(&compiled, collector.id, 0, edge),
            cache_policy::persistent_lane_key(&compiled, collector.id, 1, edge)
        );
    }

    #[test]
    fn persistent_cache_hit_prunes_producer_used_only_by_cached_derived_lane() {
        let (mut document, registry, producer) = selectable_word_output_contract();
        let explicit_sink = node_by_def(&document, CONTRACT_SINK);
        document.graph_mut().remove_node(explicit_sink);

        let compiled = lower(document.graph(), &registry).unwrap();
        let caches = compiled
            .nodes
            .iter()
            .filter(|node| node.data_collector)
            .flat_map(|node| node.derived_word_caches.iter().flatten().cloned())
            .collect::<Vec<_>>();
        assert!(!caches.is_empty());
        let backend = caches
            .iter()
            .fold(TestDerivedCacheBackend::default(), |backend, config| {
                backend.with_lookup(config.cache_key, DerivedCacheLookup::Hit)
            });

        let (execution, pruned) =
            cache_policy::prepare_execution_with_backend(&compiled, &registry, &backend);

        assert!(pruned);
        assert!(execution.nodes.iter().all(|node| node.id != producer));
        assert!(
            execution
                .nodes
                .iter()
                .any(|node| { node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME })
        );
        assert!(
            execution
                .edges
                .iter()
                .all(|edge| edge.kind != PortKind::of::<Word>())
        );
    }

    #[test]
    fn cached_preview_materializes_only_hit_collectors_without_executable_edges() {
        let (document, registry, producer) = selectable_word_output_contract();
        let explicit_sink = node_by_def(&document, CONTRACT_SINK);
        let compiled = lower(document.graph(), &registry).unwrap();
        let caches = compiled
            .nodes
            .iter()
            .filter(|node| node.data_collector)
            .flat_map(|node| node.derived_word_caches.iter().flatten().cloned())
            .collect::<Vec<_>>();
        let hit = caches
            .first()
            .expect("fixture must define a persistent lane");
        let backend =
            TestDerivedCacheBackend::default().with_lookup(hit.cache_key, DerivedCacheLookup::Hit);

        let preview = cache_policy::prepare_cached_preview_with_backend(&compiled, &backend)
            .expect("one cache hit should produce a preview");

        assert!(preview.edges.is_empty());
        assert!(preview.nodes.iter().all(|node| node.data_collector));
        assert!(preview.nodes.iter().all(|node| node.id != producer));
        assert!(preview.nodes.iter().all(|node| node.id != explicit_sink));
        let preview_caches = preview
            .nodes
            .iter()
            .flat_map(|node| node.derived_word_caches.iter().flatten())
            .collect::<Vec<_>>();
        assert_eq!(preview_caches.len(), 1);
        assert_eq!(preview_caches[0].cache_key, hit.cache_key);
        let preview_inputs = preview
            .nodes
            .iter()
            .flat_map(|node| node.resolved.members(0))
            .collect::<Vec<_>>();
        assert_eq!(preview_inputs.len(), 1);
        assert_eq!(preview_inputs[0].0, 0);
    }

    #[test]
    fn cached_data_is_published_without_starting_a_graph() {
        let (document, registry, _producer) = selectable_word_output_contract();
        let subscriptions = test_output_subscriptions(document.graph(), &registry);
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let mut compiled =
            lower_with_subscriptions(document.graph(), &registry, &subscriptions).unwrap();
        cache_policy::configure_repository(&mut compiled, &repository);
        let persistent = compiled
            .nodes
            .iter()
            .filter(|node| node.data_collector)
            .flat_map(|node| node.derived_word_caches.iter().flatten())
            .next()
            .expect("fixture must define a persistent lane")
            .clone();
        let store_config = LiveStoreConfig {
            persistence: Some(persistent),
            ..LiveStoreConfig::default()
        };
        let (mut writer, _store) = IndexedAnnotationWriter::create(store_config).unwrap();
        writer
            .append_batch(&[Word::spanning(0x42, 100, 20)])
            .unwrap();
        writer.finish().unwrap();

        let mut context = CompileCtx::default();
        context.set_artifact_repository(repository);
        assert!(
            load_cached_data_with_subscriptions(
                document.graph(),
                &registry,
                &subscriptions,
                &mut context,
            )
            .unwrap()
        );
        let lanes = context.derived_lanes().opaque_lanes();
        let annotations = lanes
            .iter()
            .find_map(|lane| {
                lane.snapshot(CollectedLaneSnapshotRequest {
                    start_time_ns: 0,
                    end_time_ns: 1_000,
                    max_items: 16,
                })
                .and_then(|snapshot| snapshot.value::<WordLaneSnapshot>())
                .and_then(|snapshot| match snapshot.as_ref() {
                    WordLaneSnapshot::Exact { annotations, .. } if !annotations.is_empty() => {
                        Some(annotations.clone())
                    }
                    _ => None,
                })
            })
            .expect("cached word lane must publish its exact data");
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].value, 0x42);
    }

    #[test]
    fn missing_or_unreadable_persistent_caches_keep_the_producer_connected() {
        let (mut document, registry, producer) = selectable_word_output_contract();
        let explicit_sink = node_by_def(&document, CONTRACT_SINK);
        document.graph_mut().remove_node(explicit_sink);
        let compiled = lower(document.graph(), &registry).unwrap();
        let cache_keys = compiled
            .nodes
            .iter()
            .filter(|node| node.data_collector)
            .flat_map(|node| node.derived_word_caches.iter().flatten())
            .map(|config| config.cache_key)
            .collect::<Vec<_>>();
        let backend = cache_keys.iter().enumerate().fold(
            TestDerivedCacheBackend::default(),
            |backend, (index, key)| {
                backend.with_lookup(
                    *key,
                    if index == 0 {
                        DerivedCacheLookup::Unreadable
                    } else {
                        DerivedCacheLookup::Miss
                    },
                )
            },
        );

        let (execution, pruned) =
            cache_policy::prepare_execution_with_backend(&compiled, &registry, &backend);

        assert!(!pruned);
        assert!(execution.nodes.iter().any(|node| node.id == producer));
        assert!(
            execution
                .edges
                .iter()
                .any(|edge| edge.kind == PortKind::of::<Word>())
        );
    }

    #[test]
    fn duplicate_and_renamed_producers_keep_distinct_explicit_groups() {
        let (mut document, mut builders, first_producer) = selectable_output_contract();
        builders.insert_test_builder(
            CONTRACT_TRANSFORM,
            Box::new(ContractBuilder::presenting_transform(
                PortKind::of::<Sample>(),
            )),
        );
        let source = node_by_def(&document, CONTRACT_SOURCE);
        let second_producer = document.add_node(CONTRACT_TRANSFORM).unwrap();
        connect_named(&mut document, (source, "Out"), (second_producer, "In"));
        set_viewer_selection(&mut document, second_producer, 0, true);
        for producer in [first_producer, second_producer] {
            document.graph_mut().nodes.get_mut(&producer).unwrap().title = "Duplicate title".into();
        }

        let build_groups = |document: &GraphDocumentBuilder| {
            let compiled = lower(document.graph(), &builders).unwrap();
            let subscriptions = test_output_subscriptions(document.graph(), &builders);
            let mut groups = collected_output_subscriptions(&compiled, &builders, &subscriptions)
                .iter()
                .flat_map(|subscription| &subscription.lanes)
                .filter_map(|lane| {
                    lane.input
                        .lane_presentation
                        .as_ref()
                        .and_then(|presentation| {
                            (presentation.track_key == "frame").then(|| {
                                (
                                    (lane.input.source_node, presentation.group_key.clone()),
                                    lane.source_label.clone(),
                                )
                            })
                        })
                })
                .collect::<Vec<_>>();
            groups.sort_by_key(|((node, key), _)| (node.0, key.clone()));
            groups.dedup();
            groups
        };

        let before = build_groups(&document);
        assert_eq!(before.len(), 2);
        assert_ne!(before[0].0, before[1].0);
        assert!(before.iter().all(|(_, label)| label == "Duplicate title"));

        document
            .graph_mut()
            .nodes
            .get_mut(&first_producer)
            .unwrap()
            .title = "Renamed producer".into();
        let after = build_groups(&document);
        assert_eq!(
            before.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            after.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );
        assert!(after.iter().any(|(_, label)| label == "Renamed producer"));
    }

    #[test]
    fn plugin_builder_can_contribute_lane_presentation_metadata() {
        use logic_analyzer_graph_api::node_support::{
            LaneBadgeDescriptor, LanePresentationDescriptor,
        };

        struct PluginBuilder;
        impl RuntimeBuilder for PluginBuilder {
            fn accepted_kinds(&self, _: &Socket, _: &Value) -> Vec<PortKind> {
                Vec::new()
            }

            fn offered_kinds(&self, _: &Socket, _: &Value) -> Vec<PortKind> {
                Vec::new()
            }

            fn input_port(&self, _: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
                None
            }

            fn output_port(&self, _: &Socket, _: &Value, _: PortKind) -> Option<String> {
                None
            }

            fn lane_presentation(
                &self,
                _: &Socket,
                _: &Value,
            ) -> Option<LanePresentationDescriptor> {
                Some(LanePresentationDescriptor::new(
                    "plugin group",
                    "plugin track",
                    0,
                    1.0,
                    LaneBadgeDescriptor::new("P", [255, 255, 255]),
                    "org.example.renderer/v1",
                ))
            }

            fn build(
                &self,
                _: &str,
                _: &Value,
                _: &ResolvedInputs,
                _: &mut dyn NodeBuildContext,
            ) -> Result<Box<dyn ProcessNode>, String> {
                Err("not needed by presentation registration test".into())
            }
        }

        let (document, mut builders, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders
            .builders
            .insert("Plugin Presenter".into(), Box::new(PluginBuilder));
        let socket = &document.graph().nodes[&source].outputs[0];
        let presentation = builders
            .get("Plugin Presenter")
            .unwrap()
            .lane_presentation(socket, &Value::Null)
            .unwrap();

        assert_eq!(presentation.group_key, "plugin group");
        assert_eq!(presentation.track_key, "plugin track");
    }

    #[test]
    fn buffered_provider_registers_through_the_existing_live_feature_contract() {
        let (document, mut builders, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders
            .builders
            .insert(CONTRACT_SOURCE.to_owned(), Box::new(BufferedPluginBuilder));

        let feature = discover_live_capture_feature(document.graph(), &builders)
            .unwrap()
            .expect("registered builder should expose its live feature");

        assert_eq!(feature.source_node, source);
        assert_eq!(
            feature.capabilities().data_delivery(),
            CaptureDataDelivery::BufferedUpload
        );
        assert_eq!(
            feature.channels(),
            [
                CaptureChannelId::new("pod-a:3"),
                CaptureChannelId::new("pod-q:41"),
                CaptureChannelId::new("aux-bank:9"),
            ]
        );
        assert_eq!(feature.capabilities().setting_matrix().len(), 2);
        assert!(
            feature
                .capabilities()
                .supports(feature.channels(), feature.sample_rate_hz())
        );
        assert!(!feature.capabilities().supports_force_trigger());
    }

    #[test]
    fn advanced_trigger_program_routes_unchanged_to_the_registered_builder() {
        let (document, mut builders, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders
            .builders
            .insert(CONTRACT_SOURCE.to_owned(), Box::new(BufferedPluginBuilder));
        let program = TriggerProgram::new(
            TriggerIdentifier::new("plugin.vendor-neutral.engine").unwrap(),
            17,
            vec![TriggerStage {
                predicates: vec![TriggerPredicate::Digital {
                    channel: CaptureChannelId::new("pod-q:41"),
                    condition: SimpleTriggerCondition::Falling,
                }],
                logic: TriggerLogicOperator::And,
                inverted: false,
                count: None,
            }],
        );

        let state = apply_live_capture_edit(
            document.graph(),
            &builders,
            source,
            &LiveCaptureEdit::SetTriggerProgram {
                program: Some(program.clone()),
            },
        )
        .unwrap();

        assert_eq!(
            state["received_program"],
            serde_json::to_value(Some(program)).unwrap()
        );
        assert_eq!(
            state["previous_state"],
            document.graph().nodes[&source].state
        );
    }

    #[test]
    fn file_source_bounds_exact_derived_data_entries() {
        let (document, mut builders, _source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        builders.insert_test_builder(
            CONTRACT_SOURCE,
            Box::new(ContractBuilder::finite_source(PortKind::of::<Sample>())),
        );
        let compiled = lower(document.graph(), &builders)
            .unwrap_or_else(|errors| panic!("lower failed: {errors:?}"));

        assert_eq!(
            compiled.derived_data_retention,
            DerivedDataRetention::MaxEntries(signal_processing::DEFAULT_DERIVED_DATA_MAX_ENTRIES)
        );
    }

    #[test]
    fn missing_required_configuration_input_is_reported() {
        let (document, builders, source) = configurable_source_pipeline();

        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.node == Some(source) && error.message.contains("Configuration")),
            "expected configuration-input error, got {errors:?}"
        );
    }

    #[test]
    fn builder_state_can_make_configuration_input_optional() {
        let (mut document, builders, source) = configurable_source_pipeline();
        document.set_node_state(source, serde_json::json!({ "configured": true }));

        lower(document.graph(), &builders)
            .unwrap_or_else(|errors| panic!("expected the graph to compile: {errors:?}"));
    }

    #[test]
    fn connected_payload_kind_mismatch_is_rejected() {
        let (document, builders, _source, transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Trigger>(),
            PortKind::of::<Trigger>(),
            PortKind::of::<Trigger>(),
            true,
        );

        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(
            errors.iter().any(|error| error.node == Some(transform)),
            "expected a compile error on the transform node, got {errors:?}"
        );
    }

    #[test]
    fn muted_node_with_compatible_pass_through_lowers_to_a_direct_connection() {
        let (mut document, builders, source, transform, sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        document
            .graph_mut()
            .nodes
            .get_mut(&transform)
            .unwrap()
            .muted = true;

        let compiled = lower(document.graph(), &builders)
            .unwrap_or_else(|errors| panic!("expected the muted node to splice: {errors:?}"));

        assert!(
            compiled.nodes.iter().all(|node| node.id != transform),
            "muted node must be dropped from the compiled graph, got {:?}",
            compiled.nodes
        );
        assert_eq!(compiled.edges.len(), 1);
        let edge = &compiled.edges[0];
        assert_eq!(edge.from.0, source);
        assert_eq!(edge.to.0, sink);
    }

    #[test]
    fn muted_node_without_compatible_pass_through_reports_a_targeted_error() {
        let (mut document, builders, _source, transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Trigger>(),
            PortKind::of::<Trigger>(),
            false,
        );
        document
            .graph_mut()
            .nodes
            .get_mut(&transform)
            .unwrap()
            .muted = true;

        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.node == Some(transform) && error.message.contains("Muted")),
            "expected a targeted error on the muted transform, got {errors:?}"
        );
    }

    #[test]
    fn muted_source_reports_the_break_and_prunes_its_branch() {
        let (mut document, builders, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        document.graph_mut().nodes.get_mut(&source).unwrap().muted = true;

        let errors = lower(document.graph(), &builders).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.node == Some(source) && e.message.contains("Muted")),
            "expected a targeted error on the muted source, got {errors:?}"
        );
    }

    fn output_index(document: &GraphDocumentBuilder, node: NodeId, name: &str) -> usize {
        document.graph().nodes[&node]
            .outputs
            .iter()
            .position(|socket| socket.name == name)
            .unwrap_or_else(|| panic!("no output socket '{name}'"))
    }

    fn input_index(document: &GraphDocumentBuilder, node: NodeId, name: &str) -> usize {
        document.graph().nodes[&node]
            .inputs
            .iter()
            .position(|socket| socket.name == name && socket.visible)
            .unwrap_or_else(|| panic!("no input socket '{name}'"))
    }

    fn node_by_def(document: &GraphDocumentBuilder, def: &str) -> NodeId {
        document
            .graph()
            .nodes
            .values()
            .find(|node| node.def_name() == def)
            .unwrap_or_else(|| panic!("no '{def}' node"))
            .id
    }

    // ── diff classification ───────────────────────────────────────────

    #[test]
    fn diff_classifies_builder_owned_state_change_as_hot_config() {
        let (mut document, mut registry, _source, transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        registry.insert_test_builder(
            CONTRACT_TRANSFORM,
            Box::new(ContractBuilder::hot_transform(PortKind::of::<Sample>())),
        );
        document.set_node_state(transform, serde_json::json!({ "value": 1 }));
        let old = lower(document.graph(), &registry).unwrap();

        document.set_node_state(transform, serde_json::json!({ "value": 0x600082 }));

        let new = lower(document.graph(), &registry).unwrap();
        let edits = diff(&old, &new, &registry).unwrap();
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            LiveEdit::Configure(id, config) => {
                assert_eq!(*id, transform);
                assert_eq!(config.get("value"), Some(&ConfigValue::U64(0x600082)));
            }
            other => panic!("expected Configure, got {other:?}"),
        }
    }

    #[test]
    fn diff_ignores_builder_declared_presentation_only_state() {
        let (mut document, mut registry, _source, transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        registry.insert_test_builder(
            CONTRACT_TRANSFORM,
            Box::new(ContractBuilder::presenting_transform(
                PortKind::of::<Sample>(),
            )),
        );
        document.set_node_state(
            transform,
            serde_json::json!({ "runtime": 1, "display_format": "Hex" }),
        );
        let old = lower(document.graph(), &registry).unwrap();

        document.set_node_state(
            transform,
            serde_json::json!({ "runtime": 1, "display_format": "Binary" }),
        );

        let new = lower(document.graph(), &registry).unwrap();
        assert!(diff(&old, &new, &registry).unwrap().is_empty());
    }

    #[test]
    fn diff_rejects_source_fed_restart() {
        let (mut document, registry, _source, transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        let old = lower(document.graph(), &registry).unwrap();

        document.set_node_state(transform, serde_json::json!({ "revision": 2 }));

        let new = lower(document.graph(), &registry).unwrap();
        let error = diff(&old, &new, &registry).unwrap_err();
        assert!(error.contains("fed directly by the source"), "{error}");
    }

    #[test]
    fn diff_classifies_tap_attach_as_adds_without_restarting_existing_caches() {
        let (mut document, registry, existing_transform) = selectable_output_contract();
        let old = lower(document.graph(), &registry).unwrap();

        let tap = document.add_node(CONTRACT_TRANSFORM).unwrap();
        connect_named(&mut document, (existing_transform, "Out"), (tap, "In"));
        set_viewer_selection(&mut document, tap, 0, true);
        let new = lower(document.graph(), &registry).unwrap();
        let edits = diff(&old, &new, &registry).unwrap();

        assert!(
            edits
                .iter()
                .any(|edit| matches!(edit, LiveEdit::Add(id) if *id == tap)),
            "{edits:?}"
        );
        let added_collector = new
            .nodes
            .iter()
            .find(|node| {
                node.builder == crate::OUTPUT_SUBSCRIPTION_BUILDER_NAME
                    && !old.nodes.iter().any(|old_node| old_node.id == node.id)
            })
            .expect("the new producer has its own retained-output collector")
            .id;
        assert!(
            edits
                .iter()
                .any(|edit| matches!(edit, LiveEdit::Add(id) if *id == added_collector))
        );
        assert!(
            edits
                .iter()
                .all(|edit| !matches!(edit, LiveEdit::Restart(_))),
            "{edits:?}"
        );
        assert_eq!(edits.len(), 2, "{edits:?}");
    }

    #[test]
    fn diff_ignores_a_legacy_source_view_flag() {
        let (mut document, mut registry, source, _transform, _sink) = contract_pipeline(
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            PortKind::of::<Sample>(),
            true,
        );
        registry.insert_test_builder(CONTRACT_SOURCE, Box::new(BufferedPluginBuilder));
        let old = lower(document.graph(), &registry).unwrap();

        set_viewer_selection(&mut document, source, 0, true);

        let new = lower(document.graph(), &registry).unwrap();
        assert!(diff(&old, &new, &registry).unwrap().is_empty());
    }
}
