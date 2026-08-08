//! Deterministic graph-document semantic analysis and lowering.
//!
//! Lowering turns the editable document into a pure, diffable [`ProcessingGraph`]: it prunes to
//! sink-reachable nodes, follows reroutes, validates semantics, and negotiates stream kinds.
//! Materialization and all execution-lifetime behavior belong to `logic_analyzer_graph_runtime`.
//!
//! Kind negotiation: each edge picks `offered ∩ accepted`, producer
//! preference order winning. That is what maps one UI `Signal` socket onto
//! the source's dual `d{i}`/`b{i}` ports; every `Words` socket carries the
//! same `Word` runtime type regardless of which decoder produced it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{CaptureGraphSourceFactory, LiveCaptureFeature};
use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, ResolvedInput, ResolvedInputs, SimpleTriggerChannel, TimelineMarkerDescriptor,
    TimelineMarkerEdit, TimelineMarkerReferenceBindingDescriptor,
    TimelineMarkerReferenceBindingEdit, TriggerConfigurationFeature,
};
use logic_analyzer_graph_plan::{
    CapturePresentationDiscoveryError, DiscoveredCapturePresentation, OutputSubscriptionPlan,
    ProcessingEdge, ProcessingGraph, ProcessingGraphError as CompileError, ProcessingNode,
    ProcessingPayloadCatalog, ResolvedSamplingOverlay, SamplingOverlayCandidate,
};
use logic_analyzer_graph_registry::GraphRegistry;
use logic_analyzer_trigger::{SimpleTriggerCondition, TriggerProgram};
use node_graph_document::{
    Connection, GraphState, Node, NodeId, NodeKind, Socket, SocketDirection, SocketId,
    SocketReference, SocketShape, VariadicInfo,
};
use signal_capture::CaptureChannelId;
use signal_capture_session::{
    AcquisitionContext, AcquisitionResult, CaptureProviderCapabilities, CaptureSessionPlan,
    CaptureStartMode, PreparedAcquisition,
};
use signal_derived::{DerivedDataRetention, SamplingPointStore};

use super::error::{LiveCaptureOperationError, TimelineOperationError};

// ── Builder trait & registry ─────────────────────────────────────────────────

/// Trigger configuration discovered from one live-capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTriggerConfiguration {
    /// Source node that contributed the feature.
    pub source_node: NodeId,
    /// User-visible title of the source node.
    pub source_title: String,
    /// Validated trigger configuration exposed by the source.
    pub feature: TriggerConfigurationFeature,
}

/// Node-owned marker discovered for presentation in the host timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTimelineMarker {
    /// Node that owns and receives edits for the marker.
    pub owner_node: NodeId,
    /// User-visible title of the owning node.
    pub owner_title: String,
    /// Marker definition contributed by the node.
    pub marker: TimelineMarkerDescriptor,
}

/// Timeline-reference control discovered from one concrete node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTimelineMarkerReferenceBinding {
    /// Node that owns and receives edits for the control.
    pub owner_node: NodeId,
    /// User-visible title of the owning node.
    pub owner_title: String,
    /// Host-reference binding contributed by the node.
    pub binding: TimelineMarkerReferenceBindingDescriptor,
}

/// One live-capture feature discovered from the lowered graph.
pub struct DiscoveredLiveCaptureFeature {
    source_node: NodeId,
    source_title: String,
    visible_channels: Vec<usize>,
    feature: Box<dyn LiveCaptureFeature>,
}

impl DiscoveredLiveCaptureFeature {
    /// Creates a discovery result with every provider channel initially visible.
    ///
    /// # Parameters
    /// - `source_node`: Graph node that owns the live-capture feature.
    /// - `source_title`: User-visible title of that source node.
    /// - `feature`: Provider feature used to prepare capture data.
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

    /// Returns physical provider channels in provider-defined order.
    pub fn channels(&self) -> &[CaptureChannelId] {
        self.feature.channels()
    }

    /// Returns the graph node that owns this feature.
    pub fn source_node(&self) -> NodeId {
        self.source_node
    }

