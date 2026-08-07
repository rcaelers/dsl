use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_trigger::TriggerProgram;
use node_graph_document::SocketReference;
use signal_capture::CaptureChannelId;
use signal_capture_session::{
    AcquisitionContext, AcquisitionError, AcquisitionResult, CaptureProviderCapabilities,
    CaptureSessionPlan, CaptureStartMode, CaptureStoreCursor, PreparedAcquisition,
};
use signal_derived::DerivedDataRetention;
use signal_runtime::{NodeConfig, ProcessNode};

use crate::node_support::{
    CaptureCacheIdentity, CapturePresentation, DecoderTableColumnDescriptor,
    LanePresentationDescriptor, LiveCaptureEdit, NodeBuildContext, PortKind, ResolvedInputs,
    SamplingOverlayDescriptor, SimpleTriggerChannel, SourceDataLifecycle, TimelineMarkerDescriptor,
    TimelineMarkerEdit, TimelineMarkerReferenceBindingDescriptor,
    TimelineMarkerReferenceBindingEdit, TriggerConfigurationFeature, ViewerOutputControl,
};

/// Builds the processing source that replays a prepared live-capture store.
///
/// The graph runtime invokes this only after acquisition has prepared the source;
/// factories do not select host storage or execute a capture themselves.
pub trait CaptureGraphSourceFactory: Send + Sync {
    /// Creates the replay source reading from `cursor`.
    ///
    /// # Parameters
    /// - `cursor`: Prepared capture-store cursor to replay through the graph.
    fn create(&self, cursor: Box<dyn CaptureStoreCursor>) -> Result<Box<dyn ProcessNode>, String>;
}

/// Host-selected replacement capabilities for one inventory-provided graph node.
///
/// The stable node identifier keeps host composition independent of display names and concrete
/// node types. A host can replace one narrow capability without acquiring or replacing unrelated
/// compiler, runtime, capture, or presentation behavior.
pub struct GraphNodeCapabilityOverride {
    stable_id: String,
    semantics: Option<Box<dyn GraphNodeSemantics>>,
    materializer: Option<Box<dyn RuntimeMaterializer>>,
    capture_source: Option<Box<dyn CaptureSourceFeature>>,
    live_capture: Option<Box<dyn LiveCaptureFeatureProvider>>,
    presentation: Option<Box<dyn GraphNodePresentation>>,
    timeline: Option<Box<dyn TimelineFeature>>,
}

impl GraphNodeCapabilityOverride {
    /// Creates an empty narrow-capability override for a persisted node identity.
    pub fn capabilities(stable_id: impl Into<String>) -> Self {
        Self {
            stable_id: stable_id.into(),
            semantics: None,
            materializer: None,
            capture_source: None,
            live_capture: None,
            presentation: None,
            timeline: None,
        }
    }

    /// Replaces compiler-facing graph semantics.
    pub fn with_semantics(mut self, semantics: Box<dyn GraphNodeSemantics>) -> Self {
        self.semantics = Some(semantics);
        self
    }

    /// Replaces runtime materialization behavior.
    pub fn with_materializer(mut self, materializer: Box<dyn RuntimeMaterializer>) -> Self {
        self.materializer = Some(materializer);
        self
    }

    /// Replaces capture discovery and cache behavior.
    pub fn with_capture_source(mut self, capture_source: Box<dyn CaptureSourceFeature>) -> Self {
        self.capture_source = Some(capture_source);
        self
    }

    /// Replaces live acquisition and trigger-editing behavior.
    pub fn with_live_capture(mut self, live_capture: Box<dyn LiveCaptureFeatureProvider>) -> Self {
        self.live_capture = Some(live_capture);
        self
    }

    /// Replaces viewer and result-presentation metadata.
    pub fn with_presentation(mut self, presentation: Box<dyn GraphNodePresentation>) -> Self {
        self.presentation = Some(presentation);
        self
    }

