use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use node_graph::api::NodeId;
use signal_artifacts::{ArtifactRepository, MemoryArtifactRepository, SourceIdentity};
use signal_capture::CaptureIndexFactory;
use signal_capture_session::{
    CaptureChannelId, SimpleTriggerCondition, TriggerEditorSchema, TriggerProgram,
};
use signal_derived::{
    DerivedDataRetention, DerivedLanes, PersistentStoreConfig, SamplingPointStore, TimelineMarker,
};
use signal_runtime::{InlineWorkExecutor, WorkExecutor};

use super::port::PortKind;

/// Logic-analyzer presentation choice contributed by a concrete graph node.
/// Generic graph widgets receive only a transient, application-neutral UI
/// model derived from this contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerOutputControl {
    /// Excludes the output from viewer-output selection.
    Hidden,
    /// Makes the output selectable in the node's View panel.
    Selectable {
        /// Whether the output is selected when the node is first created.
        default_selected: bool,
        /// Output indexes whose node-header indicators reflect this selection.
        indicator_outputs: Vec<usize>,
    },
}

/// Logic-analyzer-owned data supplied to the node's viewer-output panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewerOutputPanelModel {
    /// Entries rendered by the host-owned View panel.
    pub outputs: Vec<ViewerOutputPanelEntry>,
}

/// One selectable output exposed by a node's View panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerOutputPanelEntry {
    /// Stable output identifier used by panel actions.
    pub id: String,
    /// User-facing output label.
    pub label: String,
    /// Whether the output is currently selected for presentation.
    pub selected: bool,
}

/// Edit emitted by a node's View panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerOutputPanelAction {
    /// Changes the presentation selection of an output.
    SetSelected {
        /// Stable output identifier.
        id: String,
        /// New selection state.
        selected: bool,
    },
}

impl ViewerOutputControl {
    /// Creates a selectable output control.
    ///
    /// # Parameters
    /// - `default_selected`: Whether the output starts selected in a new node.
    /// - `indicator_outputs`: Output indexes whose header indicators follow selection.
    pub fn new(default_selected: bool, indicator_outputs: impl IntoIterator<Item = usize>) -> Self {
        Self::Selectable {
            default_selected,
            indicator_outputs: indicator_outputs.into_iter().collect(),
        }
    }
}

/// Deserializes node-owned persisted state from the generic graph document.
///
/// Concrete features call this at their load boundary and report its error to the
/// user; generic compiler and viewer code never interprets concrete state.
pub fn parse_state<T: serde::de::DeserializeOwned>(state: &serde_json::Value) -> Result<T, String> {
    serde_json::from_value(state.clone()).map_err(|error| format!("invalid node state: {error}"))
}

/// Restricted runtime services available while a concrete node materializes.
///
/// The context exposes generic storage and execution capabilities only; it never
/// reveals compiler implementation state or host-specific handles.
pub trait NodeBuildContext {
    /// Returns the run-owned derived-lane catalog.
    fn derived_lanes(&self) -> &DerivedLanes;
    /// Returns the configured retention policy for derived data.
    fn derived_data_retention(&self) -> DerivedDataRetention;
    /// Returns persistent word-store configuration for a collected output member.
    fn derived_word_cache(&self, member: usize) -> Option<&PersistentStoreConfig>;
    /// Returns run-owned sampling-point storage for `runtime_name` when requested.
    fn sampling_points(&self, runtime_name: &str) -> Option<SamplingPointStore>;
    /// Returns the bounded executor available to the node.
    fn work_executor(&self) -> Arc<dyn WorkExecutor> {
        Arc::new(InlineWorkExecutor)
    }
    /// Returns the artifact repository selected by composition.
    fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::new(MemoryArtifactRepository::new())
    }
    /// Resolves a host-owned timeline marker reference when one is available.
    fn timeline_marker(&self, _reference: TimelineMarkerReference) -> Option<TimelineMarker> {
        None
    }
}