    /// Returns the user-visible title of the source node.
    pub fn source_title(&self) -> &str {
        &self.source_title
    }

    /// Returns display names in provider channel order.
    pub fn channel_names(&self) -> &[String] {
        self.feature.channel_names()
    }

    /// Returns provider-channel indices selected for application presentation.
    pub fn visible_channels(&self) -> &[usize] {
        &self.visible_channels
    }

    /// Returns the feature's capture sample rate in hertz.
    pub fn sample_rate_hz(&self) -> f64 {
        self.feature.sample_rate_hz()
    }

    /// Returns the provider's capture and command capabilities.
    pub fn capabilities(&self) -> &CaptureProviderCapabilities {
        self.feature.capabilities()
    }

    /// Returns simple-trigger controls supplied by the capture feature.
    pub fn simple_trigger_channels(&self) -> &[SimpleTriggerChannel] {
        self.feature.simple_trigger_channels()
    }

    /// Returns the advanced trigger program, if the provider supplied one.
    pub fn trigger_program(&self) -> Option<&TriggerProgram> {
        self.feature.trigger_program()
    }

    /// Returns whether either advanced or enabled simple triggering is configured.
    pub fn has_trigger_program(&self) -> bool {
        self.trigger_program().is_some() || self.has_simple_trigger()
    }

    /// Returns the provider's capture session plan, when available.
    pub fn session_plan(&self) -> Option<&CaptureSessionPlan> {
        self.feature.session_plan()
    }

    /// Returns whether any enabled channel has a non-ignore simple trigger.
    pub fn has_simple_trigger(&self) -> bool {
        self.simple_trigger_channels()
            .iter()
            .any(|channel| channel.enabled && channel.condition != SimpleTriggerCondition::Ignore)
    }

    /// Returns the factory for replaying the prepared capture through the graph.
    pub fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory> {
        self.feature.graph_source_factory()
    }

    /// Prepares the capture provider using the requested start mode.
    ///
    /// # Parameters
    /// - `context`: Host capabilities, cancellation, and storage for preparation.
    /// - `mode`: Requested acquisition start behavior.
    pub fn prepare(
        self,
        context: AcquisitionContext,
        mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        self.feature.prepare_with_mode(context, mode)
    }
}

fn capture_channel_selection(
    subscriptions: &OutputSubscriptionPlan,
    node_id: NodeId,
    node: &Node,
    viewer_channel_origin: impl Fn(SocketReference<'_>) -> Option<usize>,
) -> Vec<usize> {
    subscriptions
        .outputs()
        .filter(|(selected_node, _)| *selected_node == node_id)
        .filter_map(|(_, output)| {
            node.outputs.get(output).and_then(|socket| {
                viewer_channel_origin(socket.reference(SocketDirection::Output, 0))
            })
        })
        .collect()
}

pub(crate) fn discover_capture_presentation_with_subscriptions(
    graph: &GraphState,
    builders: &GraphRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<Option<DiscoveredCapturePresentation>, CapturePresentationDiscoveryError> {
    let mut candidates = Vec::new();
    for (&node_id, node) in &graph.nodes {
        if node.kind != NodeKind::Regular || node.muted {
            continue;
        }
        let Some(feature) = builders.capture_source(node.def_name()) else {
            continue;
        };
        let Some(presentation) = feature.capture_presentation(&node.state).map_err(|error| {
            CapturePresentationDiscoveryError::source_feature(node_id, node.title.clone(), error)
        })?
        else {
            continue;
        };
        let visible_channels = capture_channel_selection(subscriptions, node_id, node, |output| {
            builders
                .presentation(node.def_name())
                .and_then(|presentation| presentation.viewer_channel_origin(output, &node.state))
        });
        let identity_state = (&node.state, &visible_channels);
        let state = serde_json::to_vec(&identity_state)
            .map_err(CapturePresentationDiscoveryError::identity)?;
        candidates.push(DiscoveredCapturePresentation {
            identity: format!("{node_id:?}:{}", blake3::hash(&state).to_hex()),
            visible_channels,
            presentation,
        });
    }
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        count => Err(CapturePresentationDiscoveryError::multiple_sources(count)),
    }
}
/// Resolves exactly one enabled live-capture feature without identifying a
/// concrete node type. Muted nodes do not participate in acquisition.
pub(crate) fn discover_live_capture_feature_with_subscriptions(
    graph: &GraphState,
    builders: &GraphRegistry,
    subscriptions: &OutputSubscriptionPlan,
) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureOperationError> {
    discover_live_capture_feature_from(graph, builders, subscriptions, |_| true)
}
/// Resolves exactly one enabled trigger-configuration feature without consulting acquisition
/// backends or identifying a concrete node type.
pub(crate) fn discover_trigger_configuration(
    graph: &GraphState,
    builders: &GraphRegistry,
) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureOperationError> {
    let mut candidates = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(feature) = builders.live_capture(node.def_name()) else {
            continue;
        };
        match feature.trigger_configuration(&node.state) {
            Ok(Some(feature)) => candidates.push(DiscoveredTriggerConfiguration {
                source_node: node.id,
                source_title: node.title.clone(),
                feature,
            }),
            Ok(None) => {}
            Err(error) => {
                return Err(LiveCaptureOperationError::feature(
                    node.id,
                    node.title.clone(),
                    error,
                ));
            }
        }
    }
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => Err(LiveCaptureOperationError::MultipleTriggerConfigurations {
            source_nodes: candidates
                .iter()
                .map(|candidate| candidate.source_node)
                .collect(),
        }),
    }
}