    /// Replaces timeline metadata and editing behavior.
    pub fn with_timeline(mut self, timeline: Box<dyn TimelineFeature>) -> Self {
        self.timeline = Some(timeline);
        self
    }

    /// Returns the persisted feature identity replaced by this override.
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    /// Consumes the override into its validated registry input record.
    pub fn into_bundle(self) -> GraphNodeCapabilityBundle {
        GraphNodeCapabilityBundle {
            semantics: self.semantics,
            materializer: self.materializer,
            capture_source: self.capture_source,
            live_capture: self.live_capture,
            presentation: self.presentation,
            timeline: self.timeline,
        }
    }
}

/// Owned capability replacements consumed by graph-registry construction.
pub struct GraphNodeCapabilityBundle {
    /// Compiler-facing graph semantics replacement.
    pub semantics: Option<Box<dyn GraphNodeSemantics>>,
    /// Runtime materializer replacement.
    pub materializer: Option<Box<dyn RuntimeMaterializer>>,
    /// Capture-source replacement.
    pub capture_source: Option<Box<dyn CaptureSourceFeature>>,
    /// Live-capture replacement.
    pub live_capture: Option<Box<dyn LiveCaptureFeatureProvider>>,
    /// Presentation replacement.
    pub presentation: Option<Box<dyn GraphNodePresentation>>,
    /// Timeline replacement.
    pub timeline: Option<Box<dyn TimelineFeature>>,
}

impl GraphNodeCapabilityBundle {
    /// Creates an infrastructure bundle with graph semantics and runtime materialization.
    pub fn runtime(
        semantics: Box<dyn GraphNodeSemantics>,
        materializer: Box<dyn RuntimeMaterializer>,
    ) -> Self {
        Self {
            semantics: Some(semantics),
            materializer: Some(materializer),
            capture_source: None,
            live_capture: None,
            presentation: None,
            timeline: None,
        }
    }
}

/// Node-contributed live-acquisition capability discovered by the graph runtime.
pub trait LiveCaptureFeature: Send {
    /// Returns the enabled capture channels in provider order.
    fn channels(&self) -> &[CaptureChannelId];
    /// Returns the display names corresponding to [`Self::channels`].
    fn channel_names(&self) -> &[String];
    /// Returns the configured sampling frequency in hertz.
    fn sample_rate_hz(&self) -> f64;
    /// Returns the provider capabilities used to validate capture settings.
    fn capabilities(&self) -> &CaptureProviderCapabilities;
    /// Returns simple trigger controls, or an empty slice when unsupported.
    fn simple_trigger_channels(&self) -> &[SimpleTriggerChannel] {
        &[]
    }
    /// Returns the advanced trigger program, or `None` for free-run capture.
    fn trigger_program(&self) -> Option<&TriggerProgram> {
        None
    }
    /// Returns the provider's capture session plan when it supplies one.
    fn session_plan(&self) -> Option<&CaptureSessionPlan> {
        None
    }
    /// Returns the factory that turns prepared capture storage into a graph source.
    fn graph_source_factory(&self) -> Arc<dyn CaptureGraphSourceFactory>;
    /// Prepares the capture source using the provider's default start mode.
    ///
    /// # Parameters
    /// - `context`: Host capabilities, cancellation, and storage for preparation.
    fn prepare(
        self: Box<Self>,
        context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>>;
    /// Prepares the capture source with an explicit requested start mode.
    ///
    /// The default supports scheduled preparation and rejects immediate capture.
    ///
    /// # Parameters
    /// - `context`: Host capabilities, cancellation, and storage for preparation.
    /// - `mode`: Requested acquisition start behavior.
    fn prepare_with_mode(
        self: Box<Self>,
        context: AcquisitionContext,
        mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        if mode == CaptureStartMode::CaptureNow {
            return Err(AcquisitionError::UnsupportedOperation("capture now".into()));
        }
        self.prepare(context)
    }
}

/// Supplies capture-source discovery and cache behavior for one graph-node feature.
///
/// This capability is consumed during compilation and pre-run discovery. It does not expose graph
/// lowering, runtime materialization, or unrelated presentation behavior.
pub trait CaptureSourceFeature: Send + Sync {
    /// Returns capture data or metadata to hand to the waveform viewer.
    fn capture_presentation(&self, _state: &Value) -> Result<Option<CapturePresentation>, String> {
        Ok(None)
    }