/// Host-owned timeline position requested by a concrete graph node.
///
/// The compiler transports this key without knowing which widget owns the
/// referenced value. The application supplies the current value when it
/// creates a run.
/// Host-owned timeline position requested by a concrete graph node.
///
/// The compiler transports this key without knowing which widget owns the
/// referenced value. The application supplies the current value when it
/// creates a run.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TimelineMarkerReference {
    /// A numbered cursor provided by the host timeline UI.
    Cursor {
        /// One-based cursor number in the host's timeline UI.
        number: u32,
    },
}

/// One host-owned timeline position available to a node reference control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerReferenceChoice {
    /// Reference resolved by the host.
    pub reference: TimelineMarkerReference,
    /// User-facing description of the reference.
    pub label: String,
    /// Current referenced position in the shared nanosecond time domain.
    pub timestamp_ns: u64,
}

impl TimelineMarkerReferenceChoice {
    /// Creates a host-provided marker-reference choice.
    pub fn new(
        reference: TimelineMarkerReference,
        label: impl Into<String>,
        timestamp_ns: u64,
    ) -> Self {
        Self {
            reference,
            label: label.into(),
            timestamp_ns,
        }
    }
}

/// Identifies whether a source supplies finite or continuously acquired data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDataLifecycleKind {
    /// A finite source backed by an imported or persisted capture.
    File,
    /// A source that acquires or follows live capture data.
    Live,
}

/// Source-preparation behavior requested by a concrete capture source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceDataLifecycle {
    /// Whether the source is finite or live.
    pub kind: SourceDataLifecycleKind,
    /// Whether source preparation reads all source data before execution.
    pub preload: bool,
    /// Whether prepared results are eligible for cache reuse.
    pub cache: bool,
    /// Whether source preparation builds a waveform index.
    pub index: bool,
}

impl SourceDataLifecycle {
    /// Describes the source-preparation work requested by a concrete source.
    ///
    /// # Parameters
    /// - `kind`: Whether the source is finite or acquires live data.
    /// - `preload`: Whether preparation reads all source data before execution.
    /// - `cache`: Whether a prepared result can be reused.
    /// - `index`: Whether preparation constructs a waveform index.
    pub const fn new(
        kind: SourceDataLifecycleKind,
        preload: bool,
        cache: bool,
        index: bool,
    ) -> Self {
        Self {
            kind,
            preload,
            cache,
            index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Declares how a node's sampled inputs can be reconstructed for presentation.
pub struct SamplingOverlayDescriptor {
    /// Input-definition index of the sampling clock.
    pub clock_input: usize,
    /// Input-definition indexes grouped as sampled data sources.
    pub sampled_input_groups: Vec<usize>,
    /// Existing retained output that records the same sampling decisions.
    pub retained_word_source: Option<RetainedWordSamplingSource>,
}

/// A retained word output whose events are identical to the node's sampling
/// decisions. The compiler may use the indexed output lane instead of
/// persisting a duplicate sampling-point stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedWordSamplingSource {
    /// Output-definition index of the retained word lane.
    pub output: usize,
    /// Clock level at which the source accepts samples.
    pub clock_high: bool,
}

/// One resolved upstream connection supplied to a runtime builder.
#[derive(Debug, Clone)]
pub struct ResolvedInput {
    /// Negotiated generic runtime kind.
    pub kind: PortKind,
    /// Stable source runtime name.
    pub source: String,
    /// Source node identity in the graph document.
    pub source_node: NodeId,
    /// Source output-definition index.
    pub source_output: usize,
    /// User-visible source-node title for diagnostics.
    pub source_node_title: String,
    /// User-visible source output-socket title for diagnostics and provenance.
    pub source_output_title: String,
    /// Optional concrete word display format chosen by the source.
    pub word_display_format: Option<String>,
    /// Optional explicit rendering metadata for one track of a compound lane.
    pub lane_presentation: Option<LanePresentationDescriptor>,
    /// Optional fallback rendering metadata when no compound-lane track is supplied.
    pub default_lane_presentation: Option<DefaultLanePresentationDescriptor>,
    /// Optional table-column contract emitted by a decoder source.
    pub decoder_table_column: Option<DecoderTableColumnDescriptor>,
    /// Optional waveform-viewer channel index for a capture source.
    pub capture_channel: Option<usize>,
}

/// Presentation badge attached to a rendered lane or track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneBadgeDescriptor {
    /// Short label rendered with the lane.
    pub label: String,
    /// RGB badge color.
    pub color: [u8; 3],
}

impl LaneBadgeDescriptor {
    /// Creates a badge descriptor with a display label and RGB color.
    pub fn new(label: impl Into<String>, color: [u8; 3]) -> Self {
        Self {
            label: label.into(),
            color,
        }
    }
}

/// Application-neutral description of one track in a compound retained lane.
#[derive(Clone, Debug, PartialEq)]
pub struct LanePresentationDescriptor {
    /// Stable compound-lane group key.
    pub group_key: String,
    /// Stable track key within the group.
    pub track_key: String,
    /// Relative ordering among tracks in the group.
    pub track_order: usize,
    /// Height multiplier requested for the track.
    pub relative_height: f32,
    /// Badge displayed beside the track.
    pub badge: LaneBadgeDescriptor,
    /// Stable renderer registration key.
    pub renderer_key: String,
}

impl LanePresentationDescriptor {
    /// Creates explicit presentation metadata for one compound-lane track.
    pub fn new(
        group_key: impl Into<String>,
        track_key: impl Into<String>,
        track_order: usize,
        relative_height: f32,
        badge: LaneBadgeDescriptor,
        renderer_key: impl Into<String>,
    ) -> Self {
        Self {
            group_key: group_key.into(),
            track_key: track_key.into(),
            track_order,
            relative_height,
            badge,
            renderer_key: renderer_key.into(),
        }
    }
}

/// Default presentation metadata for a payload kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultLanePresentationDescriptor {
    /// Badge for the singleton lane.
    pub badge: LaneBadgeDescriptor,
    /// Stable renderer registration key.
    pub renderer_key: String,
}

