use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_capabilities::node::RuntimeMaterializer;
use logic_analyzer_graph_capabilities::node_support::{
    CaptureCacheIdentity, CapturePresentation, NodeBuildContext, PortKind, ResolvedInput,
    ResolvedInputs, SourceDataLifecycle,
};
use node_graph_document::NodeId;
use signal_derived::{
    CollectedLaneRequest, DerivedDataRetention, DerivedLanes, PayloadRegistry,
    PersistentStoreConfig, SamplingPointStore,
};

/// Error produced while constructing or materializing a processing graph.
#[derive(Debug, Clone)]
pub struct ProcessingGraphError {
    /// Offending editor node, or `None` for a graph-level failure.
    pub node: Option<NodeId>,
    /// User-presentable explanation of the failure.
    pub message: String,
}

impl ProcessingGraphError {
    /// Creates an error associated with one editor node.
    pub fn on(node: NodeId, message: impl Into<String>) -> Self {
        Self {
            node: Some(node),
            message: message.into(),
        }
    }

    /// Creates a graph-level error.
    pub fn global(message: impl Into<String>) -> Self {
        Self {
            node: None,
            message: message.into(),
        }
    }
}

/// Runtime-facing payload capabilities embedded into a processing graph by its producer.
pub trait ProcessingPayloadCatalog {
    /// Returns registered payload identities and adapters.
    fn payloads(&self) -> &PayloadRegistry;
    /// Returns whether the payload kind supports persistent caching.
    fn uses_persistent_cache(&self, kind: PortKind) -> bool;
    /// Configures a retained-lane request for the payload kind.
    fn configure_collected_lane_request(
        &self,
        kind: PortKind,
        request: CollectedLaneRequest,
        member: usize,
        input: &ResolvedInput,
        context: &dyn NodeBuildContext,
    ) -> Result<(CollectedLaneRequest, &str), String>;
}

/// Execution-ready graph produced by a graph compiler and consumed by a graph runtime.
#[derive(Clone)]
pub struct ProcessingGraph {
    /// Nodes in stable graph order.
    pub nodes: Vec<ProcessingNode>,
    /// Runtime connections between named ports.
    pub edges: Vec<ProcessingEdge>,
    /// Combined derived-data retention policy.
    pub derived_data_retention: DerivedDataRetention,
    /// Sampling overlays available to application presentation.
    pub sampling_overlays: Vec<SamplingOverlayCandidate>,
    /// Application output subscription choices resolved with this graph.
    pub output_subscriptions: OutputSubscriptionPlan,
    /// Payload capabilities needed while materializing generated collectors.
    pub payload_catalog: Arc<dyn ProcessingPayloadCatalog>,
}

impl fmt::Debug for ProcessingGraph {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessingGraph")
            .field("nodes", &self.nodes)
            .field("edges", &self.edges)
            .field("derived_data_retention", &self.derived_data_retention)
            .field("sampling_overlays", &self.sampling_overlays)
            .field("output_subscriptions", &self.output_subscriptions)
            .finish_non_exhaustive()
    }
}

/// Execution-ready description of one graph node.
#[derive(Clone)]
pub struct ProcessingNode {
    /// Source node identity from the editor graph.
    pub id: NodeId,
    /// Stable diagnostic name of the resolved builder.
    pub builder: String,
    /// Resolved materializer captured by the compiler.
    pub materializer: Arc<dyn RuntimeMaterializer>,
    /// Runtime-relevant state projection used to classify live graph edits.
    pub execution_state: serde_json::Value,
    /// Source readiness behavior projected by the compiler.
    pub source_data_lifecycle: Option<SourceDataLifecycle>,
    /// Whether this node establishes the graph's capture/data time domain.
    pub time_domain_source: bool,
    /// Whether this node terminates a graph data flow.
    pub sink: bool,
    /// Persisted state used to build the node.
    pub state: serde_json::Value,
    /// Pipeline node name.
    pub runtime_name: String,
    /// Whether the node collects inputs into retained output lanes.
    pub data_collector: bool,
    /// Retained lane names projected by the compiler for collector input members.
    pub collected_lane_names: Vec<(usize, String)>,
    /// User-facing source labels projected for collector input members.
    pub collected_source_labels: Vec<(usize, String)>,
    /// Negotiated upstream connections by input definition and member.
    pub resolved: ResolvedInputs,
    /// Source cache-reuse identity determined during compilation.
    pub capture_cache_identity: CaptureCacheIdentity,
    /// Persistent caches associated with retained outputs.
    pub derived_word_caches: Vec<Option<PersistentStoreConfig>>,
}

impl fmt::Debug for ProcessingNode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessingNode")
            .field("id", &self.id)
            .field("builder", &self.builder)
            .field("state", &self.state)
            .field("execution_state", &self.execution_state)
            .field("source_data_lifecycle", &self.source_data_lifecycle)
            .field("time_domain_source", &self.time_domain_source)
            .field("sink", &self.sink)
            .field("runtime_name", &self.runtime_name)
            .field("data_collector", &self.data_collector)
            .field("collected_lane_names", &self.collected_lane_names)
            .field("collected_source_labels", &self.collected_source_labels)
            .field("resolved", &self.resolved)
            .field("capture_cache_identity", &self.capture_cache_identity)
            .field("derived_word_caches", &self.derived_word_caches)
            .finish()
    }
}

/// Runtime edge between two named processing-node ports.
#[derive(Debug, Clone)]
pub struct ProcessingEdge {
    /// Source node and runtime output name.
    pub from: (NodeId, String),
    /// Destination node and runtime input name.
    pub to: (NodeId, String),
    /// Bounded channel capacity.
    pub buffer: usize,
    /// Negotiated payload kind.
    pub kind: PortKind,
}