/// Discovers every enabled marker through node-owned, protocol-neutral contracts.
pub(crate) fn discover_timeline_markers(
    graph: &GraphState,
    builders: &GraphRegistry,
) -> Result<Vec<DiscoveredTimelineMarker>, TimelineOperationError> {
    let mut discovered = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(timeline) = builders.timeline(node.def_name()) else {
            continue;
        };
        let markers = timeline
            .timeline_markers(&node.state)
            .map_err(|error| TimelineOperationError::feature(node.id, node.title.clone(), error))?;
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
    builders: &GraphRegistry,
    owner_node: NodeId,
    edit: &TimelineMarkerEdit,
) -> Result<Value, TimelineOperationError> {
    let node = graph
        .nodes
        .get(&owner_node)
        .ok_or(TimelineOperationError::MarkerOwnerMissing { owner_node })?;
    let timeline = builders.timeline(node.def_name()).ok_or_else(|| {
        TimelineOperationError::FeatureUnavailable {
            definition_name: node.def_name().to_owned(),
        }
    })?;
    timeline
        .apply_timeline_marker_edit(&node.state, edit)
        .map_err(|error| TimelineOperationError::feature(node.id, node.title.clone(), error))?
        .ok_or_else(|| TimelineOperationError::UnsupportedMarkerEdit {
            owner_title: node.title.clone(),
        })
}

/// Discovers controls which select a host-owned timeline position.
pub(crate) fn discover_timeline_marker_reference_bindings(
    graph: &GraphState,
    builders: &GraphRegistry,
) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, TimelineOperationError> {
    let mut discovered = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted)
    {
        let Some(timeline) = builders.timeline(node.def_name()) else {
            continue;
        };
        let bindings = timeline
            .timeline_marker_reference_bindings(&node.state)
            .map_err(|error| TimelineOperationError::feature(node.id, node.title.clone(), error))?;
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
    builders: &GraphRegistry,
    owner_node: NodeId,
    edit: &TimelineMarkerReferenceBindingEdit,
) -> Result<Value, TimelineOperationError> {
    let node = graph
        .nodes
        .get(&owner_node)
        .ok_or(TimelineOperationError::ReferenceOwnerMissing { owner_node })?;
    let timeline = builders.timeline(node.def_name()).ok_or_else(|| {
        TimelineOperationError::FeatureUnavailable {
            definition_name: node.def_name().to_owned(),
        }
    })?;
    timeline
        .apply_timeline_marker_reference_binding_edit(&node.state, edit)
        .map_err(|error| TimelineOperationError::feature(node.id, node.title.clone(), error))?
        .ok_or_else(|| TimelineOperationError::UnsupportedReferenceEdit {
            owner_title: node.title.clone(),
        })
}