    /// Returns the identity used to decide whether a prepared capture can be reused.
    fn capture_cache_identity(
        &self,
        _state: &Value,
        _resolved: &ResolvedInputs,
    ) -> CaptureCacheIdentity {
        CaptureCacheIdentity::NotCapture
    }
}

/// Discovers and edits live-acquisition behavior contributed by one graph-node feature.
///
/// The returned [`LiveCaptureFeature`] owns one state-specific acquisition session contract.
pub trait LiveCaptureFeatureProvider: Send + Sync {
    /// Returns a live-acquisition feature for the current saved state, when available.
    fn live_capture_feature(
        &self,
        state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String>;

    /// Returns validated trigger configuration exposed by this node, when any.
    fn trigger_configuration(
        &self,
        _state: &Value,
    ) -> Result<Option<TriggerConfigurationFeature>, String> {
        Ok(None)
    }

    /// Applies a user edit to live-capture trigger state.
    fn apply_live_capture_edit(
        &self,
        _state: &Value,
        _edit: &LiveCaptureEdit,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
}

/// Supplies viewer and result-presentation metadata for one graph-node feature.
pub trait GraphNodePresentation: Send + Sync {
    /// Returns an optional concrete word-display format for this output.
    fn word_display_format(&self, _socket: SocketReference<'_>, _state: &Value) -> Option<String> {
        None
    }

    /// Returns explicit compound-lane presentation metadata for this output.
    fn lane_presentation(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
    ) -> Option<LanePresentationDescriptor> {
        None
    }

    /// Returns result-table column metadata for this decoder output.
    fn decoder_table_column(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        None
    }

    /// Returns the capture-viewer channel from which this output originates.
    fn viewer_channel_origin(&self, _socket: SocketReference<'_>, _state: &Value) -> Option<usize> {
        None
    }

    /// Returns selection behavior for presenting this output in the viewer.
    fn viewer_output_control(
        &self,
        socket: SocketReference<'_>,
        state: &Value,
    ) -> Option<ViewerOutputControl> {
        if self.viewer_channel_origin(socket, state).is_some() {
            return Some(ViewerOutputControl::new(true, [socket.definition_index()]));
        }
        Some(ViewerOutputControl::new(false, [socket.definition_index()]))
    }

    /// Returns sampling-overlay reconstruction metadata when this node samples inputs.
    fn sampling_overlay(&self, _state: &Value) -> Option<SamplingOverlayDescriptor> {
        None
    }
}

/// Supplies node-owned timeline metadata and editing behavior.
pub trait TimelineFeature: Send + Sync {
    /// Returns node-owned markers for the host timeline.
    fn timeline_markers(&self, _state: &Value) -> Result<Vec<TimelineMarkerDescriptor>, String> {
        Ok(Vec::new())
    }

    /// Applies a host edit to one node-owned timeline marker.
    fn apply_timeline_marker_edit(
        &self,
        _state: &Value,
        _edit: &TimelineMarkerEdit,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }

    /// Returns controls bound to host-owned timeline marker references.
    fn timeline_marker_reference_bindings(
        &self,
        _state: &Value,
    ) -> Result<Vec<TimelineMarkerReferenceBindingDescriptor>, String> {
        Ok(Vec::new())
    }

    /// Applies new host choices to a timeline-marker reference control.
    fn apply_timeline_marker_reference_binding_edit(
        &self,
        _state: &Value,
        _edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
}

/// Supplies the compiler-facing semantics of one graph-node feature.
///
/// This contract contains graph classification, payload negotiation, runtime-port projection, and
/// retained-data policy. It does not materialize processing nodes or expose presentation and
/// capture-discovery behavior.
pub trait GraphNodeSemantics: Send + Sync {
    /// Returns the part of saved node state that can affect runtime behavior.
    fn execution_state(&self, state: &Value) -> Value {
        state.clone()
    }
    /// Returns whether this feature originates data instead of transforming it.
    fn is_source(&self) -> bool {
        false
    }
    /// Returns whether this source establishes the graph's capture/data time domain.
    fn is_time_domain_source(&self) -> bool {
        self.is_source()
    }
    /// Returns source-preparation behavior when this is a capture source.
    fn source_data_lifecycle(&self) -> Option<SourceDataLifecycle> {
        None
    }
    /// Returns the retention policy for data derived from this node.
    fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
        DerivedDataRetention::Unlimited
    }
    /// Returns whether this feature terminates a graph data flow.
    fn is_sink(&self) -> bool {
        false
    }
    /// Returns whether this feature subscribes to collected data.
    fn is_data_subscription(&self) -> bool {
        false
    }
    /// Returns whether this feature collects incoming data into retained lanes.
    fn is_data_collector(&self) -> bool {
        false
    }
    /// Names retained lanes contributed by each resolved input member.
    fn collected_lane_names(
        &self,
        _state: &Value,
        _resolved: &ResolvedInputs,
    ) -> Vec<(usize, String)> {
        Vec::new()
    }
    /// Returns the user-facing source label for a collected lane.
    fn collected_source_label(&self, _state: &Value, source_title: &str) -> String {
        source_title.to_owned()
    }
    /// Returns payload kinds accepted by an input socket in the given state.
    fn accepted_kinds(&self, socket: SocketReference<'_>, state: &Value) -> Vec<PortKind>;
    /// Returns payload kinds offered by an output socket in the given state.
    fn offered_kinds(&self, socket: SocketReference<'_>, state: &Value) -> Vec<PortKind>;
    /// Returns semantic contracts carried by an output socket.
    fn offered_connection_contracts(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
    ) -> Vec<String> {
        Vec::new()
    }
    /// Returns semantic contracts accepted by an input socket.
    fn accepted_connection_contracts(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
    ) -> Vec<String> {
        Vec::new()
    }
    /// Returns the runtime input port for a negotiated socket member.
    fn input_port(
        &self,
        socket: SocketReference<'_>,
        state: &Value,
        kind: PortKind,
    ) -> Option<String>;
    /// Returns the runtime output port for a negotiated socket.
    fn output_port(
        &self,
        socket: SocketReference<'_>,
        state: &Value,
        kind: PortKind,
    ) -> Option<String>;
    /// Returns whether an input socket must be connected in the given state.
    fn input_required(&self, _socket: SocketReference<'_>, _state: &Value) -> bool {
        true
    }
    /// Returns a node-specific input buffer capacity, when required.
    fn input_buffer_override(&self, _socket: SocketReference<'_>, _state: &Value) -> Option<usize> {
        None
    }
}

/// Materializes one compiler-resolved graph node into the processing runtime.
///
/// The processing plan retains this narrow capability after lowering. Runtime consumers do not
/// receive graph semantics, discovery, capture presentation, or editor behavior through it.
pub trait RuntimeMaterializer: Send + Sync {
    /// Builds the concrete processing node for the negotiated graph instance.
    ///
    /// # Parameters
    /// - `name`: Runtime name allocated to this graph node.
    /// - `state`: Current persisted node state.
    /// - `resolved`: Negotiated upstream inputs.
    /// - `ctx`: Run-scoped generic services available to the node.
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String>;

    /// Returns an in-place runtime configuration update, when supported.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    fn hot_config(&self, _state: &Value) -> Option<NodeConfig> {
        None
    }
}