/// Application-supplied retained outputs and the subset currently presented.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSubscriptionPlan {
    visible_outputs: Vec<(NodeId, usize)>,
    retained_outputs: Vec<(NodeId, usize)>,
}

impl OutputSubscriptionPlan {
    /// Creates an empty output-subscription plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retains and presents one node output.
    pub fn subscribe(&mut self, node: NodeId, output: usize) {
        self.retain(node, output);
        if !self.contains(node, output) {
            self.visible_outputs.push((node, output));
        }
    }

    /// Retains an endpoint without presenting it.
    pub fn retain(&mut self, node: NodeId, output: usize) {
        if !self.is_retained(node, output) {
            self.retained_outputs.push((node, output));
        }
    }

    /// Returns whether the endpoint is presented.
    pub fn contains(&self, node: NodeId, output: usize) -> bool {
        self.visible_outputs.contains(&(node, output))
    }

    /// Returns whether the endpoint is retained.
    pub fn is_retained(&self, node: NodeId, output: usize) -> bool {
        self.retained_outputs.contains(&(node, output))
    }

    /// Iterates presented endpoints.
    pub fn outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.visible_outputs.iter().copied()
    }

    /// Iterates retained endpoints.
    pub fn retained_outputs(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.retained_outputs.iter().copied()
    }
}

impl FromIterator<(NodeId, usize)> for OutputSubscriptionPlan {
    fn from_iter<T: IntoIterator<Item = (NodeId, usize)>>(iter: T) -> Self {
        let mut plan = Self::new();
        for (node, output) in iter {
            plan.subscribe(node, output);
        }
        plan
    }
}

/// One retained lane produced for an application output subscription.
#[derive(Clone, Debug)]
pub struct CollectedOutputLane {
    /// Variadic input member that produced the lane.
    pub member: usize,
    /// Runtime lane name.
    pub lane_name: String,
    /// User-facing source label.
    pub source_label: String,
    /// Resolved input and presentation metadata.
    pub input: ResolvedInput,
}

/// Runtime identities and source metadata for one collected output set.
#[derive(Clone, Debug)]
pub struct CollectedOutputSubscription {
    /// Runtime name of the collector node.
    pub runtime_name: String,
    /// Retained lanes produced by the collector.
    pub lanes: Vec<CollectedOutputLane>,
}

/// Retained lanes carrying decoder-table metadata for one collector.
#[derive(Clone, Debug)]
pub struct CollectedTableSubscription {
    /// Graph node owning the collector.
    pub collector: NodeId,
    /// Retained table lanes.
    pub lanes: Vec<CollectedOutputLane>,
}

/// Fully resolved sampling inputs rendered as one overlay.
#[derive(Debug, Clone)]
pub struct ResolvedSamplingOverlay {
    /// Viewer channel carrying the clock.
    pub clock_channel: usize,
    /// Viewer channels sampled at clock transitions.
    pub sampled_channels: Vec<usize>,
    /// Sampling-point store.
    pub points: SamplingPointStore,
}

/// Fully resolved selectable sampling overlay.
#[derive(Debug, Clone)]
pub struct SamplingOverlayCandidate {
    node_id: NodeId,
    node_title: String,
    overlay: ResolvedSamplingOverlay,
    cache_key: Option<[u8; 32]>,
    retained_word_lane: Option<(String, bool)>,
}

impl SamplingOverlayCandidate {
    /// Creates a resolved overlay.
    pub fn new(
        node_id: NodeId,
        node_title: String,
        overlay: ResolvedSamplingOverlay,
        retained_word_lane: Option<(String, bool)>,
    ) -> Self {
        Self {
            node_id,
            node_title,
            overlay,
            cache_key: None,
            retained_word_lane,
        }
    }

    /// Returns the owning graph node.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Returns the node title.
    pub fn node_title(&self) -> &str {
        &self.node_title
    }

    /// Returns the resolved overlay.
    pub fn overlay(&self) -> &ResolvedSamplingOverlay {
        &self.overlay
    }

    /// Returns the persistent sampling-cache identity assigned by runtime policy.
    pub fn cache_key(&self) -> Option<[u8; 32]> {
        self.cache_key
    }

    /// Replaces the persistent sampling-cache identity.
    pub fn set_cache_key(&mut self, cache_key: Option<[u8; 32]>) {
        self.cache_key = cache_key;
    }

    /// Replaces the sampling-point store retained by the overlay.
    pub fn set_points(&mut self, points: SamplingPointStore) {
        self.overlay.points = points;
    }

    /// Installs a lazy provider backed by the retained word lane when one was resolved.
    pub fn install_retained_word_provider(&mut self, lanes: DerivedLanes) -> bool {
        let Some((name, clock_high)) = &self.retained_word_lane else {
            return false;
        };
        self.overlay.points.set_retained_word_provider(
            lanes,
            name,
            *clock_high,
            self.overlay.sampled_channels.len(),
        );
        true
    }

    /// Returns whether lowering resolved a reusable retained word lane.
    pub fn uses_retained_word_lane(&self) -> bool {
        self.retained_word_lane.is_some()
    }
}

/// Capture presentation request passed from compiler discovery to runtime preparation.
pub struct DiscoveredCapturePresentation {
    /// Stable source identity.
    pub identity: String,
    /// Viewer channel indexes selected for display.
    pub visible_channels: Vec<usize>,
    /// Indexed, in-memory, or metadata-only presentation.
    pub presentation: CapturePresentation,
}