impl DefaultLanePresentationDescriptor {
    /// Creates default presentation metadata for a payload kind.
    ///
    /// # Parameters
    /// - `badge`: Display badge for a singleton lane.
    /// - `renderer_key`: Registered renderer used for the lane's payload.
    pub fn new(badge: LaneBadgeDescriptor, renderer_key: impl Into<String>) -> Self {
        Self {
            badge,
            renderer_key: renderer_key.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Resolved inputs indexed by input definition and variadic member index.
pub struct ResolvedInputs(HashMap<(usize, usize), ResolvedInput>);

impl ResolvedInputs {
    /// Returns the resolved input for one definition/member pair.
    pub fn get(&self, def_index: usize, member_index: usize) -> Option<&ResolvedInput> {
        self.0.get(&(def_index, member_index))
    }

    /// Returns the kind resolved for the first member of an input definition.
    pub fn kind(&self, def_index: usize) -> Option<PortKind> {
        self.0.get(&(def_index, 0)).map(|input| input.kind)
    }

    /// Returns the number of resolved members for an input definition.
    pub fn member_count(&self, def_index: usize) -> usize {
        self.0.keys().filter(|(def, _)| *def == def_index).count()
    }

    /// Returns resolved members in increasing member-index order.
    pub fn members(&self, def_index: usize) -> Vec<(usize, &ResolvedInput)> {
        let mut members = self
            .0
            .iter()
            .filter(|((def, _), _)| *def == def_index)
            .map(|((_, member), input)| (*member, input))
            .collect::<Vec<_>>();
        members.sort_by_key(|(member, _)| *member);
        members
    }

    #[doc(hidden)]
    /// Inserts or replaces a registered viewer-output entry by its stable identity.
    pub fn insert(&mut self, def_index: usize, member_index: usize, input: ResolvedInput) {
        self.0.insert((def_index, member_index), input);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One capture channel exposed by a simple trigger editor.
pub struct SimpleTriggerChannel {
    /// Provider-stable capture channel identity.
    pub channel_id: CaptureChannelId,
    /// Corresponding waveform-viewer channel index.
    pub viewer_channel: usize,
    /// User-facing channel name.
    pub name: String,
    /// Whether the channel participates in the configured capture.
    pub enabled: bool,
    /// Current simple trigger condition for the channel.
    pub condition: SimpleTriggerCondition,
}

/// Validated trigger configuration exposed by a live-capture node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerConfigurationFeature {
    schema: Arc<TriggerEditorSchema>,
    program: Option<TriggerProgram>,
    channels: Vec<SimpleTriggerChannel>,
}

impl TriggerConfigurationFeature {
    /// Creates validated trigger configuration contributed by a live-capture node.
    ///
    /// The schema is validated against the enabled channels whenever an advanced
    /// program is supplied. Channel and viewer identities must be unique.
    ///
    /// # Parameters
    /// - `schema`: Trigger-program grammar and validation rules provided by the source.
    /// - `program`: Optional advanced program; `None` represents free-run capture.
    /// - `channels`: Capture channels, their viewer mapping, and simple-trigger state.
    pub fn new(
        schema: TriggerEditorSchema,
        program: Option<TriggerProgram>,
        channels: Vec<SimpleTriggerChannel>,
    ) -> Result<Self, String> {
        let all_channel_ids = channels
            .iter()
            .map(|channel| channel.channel_id.clone())
            .collect::<Vec<_>>();
        let channel_ids = channels
            .iter()
            .filter(|channel| channel.enabled)
            .map(|channel| channel.channel_id.clone())
            .collect::<Vec<_>>();
        if all_channel_ids.iter().collect::<HashSet<_>>().len() != all_channel_ids.len() {
            return Err("trigger configuration channel identities must be unique".into());
        }
        if channels
            .iter()
            .map(|channel| channel.viewer_channel)
            .collect::<HashSet<_>>()
            .len()
            != channels.len()
        {
            return Err("trigger configuration viewer channels must be unique".into());
        }
        if let Some(program) = &program {
            schema
                .validate_program(program, &channel_ids)
                .map_err(|error| error.to_string())?;
        }
        Ok(Self {
            schema: Arc::new(schema),
            program,
            channels,
        })
    }

    /// Returns the provider's trigger editor schema.
    pub fn schema(&self) -> &TriggerEditorSchema {
        &self.schema
    }

    /// Returns the current advanced trigger program, or `None` for free run.
    pub fn program(&self) -> Option<&TriggerProgram> {
        self.program.as_ref()
    }

    /// Returns the editable capture-channel trigger state.
    pub fn channels(&self) -> &[SimpleTriggerChannel] {
        &self.channels
    }
}

/// User edit of trigger state owned by a live-capture node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveCaptureEdit {
    /// Changes a single channel's simple trigger condition.
    SetSimpleTrigger {
        /// Channel to edit.
        channel_id: CaptureChannelId,
        /// Replacement simple condition.
        condition: SimpleTriggerCondition,
    },
    /// Replaces the complete advanced trigger program.
    SetTriggerProgram {
        /// Program to install, or `None` for free run.
        program: Option<TriggerProgram>,
    },
}

/// One persisted timeline marker exposed by a concrete node to an application host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerDescriptor {
    /// Stable node-owned marker identifier.
    pub id: String,
    /// User-visible marker name.
    pub name: String,
    /// Marker position in the shared nanosecond time domain.
    pub timestamp_ns: u64,
}

impl TimelineMarkerDescriptor {
    /// Creates a node-owned persisted timeline marker.
    ///
    /// # Parameters
    /// - `id`: Stable identifier used to route later host edits to this marker.
    /// - `name`: Display name shown by the host timeline UI.
    /// - `timestamp_ns`: Initial position in the shared nanosecond time domain.
    pub fn new(id: impl Into<String>, name: impl Into<String>, timestamp_ns: u64) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            timestamp_ns,
        }
    }
}

