use std::sync::Arc;

use serde_json::Value;

use node_graph::api::Socket;
use signal_processing::{
    AcquisitionContext, AcquisitionError, AcquisitionResult, CaptureChannelId,
    CaptureProviderCapabilities, CaptureSessionPlan, CaptureStartMode, CaptureStoreCursor,
    DerivedDataRetention, NodeConfig, PreparedAcquisition, ProcessNode, TriggerProgram,
};

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

/// Host-selected replacement for one inventory-provided runtime builder.
///
/// The stable node identifier keeps host composition independent of graph-node
/// display names and concrete builder types.
pub struct RuntimeBuilderOverride {
    stable_id: String,
    builder: Box<dyn RuntimeBuilder>,
}

impl RuntimeBuilderOverride {
    /// Pairs a persisted node identity with a host-selected runtime builder.
    ///
    /// # Parameters
    /// - `stable_id`: Persisted feature identity to replace.
    /// - `builder`: Host-provided implementation for that feature.
    pub fn new(stable_id: impl Into<String>, builder: Box<dyn RuntimeBuilder>) -> Self {
        Self {
            stable_id: stable_id.into(),
            builder,
        }
    }

    /// Returns the persisted feature identity replaced by this override.
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    /// Consumes the override and returns its host-provided builder.
    pub fn into_builder(self) -> Box<dyn RuntimeBuilder> {
        self.builder
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

/// Materializes one concrete graph-node feature into a processing-runtime node.
///
/// Implementations own concrete protocol and node behavior. Generic consumers use
/// the neutral discovery and build results and never branch on node names or ports.
pub trait RuntimeBuilder {
    /// Returns the part of saved node state that can affect runtime behavior.
    ///
    /// Presentation-only controls override this projection so changing them
    /// refreshes host views without restarting the processing node.
    fn execution_state(&self, state: &Value) -> Value {
        state.clone()
    }
    /// Returns whether this feature originates data instead of transforming it.
    fn is_source(&self) -> bool {
        false
    }
    /// Whether this source establishes the graph's capture/data time domain.
    /// Auxiliary zero-input sources may emit values already expressed in that
    /// domain without competing with the capture source.
    fn is_time_domain_source(&self) -> bool {
        self.is_source()
    }
    /// Returns source-preparation behavior when this is a capture source.
    fn source_data_lifecycle(&self) -> Option<SourceDataLifecycle> {
        None
    }
    /// Returns the retention policy for data derived from this node.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    fn derived_data_retention(&self, state: &Value) -> DerivedDataRetention {
        let _ = state;
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
    /// Names retained lanes contributed by each output member.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    /// - `resolved`: Negotiated upstream inputs, including variadic members.
    fn collected_lane_names(
        &self,
        _state: &Value,
        _resolved: &ResolvedInputs,
    ) -> Vec<(usize, String)> {
        Vec::new()
    }
    /// Returns the user-facing source label for collected lanes.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    /// - `source_title`: User-visible title of the upstream source node.
    fn collected_source_label(&self, _state: &Value, source_title: &str) -> String {
        source_title.to_owned()
    }
    /// Returns payload kinds this input socket accepts in the given state.
    ///
    /// # Parameters
    /// - `socket`: Input socket being negotiated.
    /// - `state`: Current persisted node state.
    fn accepted_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind>;
    /// Returns payload kinds this output socket offers in the given state.
    ///
    /// # Parameters
    /// - `socket`: Output socket being negotiated.
    /// - `state`: Current persisted node state.
    fn offered_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind>;
    /// Optional owner-defined semantic contracts carried by an output.
    /// Empty means the payload type alone defines compatibility.
    fn offered_connection_contracts(&self, _socket: &Socket, _state: &Value) -> Vec<String> {
        Vec::new()
    }
    /// Optional owner-defined semantic contracts accepted by an input.
    /// When both ends declare contracts, at least one identity must match.
    fn accepted_connection_contracts(&self, _socket: &Socket, _state: &Value) -> Vec<String> {
        Vec::new()
    }
    /// Returns the runtime input name for a negotiated socket member.
    ///
    /// # Parameters
    /// - `socket`: Input socket being materialized.
    /// - `member_index`: Index of a variadic input member.
    /// - `state`: Current persisted node state.
    /// - `kind`: Negotiated payload kind.
    fn input_port(
        &self,
        socket: &Socket,
        member_index: usize,
        state: &Value,
        kind: PortKind,
    ) -> Option<String>;
    /// Returns the runtime output name for a negotiated socket.
    ///
    /// # Parameters
    /// - `socket`: Output socket being materialized.
    /// - `state`: Current persisted node state.
    /// - `kind`: Negotiated payload kind.
    fn output_port(&self, socket: &Socket, state: &Value, kind: PortKind) -> Option<String>;
    /// Returns an optional concrete word-display format for this output.
    fn word_display_format(&self, _socket: &Socket, _state: &Value) -> Option<String> {
        None
    }
    /// Returns explicit compound-lane presentation metadata for this output.
    fn lane_presentation(
        &self,
        _socket: &Socket,
        _state: &Value,
    ) -> Option<LanePresentationDescriptor> {
        None
    }
    /// Returns result-table column metadata for this decoder output.
    fn decoder_table_column(
        &self,
        _socket: &Socket,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        None
    }
    /// Returns the capture-viewer channel from which this output originates.
    fn viewer_channel_origin(&self, _socket: &Socket, _state: &Value) -> Option<usize> {
        None
    }
    /// Returns selection behavior for presenting this output in the viewer.
    fn viewer_output_control(&self, socket: &Socket, state: &Value) -> Option<ViewerOutputControl> {
        if self.viewer_channel_origin(socket, state).is_some() {
            return Some(ViewerOutputControl::new(true, [socket.def_index]));
        }
        Some(ViewerOutputControl::new(false, [socket.def_index]))
    }
    /// Returns capture data or metadata to hand to the waveform viewer.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        let _ = state;
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
    /// Returns sampling-overlay reconstruction metadata when this node samples inputs.
    fn sampling_overlay(&self, _state: &Value) -> Option<SamplingOverlayDescriptor> {
        None
    }
    /// Returns a live-acquisition feature when this node can acquire capture data.
    fn live_capture_feature(
        &self,
        _state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
        Ok(None)
    }
    /// Returns validated trigger configuration exposed by this node, when any.
    fn trigger_configuration(
        &self,
        _state: &Value,
    ) -> Result<Option<TriggerConfigurationFeature>, String> {
        Ok(None)
    }
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
    /// Applies a user edit to live-capture trigger state.
    fn apply_live_capture_edit(
        &self,
        _state: &Value,
        _edit: &LiveCaptureEdit,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
    /// Returns whether an input socket must be connected in the given state.
    fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
        true
    }
    /// Returns a node-specific input buffer capacity, if one is required.
    fn input_buffer_override(&self, _socket: &Socket, _state: &Value) -> Option<usize> {
        None
    }
    /// Builds the concrete processing node for the negotiated graph instance.
    ///
    /// # Parameters
    /// - `name`: Runtime name allocated to this graph node.
    /// - `state`: Current persisted node state.
    /// - `resolved`: Negotiated upstream inputs.
    /// - `ctx`: Run-scoped generic services available to the node.
    fn build(
        &self,
        _name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Err("graph-only builder has no runtime node".to_owned())
    }
    /// Returns an in-place runtime configuration update, when supported.
    ///
    /// # Parameters
    /// - `state`: Current persisted node state.
    fn hot_config(&self, state: &Value) -> Option<NodeConfig> {
        let _ = state;
        None
    }
}