/// Routes a portable live-feature edit to the concrete builder that owns `source_node`.
pub(crate) fn apply_live_capture_edit(
    graph: &GraphState,
    builders: &GraphRegistry,
    source_node: NodeId,
    edit: &LiveCaptureEdit,
) -> Result<Value, LiveCaptureOperationError> {
    let node = graph
        .nodes
        .get(&source_node)
        .ok_or(LiveCaptureOperationError::OwnerMissing {
            owner_node: source_node,
        })?;
    let feature = builders.live_capture(node.def_name()).ok_or_else(|| {
        LiveCaptureOperationError::FeatureUnavailable {
            owner_node: node.id,
            definition_name: node.def_name().to_owned(),
        }
    })?;
    feature
        .apply_live_capture_edit(&node.state, edit)
        .map_err(|error| LiveCaptureOperationError::feature(node.id, node.title.clone(), error))?
        .ok_or_else(|| LiveCaptureOperationError::UnsupportedEdit {
            owner_node: node.id,
            owner_title: node.title.clone(),
        })
}

/// Resolves a live feature only from nodes retained by a successfully
/// compiled graph. This prevents a disconnected development or hardware node
/// from becoming the acquisition source for a different active time domain.
fn discover_live_capture_feature_from(
    graph: &GraphState,
    builders: &GraphRegistry,
    subscriptions: &OutputSubscriptionPlan,
    include: impl Fn(&Node) -> bool,
) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureOperationError> {
    let mut candidates = Vec::new();
    for node in graph
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Regular && !node.muted && include(node))
    {
        let Some(feature_provider) = builders.live_capture(node.def_name()) else {
            continue;
        };
        match feature_provider.live_capture_feature(&node.state) {
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
                            timeout.action
                                == signal_capture_session::TriggerTimeoutAction::ForceTrigger
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
                    return Err(LiveCaptureOperationError::InvalidFeature {
                        owner_node: node.id,
                        owner_title: node.title.clone(),
                        message: message.into(),
                    });
                }
                candidates.push(DiscoveredLiveCaptureFeature::new_with_visible_channels(
                    node.id,
                    node.title.clone(),
                    capture_channel_selection(subscriptions, node.id, node, |output| {
                        builders
                            .presentation(node.def_name())
                            .and_then(|presentation| {
                                presentation.viewer_channel_origin(output, &node.state)
                            })
                    }),
                    feature,
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(LiveCaptureOperationError::feature(
                    node.id,
                    node.title.clone(),
                    error,
                ));
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
            Err(LiveCaptureOperationError::MultipleSources { source_nodes })
        }
    }
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
    registry: &GraphRegistry,
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
                    let Some(semantics) = registry.semantics(node.def_name()) else {
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
                                .and_then(|target| registry.semantics(target.def_name()))
                                .is_some_and(|target| {
                                    target.is_data_collector() || target.is_data_subscription()
                                })
                    });
                    let output = output.reference(SocketDirection::Output, 0);
                    (connected || subscriptions.is_retained(id, *index))
                        && !already_collected
                        && registry
                            .presentation(node.def_name())
                            .and_then(|features| {
                                features.viewer_channel_origin(output, &node.state)
                            })
                            .is_none()
                        && semantics
                            .offered_kinds(output, &node.state)
                            .into_iter()
                            .any(|kind| {
                                subscribable.contains(&kind)
                                    && semantics.output_port(output, &node.state, kind).is_some()
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
            let Some(semantics) = registry.semantics(node.def_name()) else {
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
                                .and_then(|target| registry.semantics(target.def_name()))
                                .is_some_and(|builder| {
                                    builder.is_data_collector() || builder.is_data_subscription()
                                })
                    });
                    let output = output.reference(SocketDirection::Output, 0);
                    !retained
                        && !collected_by_explicit_sink
                        && registry
                            .presentation(node.def_name())
                            .and_then(|features| features.decoder_table_column(output, &node.state))
                            .is_some()
                        && semantics
                            .offered_kinds(output, &node.state)
                            .into_iter()
                            .any(|kind| semantics.output_port(output, &node.state, kind).is_some())
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
            super::data_collector::OUTPUT_SUBSCRIPTION_BUILDER_NAME,
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
            super::data_collector::BUILDER_NAME,
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
    registry: &GraphRegistry,
    subscriptions: &OutputSubscriptionPlan,
    payload_catalog: Arc<dyn ProcessingPayloadCatalog>,
) -> Result<ProcessingGraph, Vec<CompileError>> {
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
                    .semantics(node.def_name())
                    .is_some_and(|semantics| {
                        semantics.is_sink() || semantics.is_data_subscription()
                    })
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
        match registry.semantics(node.def_name()) {
            None => errors.push(CompileError::on(
                id,
                format!("'{}' has no runtime implementation", node.def_name()),
            )),
            Some(semantics) if semantics.is_source() => {
                runtime_source_count += 1;
                if semantics.is_time_domain_source() {
                    time_domain_source_count += 1;
                    derived_data_retention = semantics.derived_data_retention(&node.state);
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
                .semantics(node.def_name())
                .is_some_and(|semantics| semantics.is_time_domain_source())
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
    let mut edges: Vec<ProcessingEdge> = Vec::new();
    let mut connected: HashMap<NodeId, HashSet<usize>> = HashMap::new();
    for wire in &wires {
        if !keep.contains(&wire.from.node) || !keep.contains(&wire.to.node) {
            continue;
        }
        let from_node = &graph.nodes[&wire.from.node];
        let to_node = &graph.nodes[&wire.to.node];
        let (Some(from_semantics), Some(to_semantics)) = (
            registry.semantics(from_node.def_name()),
            registry.semantics(to_node.def_name()),
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

        let from_reference = from_node
            .socket_reference(SocketDirection::Output, wire.from.index)
            .expect("validated source socket has a semantic reference");
        let to_reference = to_node
            .socket_reference(SocketDirection::Input, wire.to.index)
            .expect("validated destination socket has a semantic reference");
        let member = to_reference.member_index();
        let offered = from_semantics.offered_kinds(from_reference, &from_node.state);
        let data_subscription = to_semantics.is_data_subscription();
        let registered_collection = data_subscription || to_semantics.is_data_collector();
        let accepted = if registered_collection {
            registry.subscribable_payload_kinds()
        } else {
            to_semantics.accepted_kinds(to_reference, &to_node.state)
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
            from_semantics.offered_connection_contracts(from_reference, &from_node.state);
        let accepted_contracts =
            to_semantics.accepted_connection_contracts(to_reference, &to_node.state);
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

        let Some(out_port) = from_semantics.output_port(from_reference, &from_node.state, kind)
        else {
            errors.push(CompileError::on(
                wire.from.node,
                format!("No runtime port for output '{}'", from_socket.name),
            ));
            continue;
        };
        let Some(in_port) = to_semantics.input_port(to_reference, &to_node.state, kind) else {
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
                source_output_title: from_socket.name.clone(),
                word_display_format: registry.presentation(from_node.def_name()).and_then(
                    |features| features.word_display_format(from_reference, &from_node.state),
                ),
                lane_presentation: registry.presentation(from_node.def_name()).and_then(
                    |features| features.lane_presentation(from_reference, &from_node.state),
                ),
                default_lane_presentation: registered_collection
                    .then(|| registry.payload_subscription_presentation(kind))
                    .flatten(),
                decoder_table_column: registry.presentation(from_node.def_name()).and_then(
                    |features| features.decoder_table_column(from_reference, &from_node.state),
                ),
                capture_channel: registry
                    .presentation(from_node.def_name())
                    .and_then(|features| {
                        features.viewer_channel_origin(from_reference, &from_node.state)
                    }),
            },
        );
        edges.push(ProcessingEdge {
            from: (wire.from.node, out_port),
            to: (wire.to.node, in_port),
            buffer: to_semantics
                .input_buffer_override(to_reference, &to_node.state)
                .unwrap_or_else(|| kind.buffer_size(from_semantics.is_source())),
            kind,
        });
    }

    // Required inputs.
    for &id in &kept {
        let node = &graph.nodes[&id];
        let Some(semantics) = registry.semantics(node.def_name()) else {
            continue;
        };
        let node_connected = connected.get(&id);
        for (index, socket) in node.inputs.iter().enumerate() {
            let reference = node
                .socket_reference(SocketDirection::Input, index)
                .expect("enumerated input has a semantic reference");
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
                if !has_member && semantics.input_required(reference, &node.state) {
                    errors.push(CompileError::on(
                        id,
                        format!("Input '{}' needs at least one connection", socket.name),
                    ));
                }
            } else if !socket.is_variadic_member()
                && !node_connected.is_some_and(|set| set.contains(&index))
                && semantics.input_required(reference, &node.state)
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

    let nodes: Vec<ProcessingNode> = kept
        .iter()
        .map(|&id| {
            let node = &graph.nodes[&id];
            let resolved = resolved.remove(&id).unwrap_or_default();
            let semantics = registry
                .semantics(node.def_name())
                .expect("retained node has registered semantics");
            let materializer = registry
                .materializer(node.def_name())
                .expect("retained node has a registered materializer");
            let data_collector = semantics.is_data_collector() || semantics.is_data_subscription();
            let collected_lane_names = semantics.collected_lane_names(&node.state, &resolved);
            let collected_source_labels = collected_lane_names
                .iter()
                .filter_map(|(member, _)| {
                    let input = resolved.get(0, *member)?;
                    Some((
                        *member,
                        semantics.collected_source_label(&node.state, &input.source_node_title),
                    ))
                })
                .collect();
            let capture_cache_identity = registry.capture_source(node.def_name()).map_or(
                logic_analyzer_graph_capabilities::node_support::CaptureCacheIdentity::NotCapture,
                |feature| feature.capture_cache_identity(&node.state, &resolved),
            );
            ProcessingNode {
                id,
                builder: node.def_name().to_owned(),
                materializer,
                state: node.state.clone(),
                execution_state: semantics.execution_state(&node.state),
                source_data_lifecycle: semantics.source_data_lifecycle(),
                time_domain_source: semantics.is_time_domain_source(),
                sink: semantics.is_sink(),
                runtime_name: runtime_name(node),
                data_collector,
                collected_lane_names,
                collected_source_labels,
                capture_cache_identity,
                resolved,
                derived_word_caches: Vec::new(),
            }
        })
        .collect();
    let sampling_overlays = nodes
        .iter()
        .filter_map(|compiled_node| {
            let features = registry.presentation(&compiled_node.builder)?;
            let descriptor = features.sampling_overlay(&compiled_node.state)?;
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
                retained_word_sampling_lane_name(&nodes, compiled_node.id, source.output)
                    .map(|name| (name, source.clock_high))
            });
            Some(SamplingOverlayCandidate::new(
                compiled_node.id,
                graph.nodes[&compiled_node.id].title.clone(),
                ResolvedSamplingOverlay {
                    clock_channel,
                    sampled_channels,
                    points: SamplingPointStore::default(),
                },
                retained_word_lane,
            ))
        })
        .collect();
    let compiled = ProcessingGraph {
        nodes,
        edges,
        derived_data_retention,
        sampling_overlays,
        output_subscriptions: subscriptions.clone(),
        payload_catalog,
    };
    Ok(compiled)
}

fn retained_word_sampling_lane_name(
    nodes: &[ProcessingNode],
    source_node: NodeId,
    source_output: usize,
) -> Option<String> {
    nodes
        .iter()
        .filter(|node| node.data_collector)
        .find_map(|node| {
            let names = &node.collected_lane_names;
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
    registry: &GraphRegistry,
    subscriptions: &OutputSubscriptionPlan,
    payload_catalog: Arc<dyn ProcessingPayloadCatalog>,
) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
    lower_with_subscriptions(graph, registry, subscriptions, payload_catalog)
        .map(|compiled| compiled.sampling_overlays)
}

fn has_cycle(nodes: &[NodeId], edges: &[ProcessingEdge]) -> bool {
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