/// Host-owned edit routed back to the node that contributed a timeline marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineMarkerEdit {
    /// Moves one marker to a new shared-domain timestamp.
    SetTimestamp {
        /// Stable marker identifier.
        id: String,
        /// New marker position in nanoseconds.
        timestamp_ns: u64,
    },
}

/// A concrete node control bound to one of the host-owned timeline positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerReferenceBindingDescriptor {
    /// Stable control identifier within the node.
    pub id: String,
    /// Currently selected host reference, if any.
    pub selected: Option<TimelineMarkerReference>,
    /// Resolved timestamp for the current selection.
    pub timestamp_ns: u64,
    /// Host-provided reference choices.
    pub choices: Vec<TimelineMarkerReferenceChoice>,
}

/// Host-owned choices routed to the concrete node that presents the control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineMarkerReferenceBindingEdit {
    /// Replaces the host-provided choices for one node control.
    Synchronize {
        /// Stable control identifier.
        id: String,
        /// Current choices to present.
        choices: Vec<TimelineMarkerReferenceChoice>,
    },
}

/// One in-memory capture channel provided directly to the viewer.
pub struct CapturePresentationSignal {
    /// Viewer channel index.
    pub index: usize,
    /// User-facing channel name.
    pub name: String,
    /// Logic level before the first transition.
    pub initial: bool,
    /// Increasing `(time_us, level_after)` transitions.
    pub transitions: Vec<(f64, bool)>,
}

