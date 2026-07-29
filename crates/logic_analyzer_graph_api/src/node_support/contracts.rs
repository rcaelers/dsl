use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use node_graph::api::NodeId;
use signal_processing::{
    CaptureChannelId, CaptureIndexFactory, DerivedDataRetention, DerivedLanes,
    PersistentStoreConfig, SamplingActivity, SamplingEdge, SimpleTriggerCondition, TimelineMarker,
    TriggerEditorSchema, TriggerProgram,
};

use super::port::PortKind;

/// Logic-analyzer presentation choice contributed by a concrete graph node.
/// Generic graph widgets receive only a transient, application-neutral UI
/// model derived from this contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerOutputControl {
    Hidden,
    Selectable {
        default_selected: bool,
        indicator_outputs: Vec<usize>,
    },
}

/// Logic-analyzer-owned data supplied to the node's viewer-output panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewerOutputPanelModel {
    pub outputs: Vec<ViewerOutputPanelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerOutputPanelEntry {
    pub id: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerOutputPanelAction {
    SetSelected { id: String, selected: bool },
}

impl ViewerOutputControl {
    pub fn new(default_selected: bool, indicator_outputs: impl IntoIterator<Item = usize>) -> Self {
        Self::Selectable {
            default_selected,
            indicator_outputs: indicator_outputs.into_iter().collect(),
        }
    }
}

pub fn parse_state<T: serde::de::DeserializeOwned>(state: &serde_json::Value) -> Result<T, String> {
    serde_json::from_value(state.clone()).map_err(|error| format!("invalid node state: {error}"))
}

pub trait NodeBuildContext {
    fn derived_lanes(&self) -> &DerivedLanes;
    fn derived_data_retention(&self) -> DerivedDataRetention;
    fn derived_word_cache(&self, member: usize) -> Option<&PersistentStoreConfig>;
    fn sampling_activity(&self, runtime_name: &str, input: usize) -> Option<SamplingActivity>;
    fn timeline_marker(&self, _reference: TimelineMarkerReference) -> Option<TimelineMarker> {
        None
    }
}

/// Host-owned timeline position requested by a concrete graph node.
///
/// The compiler transports this key without knowing which widget owns the
/// referenced value. The application supplies the current value when it
/// creates a run.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TimelineMarkerReference {
    Cursor { number: u32 },
}

/// One host-owned timeline position available to a node reference control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerReferenceChoice {
    pub reference: TimelineMarkerReference,
    pub label: String,
    pub timestamp_ns: u64,
}

impl TimelineMarkerReferenceChoice {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDataLifecycleKind {
    File,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceDataLifecycle {
    pub kind: SourceDataLifecycleKind,
    pub preload: bool,
    pub cache: bool,
    pub index: bool,
}

impl SourceDataLifecycle {
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
pub struct SamplingOverlayDescriptor {
    pub clock_input: usize,
    pub sampled_input_groups: Vec<usize>,
    pub edge: SamplingEdge,
    pub qualifiers: Vec<SamplingQualifierDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplingQualifierDescriptor {
    pub input: usize,
    pub active_level: bool,
    pub runtime_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedInput {
    pub kind: PortKind,
    pub source: String,
    pub source_node: NodeId,
    pub source_output: usize,
    pub source_node_title: String,
    pub word_display_format: Option<String>,
    pub lane_presentation: Option<LanePresentationDescriptor>,
    pub default_lane_presentation: Option<DefaultLanePresentationDescriptor>,
    pub decoder_table_column: Option<DecoderTableColumnDescriptor>,
    pub capture_channel: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneBadgeDescriptor {
    pub label: String,
    pub color: [u8; 3],
}

impl LaneBadgeDescriptor {
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
    pub group_key: String,
    pub track_key: String,
    pub track_order: usize,
    pub relative_height: f32,
    pub badge: LaneBadgeDescriptor,
    pub renderer_key: String,
}

impl LanePresentationDescriptor {
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
    pub badge: LaneBadgeDescriptor,
    pub renderer_key: String,
}

impl DefaultLanePresentationDescriptor {
    pub fn new(badge: LaneBadgeDescriptor, renderer_key: impl Into<String>) -> Self {
        Self {
            badge,
            renderer_key: renderer_key.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedInputs(HashMap<(usize, usize), ResolvedInput>);

impl ResolvedInputs {
    pub fn get(&self, def_index: usize, member_index: usize) -> Option<&ResolvedInput> {
        self.0.get(&(def_index, member_index))
    }

    pub fn kind(&self, def_index: usize) -> Option<PortKind> {
        self.0.get(&(def_index, 0)).map(|input| input.kind)
    }

    pub fn member_count(&self, def_index: usize) -> usize {
        self.0.keys().filter(|(def, _)| *def == def_index).count()
    }

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
    pub fn insert(&mut self, def_index: usize, member_index: usize, input: ResolvedInput) {
        self.0.insert((def_index, member_index), input);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleTriggerChannel {
    pub channel_id: CaptureChannelId,
    pub viewer_channel: usize,
    pub name: String,
    pub enabled: bool,
    pub condition: SimpleTriggerCondition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerConfigurationFeature {
    schema: Arc<TriggerEditorSchema>,
    program: Option<TriggerProgram>,
    channels: Vec<SimpleTriggerChannel>,
}

impl TriggerConfigurationFeature {
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

    pub fn schema(&self) -> &TriggerEditorSchema {
        &self.schema
    }

    pub fn program(&self) -> Option<&TriggerProgram> {
        self.program.as_ref()
    }

    pub fn channels(&self) -> &[SimpleTriggerChannel] {
        &self.channels
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveCaptureEdit {
    SetSimpleTrigger {
        channel_id: CaptureChannelId,
        condition: SimpleTriggerCondition,
    },
    SetTriggerProgram {
        program: Option<TriggerProgram>,
    },
}

/// One persisted timeline marker exposed by a concrete node to an application host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerDescriptor {
    pub id: String,
    pub name: String,
    pub timestamp_ns: u64,
}

impl TimelineMarkerDescriptor {
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
    SetTimestamp { id: String, timestamp_ns: u64 },
}

/// A concrete node control bound to one of the host-owned timeline positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineMarkerReferenceBindingDescriptor {
    pub id: String,
    pub selected: Option<TimelineMarkerReference>,
    pub timestamp_ns: u64,
    pub choices: Vec<TimelineMarkerReferenceChoice>,
}

/// Host-owned choices routed to the concrete node that presents the control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineMarkerReferenceBindingEdit {
    Synchronize {
        id: String,
        choices: Vec<TimelineMarkerReferenceChoice>,
    },
}

pub struct CapturePresentationSignal {
    pub index: usize,
    pub name: String,
    pub initial: bool,
    pub transitions: Vec<(f64, bool)>,
}

pub enum CapturePresentation {
    Indexed {
        identity: PathBuf,
        factory: Box<dyn CaptureIndexFactory>,
    },
    InMemory {
        signals: Vec<CapturePresentationSignal>,
        duration_us: f64,
    },
    Channels(Vec<(usize, String)>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureCacheIdentity {
    #[default]
    NotCapture,
    Dynamic,
    Stable([u8; 32]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoderTableCellMode {
    Single,
    Joined(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoderTableColumnDescriptor {
    pub source_key: String,
    pub column_key: String,
    pub label: String,
    pub order: usize,
    pub row_anchor: bool,
    pub cell_mode: DecoderTableCellMode,
    pub track_key: String,
    pub renderer_key: String,
}

impl DecoderTableColumnDescriptor {
    #[allow(clippy::too_many_arguments)]
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