/// Capture data or channel metadata supplied to the waveform viewer.
pub enum CapturePresentation {
    /// A deferred indexed capture prepared through the supplied factory.
    Indexed {
        /// Stable raw-capture identity.
        identity: SourceIdentity,
        /// Factory used by source preparation to open or build the index.
        factory: Box<dyn CaptureIndexFactory>,
    },
    /// A finite capture already held in memory.
    InMemory {
        /// Waveform channels and transitions to render.
        signals: Vec<CapturePresentationSignal>,
        /// Capture duration, in microseconds.
        duration_us: f64,
    },
    /// Channel names without sample data.
    Channels(
        /// `(viewer_channel_index, display_name)` entries exposed by the source.
        Vec<(usize, String)>,
    ),
}

/// Cache-reuse classification supplied by a concrete capture source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureCacheIdentity {
    #[default]
    /// The node is not a capture source.
    NotCapture,
    /// Capture identity changes with each run and cannot be reused.
    Dynamic,
    /// Stable cache identity supplied by the concrete source.
    Stable([u8; 32]),
}

/// How a decoder output contributes values to a result-table cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoderTableCellMode {
    /// One output produces one independent table cell.
    Single,
    /// This output shares a named cell with related outputs.
    Joined(String),
}

/// Presentation contract for one decoder result-table column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoderTableColumnDescriptor {
    /// Stable identifier for the decoder output that provides this column.
    pub source_key: String,
    /// Stable identifier for this column within its source.
    pub column_key: String,
    /// User-facing column heading.
    pub label: String,
    /// Relative display order among columns.
    pub order: usize,
    /// Whether a value in this column begins a new table row.
    pub row_anchor: bool,
    /// Whether the column owns a cell or joins a named cell with related outputs.
    pub cell_mode: DecoderTableCellMode,
    /// Stable presentation-track key associated with the column.
    pub track_key: String,
    /// Stable renderer key used to display values from the column.
    pub renderer_key: String,
}

impl DecoderTableColumnDescriptor {
    #[allow(clippy::too_many_arguments)]
    /// Creates a decoder-table column contract.
    ///
    /// # Parameters
    /// - `source_key`: Stable decoder-output identifier.
    /// - `column_key`: Stable column identifier within `source_key`.
    /// - `label`: Heading shown to the user.
    /// - `order`: Relative display order.
    /// - `row_anchor`: Whether this column starts each decoder event row.
    /// - `cell_mode`: Independent or joined-cell behavior for the column.
    /// - `track_key`: Associated lane-presentation track identifier.
    /// - `renderer_key`: Registered renderer used for column values.
    pub fn new(
        source_key: impl Into<String>,
        column_key: impl Into<String>,
        label: impl Into<String>,
        order: usize,
        row_anchor: bool,
        cell_mode: DecoderTableCellMode,
        track_key: impl Into<String>,
        renderer_key: impl Into<String>,
    ) -> Self {
        Self {
            source_key: source_key.into(),
            column_key: column_key.into(),
            label: label.into(),
            order,
            row_anchor,
            cell_mode,
            track_key: track_key.into(),
            renderer_key: renderer_key.into(),
        }
    }
}
