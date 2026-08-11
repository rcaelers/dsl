//! Application-shell composition and per-frame dispatch.
//!
//! `App` owns the editor/viewer widgets, shell panels, host ports, notifications, and the four
//! owner objects that maintain graph-run, capture-analysis, presentation-catalog, and
//! timeline-marker invariants. The crate-root `App` re-export is its supported facade. This module
//! may compose UI-owned services and portable widget/runtime contracts; it does not own the inner
//! lifecycle state, concrete graph nodes, processing behavior, target selection, or host adapters.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use input_bindings::{InputBindings, PointerButtonName, PointerGesture, Trigger};
use logic_analyzer_graph_capabilities::node_support::{
    CapturePresentationSignal, LiveCaptureEdit, TimelineMarkerEdit as GraphTimelineMarkerEdit,
    TimelineMarkerReference, TimelineMarkerReferenceBindingEdit, TimelineMarkerReferenceChoice,
    ViewerOutputPanelAction, ViewerOutputPanelEntry, ViewerOutputPanelModel,
};
use logic_analyzer_graph_plan as plan;
use logic_analyzer_graph_runtime as runtime;
use logic_analyzer_viewer::{
    LogicAnalyzerViewer, SimpleTriggerEdit, SimpleTriggerLane, TimeCursor,
    TimelineMarkerEdit as ViewerTimelineMarkerEdit, ViewerLaneGroupId, ViewerRowHeight,
    ViewerRowHeightSettings, ViewerRowId, WaveformPresentationRegistry,
};
use node_graph::api::{
    GraphState, NodeBadge, NodeId, PanelDataProvider, PanelTabDef, SocketDirection, SocketId,
    SocketIndicatorPresentation,
};
use node_graph::{NodeContextAction, NodeGraphWidget};
use panel_layout::{BoundaryInteraction, PanelIcon, PanelLayout, PanelSlot, PanelSpec};
use trigger_editor::{TriggerEditor, TriggerEditorChannel};

use crate::about::AboutWindow;
use crate::app_services::AppServiceParts;
use crate::capture_analysis_lifecycle::CaptureAnalysisLifecycle;
use crate::capture_provider::{
    CaptureDataProvider, CapturePresentationUpdate, CaptureProviderPoll, LiveCaptureProvider,
};
use crate::collected_output_presentation::waveform_presentation_registry;
use crate::decoder_panel::{DecoderPanels, DecoderTableRegistry};
use crate::decoder_table_presentation::decoder_table_registry;
use crate::graph_run_lifecycle::{GraphRunLifecycle, GraphRunPoll};
use crate::graph_service::PreparedGraphRevision;
use crate::host_service::HostService;
use crate::live_capture::{
    CaptureAnalysisAttachment, CaptureAvailability, CaptureCoordinator, CaptureCoordinatorContract,
    CaptureReplayAttachment, ConfigurationEpochResolution, capture_availability,
};
use crate::memory_panel::{
    CaptureStorageBacking, CaptureStorageSnapshot, DerivedSignalStorageSnapshot, MemoryPanel,
    MemoryPanelSnapshot, MemoryServiceSnapshot,
};
use crate::node_catalog_service::NodeCatalogService;
use crate::output_downloads::OutputDownloadsWindow;
use crate::panel_presentation::{
    DECODER_PANEL_ICON, LOG_PANEL_ICON, LOGIC_ANALYZER_PANEL_ICON, MEMORY_PANEL_ICON,
    NODE_GRAPH_PANEL_ICON, TRIGGERS_PANEL_ICON, WATCHES_PANEL_ICON,
};
use crate::plugin_panel::{PluginPanelIcon, PluginPanelRegistry, PluginPanelsState};
use crate::preferences::PreferencesWindow;
use crate::presentation_catalogs::PresentationCatalogs;
use crate::sampling_overlay_presentation::sampling_overlay_presentation;
use crate::symbol_fonts::bundled_symbol_fonts;
use crate::timeline_marker_bindings::TimelineMarkerBindings;
use crate::toast::{ToastSource, Toasts};
use crate::viewer_selection::{
    output_subscription_plan, set_viewer_output_selected, synchronize_viewer_compatibility,
    viewer_output_selections,
};

const VIEWER_OUTPUT_PANEL_ID: &str = "viewer-outputs";
const VIEWER_SOCKET_INDICATOR_OWNER: &str = "logic-analyzer.viewer";

struct ViewerSocketIndicator;

struct ViewerOutputPanelData {
    models: HashMap<NodeId, ViewerOutputPanelModel>,
}

impl PanelDataProvider for ViewerOutputPanelData {
    fn panel_data(
        &self,
        node: NodeId,
        panel_id: &str,
    ) -> Option<&(dyn std::any::Any + Send + Sync)> {
        (panel_id == VIEWER_OUTPUT_PANEL_ID)
            .then(|| self.models.get(&node))
            .flatten()
            .map(|model| model as &(dyn std::any::Any + Send + Sync))
    }
}

impl SocketIndicatorPresentation for ViewerSocketIndicator {
    fn size(&self, zoom: f32) -> egui::Vec2 {
        egui::Vec2::new(12.0 * zoom, 7.4 * zoom)
    }

    fn draw(&self, painter: &egui::Painter, rect: egui::Rect, zoom: f32) {
        let center = rect.center();
        let half_width = rect.width() * 0.5;
        let half_height = rect.height() * 0.5;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::Pos2::new(center.x - half_width, center.y),
                egui::Pos2::new(center.x - half_width * 0.45, center.y - half_height),
                egui::Pos2::new(center.x + half_width * 0.45, center.y - half_height),
                egui::Pos2::new(center.x + half_width, center.y),
                egui::Pos2::new(center.x + half_width * 0.45, center.y + half_height),
                egui::Pos2::new(center.x - half_width * 0.45, center.y + half_height),
            ],
            egui::Color32::from_black_alpha(210),
            egui::Stroke::new(1.2 * zoom, egui::Color32::from_rgb(190, 225, 205)),
        ));
        painter.circle_filled(center, 1.65 * zoom, egui::Color32::from_rgb(110, 205, 145));
    }
}

const SAMPLING_OVERLAY_EXTENSION: &str = "logic_analyzer_ui.sampling_overlay";
const VIEWER_LANE_ORDER_EXTENSION: &str = "logic_analyzer_ui.viewer_lane_order";
const VIEWER_LANE_HEIGHTS_EXTENSION: &str = "logic_analyzer_ui.viewer_lane_heights";
const TIMELINE_CURSORS_EXTENSION: &str = "logic_analyzer_ui.timeline_cursors";
const PANEL_LAYOUT_EXTENSION: &str = "logic_analyzer_ui.panel_layout";

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct SavedPanelLayout {
    layout: panel_layout::PanelLayoutState,
    #[serde(default)]
    decoder_panels: crate::decoder_panel::DecoderPanelsState,
    #[serde(default)]
    plugin_panels: PluginPanelsState,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
struct SavedTimelineCursors {
    #[serde(default = "timeline_cursor_schema_version")]
    version: u32,
    #[serde(default)]
    cursors: Vec<SavedTimeCursor>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
struct SavedTimeCursor {
    number: u32,
    time_us: f64,
}

fn timeline_cursor_schema_version() -> u32 {
    1
}

#[derive(Debug)]
pub(crate) enum TimelineCursorExtensionError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
}

impl fmt::Display for TimelineCursorExtensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => error.fmt(formatter),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "timeline-cursor extension version {version} is not supported; it was preserved unchanged"
            ),
        }
    }
}

impl std::error::Error for TimelineCursorExtensionError {}

impl From<serde_json::Error> for TimelineCursorExtensionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) enum SavedViewerRow {
    Channel(usize),
    Derived(String),
}

impl From<&ViewerRowId> for SavedViewerRow {
    fn from(value: &ViewerRowId) -> Self {
        match value {
            ViewerRowId::Channel(index) => Self::Channel(*index),
            ViewerRowId::Derived(group) => Self::Derived(group.as_str().to_owned()),
        }
    }
}

impl From<&SavedViewerRow> for ViewerRowId {
    fn from(value: &SavedViewerRow) -> Self {
        match value {
            SavedViewerRow::Channel(index) => Self::Channel(*index),
            SavedViewerRow::Derived(group) => Self::Derived(ViewerLaneGroupId::new(group.clone())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
struct SavedViewerRowHeight {
    row: SavedViewerRow,
    scale: f32,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
struct SavedViewerRowHeightSettings {
    #[serde(default = "default_viewer_row_height_scale")]
    global_scale: f32,
    #[serde(default)]
    rows: Vec<SavedViewerRowHeight>,
}

fn default_viewer_row_height_scale() -> f32 {
    1.0
}

impl From<&ViewerRowHeightSettings> for SavedViewerRowHeightSettings {
    fn from(value: &ViewerRowHeightSettings) -> Self {
        Self {
            global_scale: value.global_scale,
            rows: value
                .rows
                .iter()
                .map(|height| SavedViewerRowHeight {
                    row: SavedViewerRow::from(&height.row),
                    scale: height.scale,
                })
                .collect(),
        }
    }
}

impl From<&SavedViewerRowHeightSettings> for ViewerRowHeightSettings {
    fn from(value: &SavedViewerRowHeightSettings) -> Self {
        Self {
            global_scale: value.global_scale,
            rows: value
                .rows
                .iter()
                .map(|height| ViewerRowHeight {
                    row: ViewerRowId::from(&height.row),
                    scale: height.scale,
                })
                .collect(),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum SavedSamplingOverlaySelection {
    Multiple(Vec<NodeId>),
    LegacySingle(NodeId),
}

fn saved_sampling_overlays(graph: &GraphState) -> Result<(Vec<NodeId>, bool), serde_json::Error> {
    let selection = graph.extension::<SavedSamplingOverlaySelection>(SAMPLING_OVERLAY_EXTENSION)?;
    Ok(match selection {
        Some(SavedSamplingOverlaySelection::Multiple(selected)) => (selected, false),
        Some(SavedSamplingOverlaySelection::LegacySingle(selected)) => (vec![selected], true),
        None => (Vec::new(), false),
    })
}

fn save_sampling_overlays(
    graph: &mut GraphState,
    selected: &[NodeId],
) -> Result<(), serde_json::Error> {
    if selected.is_empty() {
        graph.remove_extension(SAMPLING_OVERLAY_EXTENSION);
        Ok(())
    } else {
        graph.set_extension(SAMPLING_OVERLAY_EXTENSION, selected)
    }
}

fn saved_viewer_lane_order(graph: &GraphState) -> Result<Vec<SavedViewerRow>, serde_json::Error> {
    Ok(graph
        .extension(VIEWER_LANE_ORDER_EXTENSION)?
        .unwrap_or_default())
}

fn save_viewer_lane_order(
    graph: &mut GraphState,
    order: &[SavedViewerRow],
) -> Result<(), serde_json::Error> {
    if order.is_empty() {
        graph.remove_extension(VIEWER_LANE_ORDER_EXTENSION);
        Ok(())
    } else {
        graph.set_extension(VIEWER_LANE_ORDER_EXTENSION, order)
    }
}

fn saved_viewer_lane_heights(
    graph: &GraphState,
) -> Result<ViewerRowHeightSettings, serde_json::Error> {
    Ok(graph
        .extension::<SavedViewerRowHeightSettings>(VIEWER_LANE_HEIGHTS_EXTENSION)?
        .as_ref()
        .map(ViewerRowHeightSettings::from)
        .unwrap_or(ViewerRowHeightSettings {
            global_scale: 1.0,
            rows: Vec::new(),
        }))
}

fn save_viewer_lane_heights(
    graph: &mut GraphState,
    settings: &ViewerRowHeightSettings,
) -> Result<(), serde_json::Error> {
    if settings.global_scale == 1.0 && settings.rows.is_empty() {
        graph.remove_extension(VIEWER_LANE_HEIGHTS_EXTENSION);
        Ok(())
    } else {
        graph.set_extension(
            VIEWER_LANE_HEIGHTS_EXTENSION,
            SavedViewerRowHeightSettings::from(settings),
        )
    }
}

fn saved_timeline_cursors(
    graph: &GraphState,
) -> Result<Vec<TimeCursor>, TimelineCursorExtensionError> {
    let Some(saved) = graph.extension::<SavedTimelineCursors>(TIMELINE_CURSORS_EXTENSION)? else {
        return Ok(Vec::new());
    };
    if saved.version != timeline_cursor_schema_version() {
        return Err(TimelineCursorExtensionError::UnsupportedVersion(
            saved.version,
        ));
    }
    Ok(saved
        .cursors
        .into_iter()
        .map(|cursor| TimeCursor {
            number: cursor.number,
            time_us: cursor.time_us,
        })
        .collect())
}

pub(crate) fn supply_saved_timeline_cursors(
    graph: &GraphState,
    context: &mut runtime::GraphRunContext,
) -> Result<(), TimelineCursorExtensionError> {
    for cursor in saved_timeline_cursors(graph)? {
        context.set_timeline_marker(
            TimelineMarkerReference::Cursor {
                number: cursor.number,
            },
            signal_derived::TimelineMarker::new((cursor.time_us.max(0.0) * 1_000.0).round() as u64),
        );
    }
    Ok(())
}

fn save_timeline_cursors(
    graph: &mut GraphState,
    cursors: &[TimeCursor],
) -> Result<(), TimelineCursorExtensionError> {
    if let Some(saved) = graph.extension::<SavedTimelineCursors>(TIMELINE_CURSORS_EXTENSION)?
        && saved.version != timeline_cursor_schema_version()
    {
        return Err(TimelineCursorExtensionError::UnsupportedVersion(
            saved.version,
        ));
    }
    if cursors.is_empty() {
        graph.remove_semantic_extension(TIMELINE_CURSORS_EXTENSION);
        Ok(())
    } else {
        graph
            .set_semantic_extension(
                TIMELINE_CURSORS_EXTENSION,
                SavedTimelineCursors {
                    version: timeline_cursor_schema_version(),
                    cursors: cursors
                        .iter()
                        .map(|cursor| SavedTimeCursor {
                            number: cursor.number,
                            time_us: cursor.time_us,
                        })
                        .collect(),
                },
            )
            .map(|_| ())
            .map_err(Into::into)
    }
}

fn saved_panel_layout(graph: &GraphState) -> Result<Option<SavedPanelLayout>, serde_json::Error> {
    graph.extension(PANEL_LAYOUT_EXTENSION)
}

fn save_panel_layout(
    graph: &mut GraphState,
    layout: panel_layout::PanelLayoutState,
    decoder_panels: crate::decoder_panel::DecoderPanelsState,
    plugin_panels: PluginPanelsState,
) -> Result<(), serde_json::Error> {
    graph.set_extension(
        PANEL_LAYOUT_EXTENSION,
        SavedPanelLayout {
            layout,
            decoder_panels,
            plugin_panels,
        },
    )
}

/// A named graph document supplied by an application host for its Demos menu.
#[derive(Clone)]
pub struct DemoGraph {
    name: String,
    graph: GraphState,
}

impl DemoGraph {
    /// Creates a named graph document for the application's Demos menu.
    ///
    /// # Parameters
    /// - `name`: User-facing demo name.
    /// - `graph`: Complete graph document loaded when the demo is selected.
    pub fn new(name: impl Into<String>, graph: GraphState) -> Self {
        Self {
            name: name.into(),
            graph,
        }
    }

    /// Returns this demo's user-facing name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

pub struct App {
    pub(crate) node_graph: NodeGraphWidget,
    pub(crate) logic_analyzer: LogicAnalyzerViewer,
    pub(crate) input_bindings: Arc<InputBindings>,
    pub(crate) panel_layout: PanelLayout,
    pub(crate) graph_run: GraphRunLifecycle,
    pub(crate) decoded_block_cache: signal_derived::DecodedBlockCacheHandle,
    pub(crate) host_service: Box<dyn HostService>,
    pub(crate) host_ui_capabilities: crate::HostUiCapabilities,
    pub(crate) capture_analysis: CaptureAnalysisLifecycle,
    /// Transient one-off notifications (file loaded/saved, node(s)
    /// copied/pasted, live-edit results) — bottom-right, self-clearing.
    pub(crate) toasts: Toasts,
    pub(crate) platform: crate::app_platform::PlatformState,
    pub(crate) about: AboutWindow,
    pub(crate) output_downloads: OutputDownloadsWindow,
    pub(crate) preferences: PreferencesWindow,
    pub(crate) node_catalogs: Vec<Box<dyn NodeCatalogService>>,
    pub(crate) demo_graphs: Vec<DemoGraph>,
    /// Nodes badged with compile errors; cleared on the next Run.
    pub(crate) error_badges: Vec<NodeId>,
    pub(crate) presentations: PresentationCatalogs,
    pub(crate) memory_panel: MemoryPanel,
    pub(crate) timeline_markers: TimelineMarkerBindings,
    pub(crate) _worker_operation_executor: Rc<dyn platform_runtime::WorkerOperationExecutor>,
}

fn capture_storage_from_index(
    identity: &str,
    index: &dyn signal_capture::CaptureIndex,
    backing: CaptureStorageBacking,
) -> CaptureStorageSnapshot {
    let metadata = index.current_metadata();
    CaptureStorageSnapshot {
        name: if identity.is_empty() {
            index.display_name()
        } else {
            identity.to_owned()
        },
        status: if index.is_complete() {
            "Indexed capture ready"
        } else {
            "Capture index is growing"
        }
        .to_owned(),
        backing,
        channels: metadata.total_probes,
        total_samples: Some(metadata.total_samples),
        data_bytes: Some(
            metadata
                .total_samples
                .div_ceil(8)
                .saturating_mul(metadata.total_probes as u64),
        ),
        index_identity: Some(hex_identity(index.index_identity())),
        index_progress: None,
    }
}

fn hex_identity(identity: platform_artifacts::SourceIdentity) -> String {
    identity
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl App {
    fn refresh_simple_trigger_ui(&mut self) {
        let lanes = self
            .capture_analysis
            .trigger_configuration()
            .map(|configuration| configuration.feature.channels())
            .unwrap_or_default()
            .iter()
            .map(|channel| SimpleTriggerLane {
                channel: channel.viewer_channel,
                condition: channel.condition,
                enabled: channel.enabled,
            })
            .collect();
        self.logic_analyzer.set_simple_trigger_lanes(lanes);
    }

    fn refresh_trigger_configuration(&mut self) {
        match self
            .graph_run
            .service()
            .discover_trigger_configuration(self.node_graph.graph())
        {
            Ok(configuration) => self
                .capture_analysis
                .set_trigger_configuration(configuration),
            Err(error) => self
                .capture_analysis
                .set_trigger_configuration_error(error.to_string()),
        }
        self.refresh_simple_trigger_ui();
    }

    fn apply_simple_trigger_edit(&mut self, edit: SimpleTriggerEdit) {
        let request = self
            .capture_analysis
            .trigger_configuration()
            .and_then(|configuration| {
                configuration
                    .feature
                    .channels()
                    .iter()
                    .find(|channel| channel.viewer_channel == edit.channel)
                    .map(|channel| {
                        (
                            configuration.source_node,
                            LiveCaptureEdit::SetSimpleTrigger {
                                channel_id: channel.channel_id.clone(),
                                condition: edit.condition,
                            },
                        )
                    })
            });
        let Some((source_node, request)) = request else {
            self.toasts.error_from(
                ToastSource::panel("Triggers"),
                "That trigger input is no longer available",
            );
            self.refresh_simple_trigger_ui();
            return;
        };
        let toast_source = self.toast_source_for_node(source_node);
        let state = match self.graph_run.service().apply_live_capture_edit(
            self.node_graph.graph(),
            source_node,
            &request,
        ) {
            Ok(state) => state,
            Err(error) => {
                self.toasts.error_from(toast_source, error.to_string());
                self.refresh_simple_trigger_ui();
                return;
            }
        };
        if !self.node_graph.edit_node_state(source_node, state) {
            self.toasts.error_from(
                self.toast_source_for_node(source_node),
                "The trigger could not be changed while the graph is read-only",
            );
            self.refresh_simple_trigger_ui();
            return;
        }
        let availability = capture_availability(
            self.node_graph.graph(),
            self.graph_run.service(),
            self.capture_analysis
                .coordinator()
                .backend_unavailable_reason(),
        );
        self.capture_analysis.set_availability(availability);
        self.refresh_trigger_configuration();
    }

    fn apply_trigger_program_edit(
        &mut self,
        program: Option<logic_analyzer_trigger::TriggerProgram>,
    ) {
        let Some(source_node) = self
            .capture_analysis
            .trigger_configuration()
            .map(|configuration| configuration.source_node)
        else {
            self.toasts.error_from(
                ToastSource::panel("Triggers"),
                "The trigger-configurable source is no longer available",
            );
            self.refresh_trigger_configuration();
            return;
        };
        let toast_source = self.toast_source_for_node(source_node);
        let request = LiveCaptureEdit::SetTriggerProgram { program };
        let state = match self.graph_run.service().apply_live_capture_edit(
            self.node_graph.graph(),
            source_node,
            &request,
        ) {
            Ok(state) => state,
            Err(error) => {
                self.toasts.error_from(toast_source, error.to_string());
                self.refresh_trigger_configuration();
                return;
            }
        };
        if !self.node_graph.edit_node_state(source_node, state) {
            self.toasts.error_from(
                self.toast_source_for_node(source_node),
                "The trigger could not be changed while the graph is read-only",
            );
        }
        let availability = capture_availability(
            self.node_graph.graph(),
            self.graph_run.service(),
            self.capture_analysis
                .coordinator()
                .backend_unavailable_reason(),
        );
        self.capture_analysis.set_availability(availability);
        self.refresh_trigger_configuration();
    }

    fn refresh_timeline_markers(&mut self) {
        let discovered = match self
            .graph_run
            .service()
            .discover_timeline_markers(self.node_graph.graph())
        {
            Ok(discovered) => discovered,
            Err(error) => {
                let message = error.to_string();
                if self.timeline_markers.record_marker_error(message.clone()) {
                    self.toasts.error_from(
                        ToastSource::panel("Logic Analyzer"),
                        format!("Could not load timeline markers: {message}"),
                    );
                }
                self.logic_analyzer.set_timeline_markers(Vec::new());
                return;
            }
        };
        let markers = self.timeline_markers.replace_markers(discovered);
        self.logic_analyzer.set_timeline_markers(markers);
    }

    fn apply_timeline_marker_edit(&mut self, edit: ViewerTimelineMarkerEdit) {
        let Some((owner_node, local_id)) = self.timeline_markers.owner(&edit.id) else {
            self.toasts.error_from(
                ToastSource::panel("Logic Analyzer"),
                "That timeline marker is no longer available",
            );
            self.refresh_timeline_markers();
            return;
        };
        let request = GraphTimelineMarkerEdit::SetTimestamp {
            id: local_id,
            timestamp_ns: (edit.time_us.max(0.0) * 1_000.0).round() as u64,
        };
        let state = match self.graph_run.service().apply_timeline_marker_edit(
            self.node_graph.graph(),
            owner_node,
            &request,
        ) {
            Ok(state) => state,
            Err(error) => {
                self.toasts
                    .error_from(self.toast_source_for_node(owner_node), error.to_string());
                self.refresh_timeline_markers();
                return;
            }
        };
        if !self.node_graph.edit_node_state(owner_node, state) {
            self.toasts.error_from(
                self.toast_source_for_node(owner_node),
                "The timeline marker could not be moved while the graph is read-only",
            );
        }
        self.refresh_timeline_markers();
    }

    fn set_capture_preview(
        &mut self,
        identity: String,
        signals: Vec<CapturePresentationSignal>,
        duration_us: f64,
    ) {
        let resident_bytes = signals
            .iter()
            .map(|signal| {
                signal.name.capacity()
                    + signal.transitions.capacity() * std::mem::size_of::<(f64, bool)>()
            })
            .sum::<usize>() as u64;
        self.capture_analysis.set_storage(CaptureStorageSnapshot {
            name: identity,
            status: "In-memory capture ready".to_owned(),
            backing: CaptureStorageBacking::InMemory,
            channels: signals.len(),
            total_samples: None,
            data_bytes: Some(resident_bytes),
            index_identity: None,
            index_progress: None,
        });
        let channels = signals
            .into_iter()
            .map(|signal| logic_analyzer_viewer::ChannelSignal {
                index: signal.index,
                name: signal.name,
                initial: signal.initial,
                transitions: signal.transitions,
            })
            .collect();
        self.logic_analyzer
            .set_channels_with_duration(channels, duration_us);
    }

    fn set_indexed_capture(
        &mut self,
        identity: String,
        index: Box<dyn signal_capture::CaptureIndex>,
        growing: bool,
        planned_span_us: Option<f64>,
    ) {
        self.capture_analysis
            .set_storage(capture_storage_from_index(
                &identity,
                index.as_ref(),
                if growing {
                    CaptureStorageBacking::GrowingIndex
                } else {
                    CaptureStorageBacking::Indexed
                },
            ));
        if growing {
            self.logic_analyzer
                .set_growing_capture_with_planned_span(index, planned_span_us);
        } else {
            self.logic_analyzer.set_prepared_capture(identity, index);
        }
    }

    fn set_capture_channel_metadata(&mut self, identity: String, channels: Vec<(usize, String)>) {
        self.capture_analysis.set_storage(CaptureStorageSnapshot {
            name: identity,
            status: "Channel metadata ready".to_owned(),
            backing: CaptureStorageBacking::MetadataOnly,
            channels: channels.len(),
            total_samples: None,
            data_bytes: None,
            index_identity: None,
            index_progress: None,
        });
        self.logic_analyzer.set_channels(
            channels
                .into_iter()
                .map(|(index, name)| logic_analyzer_viewer::ChannelSignal {
                    index,
                    name,
                    initial: false,
                    transitions: Vec::new(),
                })
                .collect(),
        );
    }

    fn mark_capture_index_building(
        &mut self,
        identity: String,
        metadata: Option<signal_capture::CaptureMetadata>,
        progress: Option<signal_capture::CaptureIndexBuildProgress>,
    ) {
        let progress_fraction = progress.and_then(|progress| {
            (progress.total > 0).then(|| progress.completed as f32 / progress.total as f32)
        });
        self.capture_analysis.set_storage(CaptureStorageSnapshot {
            name: identity.clone(),
            status: "Preparing capture index".to_owned(),
            backing: CaptureStorageBacking::BuildingIndex,
            channels: metadata
                .as_ref()
                .map_or(0, |metadata| metadata.total_probes),
            total_samples: metadata.as_ref().map(|metadata| metadata.total_samples),
            data_bytes: metadata.as_ref().map(|metadata| {
                metadata
                    .total_samples
                    .div_ceil(8)
                    .saturating_mul(metadata.total_probes as u64)
            }),
            index_identity: None,
            index_progress: progress_fraction,
        });
        self.logic_analyzer
            .set_preparing_capture(identity, metadata, progress);
    }

    pub(crate) fn clear_capture_presentation(&mut self) {
        self.capture_analysis.set_presentation_identity(None);
        self.capture_analysis.clear_storage();
        self.logic_analyzer.clear_capture();
    }

    pub(crate) fn apply_capture_provider_poll(&mut self, poll: CaptureProviderPoll) -> bool {
        let poll_again = poll.poll_again;
        if let Some(readiness) = poll.readiness {
            readiness.publish();
        }
        match poll.presentation {
            CapturePresentationUpdate::Unchanged => {}
            CapturePresentationUpdate::Clear { restore_prepared } => {
                self.clear_capture_presentation();
                if restore_prepared {
                    self.platform_restore_graph_capture();
                }
            }
            CapturePresentationUpdate::Preparing {
                identity,
                visible_channels,
                metadata,
                progress,
            } => {
                if self.capture_analysis.presentation_identity() != Some(identity.as_str()) {
                    self.clear_capture_presentation();
                }
                self.logic_analyzer
                    .set_visible_capture_channels(visible_channels);
                self.capture_analysis
                    .set_presentation_identity(Some(identity.clone()));
                self.mark_capture_index_building(identity, metadata, progress);
            }
            CapturePresentationUpdate::Indexed {
                identity,
                visible_channels,
                index,
                growing,
                planned_span_us,
            } => {
                if let Some(visible_channels) = visible_channels {
                    self.logic_analyzer
                        .set_visible_capture_channels(visible_channels);
                }
                self.capture_analysis
                    .set_presentation_identity(Some(identity.clone()));
                self.set_indexed_capture(identity, index, growing, planned_span_us);
            }
            CapturePresentationUpdate::InMemory {
                identity,
                visible_channels,
                signals,
                duration_us,
            } => {
                self.logic_analyzer
                    .set_visible_capture_channels(visible_channels);
                self.capture_analysis
                    .set_presentation_identity(Some(identity.clone()));
                self.set_capture_preview(identity, signals, duration_us);
            }
            CapturePresentationUpdate::Channels {
                identity,
                visible_channels,
                channels,
            } => {
                self.logic_analyzer
                    .set_visible_capture_channels(visible_channels);
                self.capture_analysis
                    .set_presentation_identity(Some(identity.clone()));
                self.set_capture_channel_metadata(identity, channels);
            }
            CapturePresentationUpdate::Failed(error) => {
                self.clear_capture_presentation();
                self.toasts.error(error.to_string());
            }
        }
        poll_again
    }

    fn set_presented_derived_lanes(&mut self, lanes: signal_derived::DerivedLanes) {
        self.presentations
            .set_presented_derived_lanes(lanes.clone());
        self.logic_analyzer.set_derived_lanes(lanes);
    }

    fn toast_source_for_node(&self, node_id: NodeId) -> ToastSource {
        let title = self
            .node_graph
            .graph()
            .nodes
            .get(&node_id)
            .map(|node| node.title.clone())
            .unwrap_or_else(|| "Removed node".to_owned());
        ToastSource::node(title)
    }

    fn toast_source_for_socket(&self, node_id: NodeId, socket: impl Into<String>) -> ToastSource {
        let node = self
            .node_graph
            .graph()
            .nodes
            .get(&node_id)
            .map(|node| node.title.clone())
            .unwrap_or_else(|| "Removed node".to_owned());
        ToastSource::socket(node, socket)
    }

    /// Creates the application with unavailable default host services and an empty graph.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    pub fn new(cc: &eframe::CreationContext) -> Self {
        Self::build_with_app_services(
            cc,
            Vec::new(),
            crate::app_services::unavailable_app_services(),
        )
    }

    /// Builds the application around an initial graph supplied by the host
    /// application. The host owns where that graph comes from.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `graph`: Initial persisted graph document to restore.
    pub fn new_with_graph(
        cc: &eframe::CreationContext,
        graph: node_graph::api::GraphState,
    ) -> Self {
        let mut app = Self::new(cc);
        app.apply_graph_document(graph);
        app
    }

    /// Builds the application with host-supplied demo documents. The first
    /// entry is loaded at startup and remains available from the Demos menu.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `demo_graphs`: Available named demo documents, with the first selected initially.
    pub fn new_with_demo_graphs(cc: &eframe::CreationContext, demo_graphs: Vec<DemoGraph>) -> Self {
        let default_graph = demo_graphs.first().map(|demo| demo.graph.clone());
        let mut app = Self::new(cc);
        app.demo_graphs = demo_graphs;
        if let Some(graph) = default_graph {
            app.apply_graph_document(graph);
        }
        app
    }

    pub(crate) fn apply_graph_document(&mut self, graph: GraphState) {
        self.node_graph.set_graph(graph);
        self.synchronize_payload_subscription_manifest(true);
        self.restore_sampling_overlay_setting();
        self.restore_viewer_lane_order_setting();
        self.restore_viewer_lane_height_setting();
        self.restore_timeline_cursor_setting();
        self.restore_panel_layout_setting();
        self.graph_run.clear_cached_preview_revision();
    }

    /// Loads one configured demo after rejecting the request during active capture work.
    ///
    /// # Parameters
    /// - `index`: Position in the configured demo list. Invalid indices are ignored.
    pub fn load_demo_graph(&mut self, index: usize) {
        let Some((name, graph)) = self
            .demo_graphs
            .get(index)
            .map(|demo| (demo.name.clone(), demo.graph.clone()))
        else {
            return;
        };
        if self.capture_analysis.coordinator().is_active() || self.is_capture_analysis_active() {
            self.toasts
                .error("Stop the active capture before loading a demo");
            return;
        }
        self.clear_derived_data_presentations();
        self.capture_analysis.coordinator_mut().clear_completed();
        self.capture_analysis.clear_capture_graph();
        self.capture_analysis.clear_analysis();
        self.graph_run.clear_run_message();
        self.error_badges.clear();
        self.clear_capture_presentation();
        self.platform_restore_graph_capture();
        self.apply_graph_document(graph);
        let availability = capture_availability(
            self.node_graph.graph(),
            self.graph_run.service(),
            self.capture_analysis
                .coordinator()
                .backend_unavailable_reason(),
        );
        self.capture_analysis.set_availability(availability);
        self.refresh_trigger_configuration();
        self.toasts.info(format!("Loaded demo {name}"));
    }

    pub(crate) fn synchronize_payload_subscription_manifest(&mut self, report_warnings: bool) {
        match synchronize_viewer_compatibility(self.node_graph.graph_mut()) {
            Ok(warnings) if report_warnings => {
                for warning in warnings {
                    if let Some(node) = warning.node {
                        self.node_graph
                            .set_node_badge(node, Some(NodeBadge::warning(&warning.message)));
                        self.error_badges.push(node);
                        let source = self.toast_source_for_node(node);
                        self.toasts.warning_from(source, warning.message);
                    } else {
                        self.toasts.warning(warning.message);
                    }
                }
            }
            Ok(_) => {}
            Err(error) => self
                .toasts
                .error(format!("Could not update saved viewer selections: {error}")),
        }
        self.refresh_graph_output_selections();
    }

    fn refresh_graph_output_selections(&mut self) -> ViewerOutputPanelData {
        let selections = viewer_output_selections(self.node_graph.graph());
        let subscriptions = output_subscription_plan(self.node_graph.graph());
        self.graph_run
            .service_mut()
            .set_output_subscriptions(subscriptions);
        let mut by_node: HashMap<NodeId, Vec<ViewerOutputPanelEntry>> = HashMap::new();
        self.node_graph
            .clear_socket_indicators(VIEWER_SOCKET_INDICATOR_OWNER);
        for selection in selections {
            by_node
                .entry(selection.node)
                .or_default()
                .push(ViewerOutputPanelEntry {
                    id: selection.output_id.clone(),
                    label: selection.label,
                    selected: selection.selected,
                });
            if selection.selected {
                for output in selection.indicator_outputs {
                    self.node_graph.set_socket_indicator(
                        VIEWER_SOCKET_INDICATOR_OWNER,
                        SocketId {
                            node: selection.node,
                            index: output,
                            direction: SocketDirection::Output,
                        },
                        "active",
                        ViewerSocketIndicator,
                    );
                }
            }
        }
        ViewerOutputPanelData {
            models: by_node
                .into_iter()
                .map(|(node, outputs)| (node, ViewerOutputPanelModel { outputs }))
                .collect(),
        }
    }

    /// The persisted MRU list, most recent first — read once at startup by
    /// the native macOS menu to build its "Open Recent" submenu (Phase 5.1).
    /// Empty on wasm, where there is no recent-files list at all.
    pub fn recent_files(&self) -> &[std::path::PathBuf] {
        self.platform.recent_files()
    }

    /// Returns the validated binding configuration installed by the host services.
    pub fn input_bindings(&self) -> &InputBindings {
        &self.input_bindings
    }

    /// Creates the application and asks host services to load an optional startup file.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `file`: Optional startup path delegated to the configured host service.
    pub fn new_with_file(cc: &eframe::CreationContext, file: Option<&Path>) -> Self {
        Self::new_with_file_and_catalogs(cc, file, Vec::new())
    }

    /// Creates the application with additional host-discovered node catalogs and an optional file.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `file`: Optional startup path delegated to the configured host service.
    /// - `node_catalogs`: Host-owned node catalogs added to the built-in registry.
    pub fn new_with_file_and_catalogs(
        cc: &eframe::CreationContext,
        file: Option<&Path>,
        node_catalogs: Vec<Box<dyn NodeCatalogService>>,
    ) -> Self {
        let mut app = Self::build_with_app_services(
            cc,
            node_catalogs,
            crate::app_services::unavailable_app_services(),
        );
        app.platform_load_startup_file(file);
        app
    }

    /// Builds the application with services selected by the host composition
    /// root.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `file`: Optional startup path delegated to the configured host service.
    /// - `node_catalogs`: Host-owned node catalogs added to the built-in registry.
    /// - `services`: Complete injected host service set.
    pub fn new_with_file_catalogs_and_services(
        cc: &eframe::CreationContext,
        file: Option<&Path>,
        node_catalogs: Vec<Box<dyn NodeCatalogService>>,
        services: crate::AppServices,
    ) -> Self {
        let mut app = Self::build_with_app_services(cc, node_catalogs, services);
        app.platform_load_startup_file(file);
        app
    }

    /// Creates the application with demos and a complete injected host service set.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `demo_graphs`: Available named demo documents, with the first selected initially.
    /// - `services`: Complete injected host service set.
    pub fn new_with_demo_graphs_and_services(
        cc: &eframe::CreationContext,
        demo_graphs: Vec<DemoGraph>,
        services: crate::AppServices,
    ) -> Self {
        Self::new_with_demo_graphs_catalogs_and_services(cc, demo_graphs, Vec::new(), services)
    }

    /// Builds the application with embedded demos and host-selected node catalogs.
    ///
    /// # Parameters
    /// - `cc`: Eframe creation context used to initialize UI resources.
    /// - `demo_graphs`: Available named demo documents, with the first selected initially.
    /// - `node_catalogs`: Host-owned node catalogs added to the built-in registry.
    /// - `services`: Complete injected host service set.
    pub fn new_with_demo_graphs_catalogs_and_services(
        cc: &eframe::CreationContext,
        demo_graphs: Vec<DemoGraph>,
        node_catalogs: Vec<Box<dyn NodeCatalogService>>,
        services: crate::AppServices,
    ) -> Self {
        let default_graph = demo_graphs.first().map(|demo| demo.graph.clone());
        let mut app = Self::build_with_app_services(cc, node_catalogs, services);
        app.demo_graphs = demo_graphs;
        if let Some(graph) = default_graph {
            app.apply_graph_document(graph);
        }
        app
    }

    fn build_with_app_services(
        cc: &eframe::CreationContext,
        node_catalogs: Vec<Box<dyn NodeCatalogService>>,
        services: crate::AppServices,
    ) -> Self {
        Self::build_with_services(cc, node_catalogs, services.into_parts())
    }

    fn build_with_services(
        cc: &eframe::CreationContext,
        node_catalogs: Vec<Box<dyn NodeCatalogService>>,
        services: AppServiceParts,
    ) -> Self {
        let AppServiceParts {
            graph_service,
            host_service,
            input_bindings,
            application_settings,
            host_symbol_fonts,
            node_file_dialog,
            node_editor_overrides,
            work_executor,
            worker_operation_executor,
            capture_export_service,
            artifact_repository,
            decoded_block_cache,
        } = services;
        let mut host_service = host_service;
        let host_ui_capabilities = host_service.ui_capabilities();
        // The graph canvas and its custom widgets use a dark palette. Do not
        // inherit a light OS/browser preference for the surrounding egui
        // controls, or their dark foreground text becomes unreadable there.
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        install_fonts(&cc.egui_ctx, host_symbol_fonts);
        let repaint_context = cc.egui_ctx.clone();
        host_service.set_command_repaint(Box::new(move || repaint_context.request_repaint()));
        let registry =
            crate::node_registry::build_node_registry_with_editor_overrides(node_editor_overrides);
        let input_bindings = Arc::new(input_bindings);
        let plugin_panel_registry =
            PluginPanelRegistry::standard().expect("UI-panel inventory registration must be valid");
        let mut widget = NodeGraphWidget::new(registry);
        if let Some(mut file_dialog) = node_file_dialog {
            let repaint_context = cc.egui_ctx.clone();
            file_dialog.set_repaint(Box::new(move || repaint_context.request_repaint()));
            widget.set_file_dialog_service(file_dialog);
        }
        widget.set_input_bindings(input_bindings.clone());
        widget.set_panel_tabs(vec![PanelTabDef::new("view", "View")]);
        let platform = crate::app_platform::PlatformState::restore(cc, &mut widget);
        let mut logic_analyzer = LogicAnalyzerViewer::new();
        logic_analyzer.set_input_bindings(input_bindings.clone());
        logic_analyzer.set_color_profile(application_settings.viewer_color_profile());
        let capture = CaptureCoordinator::configured(
            application_settings.max_recent_capture_sessions(),
            application_settings
                .max_capture_storage_gib()
                .saturating_mul(1024 * 1024 * 1024),
            artifact_repository,
            work_executor,
            capture_export_service,
        );
        let capture_availability = capture_availability(
            widget.graph(),
            &graph_service,
            capture.backend_unavailable_reason(),
        );
        let presentation_graph_nodes = widget.graph().nodes.keys().copied().collect();
        Self {
            node_graph: widget,
            logic_analyzer,
            input_bindings,
            panel_layout: Self::default_panel_layout(),
            graph_run: GraphRunLifecycle::new(graph_service),
            decoded_block_cache,
            host_service,
            host_ui_capabilities,
            capture_analysis: CaptureAnalysisLifecycle::new(capture, capture_availability),
            toasts: Toasts::default(),
            platform,
            about: AboutWindow::new(),
            output_downloads: OutputDownloadsWindow::new(),
            preferences: PreferencesWindow::new(),
            node_catalogs,
            demo_graphs: Vec::new(),
            error_badges: Vec::new(),
            presentations: PresentationCatalogs::new(
                plugin_panel_registry,
                presentation_graph_nodes,
            ),
            memory_panel: MemoryPanel::default(),
            timeline_markers: TimelineMarkerBindings::default(),
            _worker_operation_executor: worker_operation_executor,
        }
    }

    fn default_panel_layout() -> PanelLayout {
        PanelLayout::new([
            ("logic_analyzer", DEFAULT_ANALYZER_SPLIT),
            ("node_graph", 1.0 - DEFAULT_ANALYZER_SPLIT),
        ])
    }

    pub(crate) fn restore_panel_layout_setting(&mut self) {
        match saved_panel_layout(self.node_graph.graph()) {
            Ok(Some(saved)) => {
                self.panel_layout = PanelLayout::from_state(saved.layout);
                self.presentations
                    .replace_decoder_panels(DecoderPanels::from_state(saved.decoder_panels));
                self.presentations
                    .plugin_panels_mut()
                    .restore_state(saved.plugin_panels);
            }
            Ok(None) => {
                self.panel_layout = Self::default_panel_layout();
                self.presentations
                    .replace_decoder_panels(DecoderPanels::default());
                self.presentations.plugin_panels_mut().reset_state();
            }
            Err(error) => {
                self.panel_layout = Self::default_panel_layout();
                self.presentations
                    .replace_decoder_panels(DecoderPanels::default());
                self.presentations.plugin_panels_mut().reset_state();
                self.toasts
                    .error(format!("Could not restore the saved panel layout: {error}"));
            }
        }
    }

    pub(crate) fn sync_panel_layout_setting(&mut self) -> Result<(), serde_json::Error> {
        save_panel_layout(
            self.node_graph.graph_mut(),
            self.panel_layout.state().clone(),
            self.presentations.decoder_panels().state().clone(),
            self.presentations.plugin_panels().state(),
        )
    }

    pub(crate) fn restore_viewer_lane_order_setting(&mut self) {
        match saved_viewer_lane_order(self.node_graph.graph()) {
            Ok(order) => self.presentations.replace_viewer_lane_order(order),
            Err(error) => {
                self.presentations.clear_viewer_lane_order();
                self.toasts
                    .error(format!("Could not restore the viewer lane order: {error}"));
            }
        }
        let order = self
            .presentations
            .viewer_lane_order()
            .iter()
            .map(ViewerRowId::from)
            .collect::<Vec<_>>();
        self.logic_analyzer.apply_viewer_row_order(&order);
    }

    fn sync_viewer_lane_order(&mut self) {
        if self.logic_analyzer.take_viewer_row_order_changed() {
            let current = self.logic_analyzer.viewer_row_order();
            let current_set = current
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let mut saved = current.iter().map(SavedViewerRow::from).collect::<Vec<_>>();
            saved.extend(
                self.presentations
                    .viewer_lane_order()
                    .iter()
                    .filter(|row| !current_set.contains(&ViewerRowId::from(*row)))
                    .cloned(),
            );
            self.presentations.replace_viewer_lane_order(saved);
            if let Err(error) = save_viewer_lane_order(
                self.node_graph.graph_mut(),
                self.presentations.viewer_lane_order(),
            ) {
                self.toasts
                    .error(format!("Could not save the viewer lane order: {error}"));
            }
            return;
        }

        let requested = self
            .presentations
            .viewer_lane_order()
            .iter()
            .map(ViewerRowId::from)
            .collect::<Vec<_>>();
        self.logic_analyzer.apply_viewer_row_order(&requested);
    }

    pub(crate) fn restore_viewer_lane_height_setting(&mut self) {
        match saved_viewer_lane_heights(self.node_graph.graph()) {
            Ok(settings) => self
                .logic_analyzer
                .apply_viewer_row_height_settings(&settings),
            Err(error) => {
                self.reset_viewer_lane_heights();
                self.toasts.error(format!(
                    "Could not restore the viewer lane heights: {error}"
                ));
            }
        }
    }

    fn sync_viewer_lane_heights(&mut self) {
        if self.logic_analyzer.take_viewer_row_height_changed() {
            let settings = self.logic_analyzer.viewer_row_height_settings();
            if let Err(error) = save_viewer_lane_heights(self.node_graph.graph_mut(), &settings) {
                self.toasts
                    .error(format!("Could not save the viewer lane heights: {error}"));
            }
        }
    }

    pub(crate) fn restore_timeline_cursor_setting(&mut self) {
        match saved_timeline_cursors(self.node_graph.graph()) {
            Ok(cursors) => self.logic_analyzer.set_time_cursors(cursors),
            Err(error) => {
                self.logic_analyzer.set_time_cursors(Vec::new());
                self.toasts
                    .error(format!("Could not restore timeline cursors: {error}"));
            }
        }
    }

    fn sync_timeline_cursor_setting(&mut self) -> bool {
        if !self.logic_analyzer.take_time_cursors_changed() {
            return false;
        }
        let cursors = self.logic_analyzer.time_cursors().to_vec();
        if let Err(error) = save_timeline_cursors(self.node_graph.graph_mut(), &cursors) {
            self.toasts
                .error(format!("Could not save timeline cursors: {error}"));
        }
        true
    }

    fn timeline_marker_reference_choices(&self) -> Vec<TimelineMarkerReferenceChoice> {
        self.logic_analyzer
            .time_cursors()
            .iter()
            .map(|cursor| {
                TimelineMarkerReferenceChoice::new(
                    TimelineMarkerReference::Cursor {
                        number: cursor.number,
                    },
                    format!("Cursor {}", cursor.number),
                    (cursor.time_us.max(0.0) * 1_000.0).round() as u64,
                )
            })
            .collect()
    }

    fn synchronize_timeline_marker_references(&mut self, viewer_changed: bool) {
        let discovered = match self
            .graph_run
            .service()
            .discover_timeline_marker_reference_bindings(self.node_graph.graph())
        {
            Ok(discovered) => {
                self.timeline_markers.clear_reference_error();
                discovered
            }
            Err(error) => {
                let message = error.to_string();
                if self
                    .timeline_markers
                    .record_reference_error(message.clone())
                {
                    self.toasts.error_from(
                        ToastSource::panel("Logic Analyzer"),
                        format!("Could not synchronize cursor marker controls: {message}"),
                    );
                }
                return;
            }
        };

        let original_choices = self.timeline_marker_reference_choices();
        if !viewer_changed {
            let mut moved = Vec::new();
            for discovered in &discovered {
                if discovered.binding.choices != original_choices {
                    continue;
                }
                let Some(selected) = discovered.binding.selected else {
                    continue;
                };
                let Some(choice) = original_choices
                    .iter()
                    .find(|choice| choice.reference == selected)
                else {
                    continue;
                };
                if choice.timestamp_ns == discovered.binding.timestamp_ns
                    || moved.contains(&selected)
                {
                    continue;
                }
                match selected {
                    TimelineMarkerReference::Cursor { number } => {
                        if self.logic_analyzer.set_time_cursor_time(
                            number,
                            discovered.binding.timestamp_ns as f64 / 1_000.0,
                        ) {
                            moved.push(selected);
                        }
                    }
                }
            }
        }

        let choices = self.timeline_marker_reference_choices();
        for discovered in discovered {
            if TimelineMarkerBindings::reference_binding_is_synchronized(
                &discovered.binding,
                &choices,
            ) {
                continue;
            }
            let edit = TimelineMarkerReferenceBindingEdit::Synchronize {
                id: discovered.binding.id,
                choices: choices.clone(),
            };
            match self
                .graph_run
                .service()
                .apply_timeline_marker_reference_binding_edit(
                    self.node_graph.graph(),
                    discovered.owner_node,
                    &edit,
                ) {
                Ok(state) => {
                    if !self.node_graph.set_node_state(discovered.owner_node, state) {
                        self.toasts.error_from(
                            self.toast_source_for_node(discovered.owner_node),
                            "Could not refresh the cursor marker controls",
                        );
                    }
                }
                Err(error) => self.toasts.error_from(
                    self.toast_source_for_node(discovered.owner_node),
                    error.to_string(),
                ),
            }
        }
    }

    fn supply_timeline_cursors(&self, context: &mut runtime::GraphRunContext) {
        for cursor in self.logic_analyzer.time_cursors() {
            context.set_timeline_marker(
                TimelineMarkerReference::Cursor {
                    number: cursor.number,
                },
                signal_derived::TimelineMarker::new(
                    (cursor.time_us.max(0.0) * 1_000.0).round() as u64
                ),
            );
        }
    }

    pub(crate) fn reset_viewer_lane_heights(&mut self) {
        self.logic_analyzer.reset_viewer_row_heights();
        self.sync_viewer_lane_heights();
    }

    fn refresh_sampling_overlay_ui(&mut self) {
        let overlays = self
            .presentations
            .selected_sampling_overlays()
            .iter()
            .filter_map(|selected| {
                self.graph_run
                    .sampling_overlay_candidates()
                    .iter()
                    .find(|candidate| {
                        candidate.node_id() == *selected
                            && self
                                .node_graph
                                .graph()
                                .nodes
                                .contains_key(&candidate.node_id())
                    })
                    .map(|candidate| sampling_overlay_presentation(candidate.overlay()))
            })
            .collect();
        self.logic_analyzer.set_sampling_overlays(overlays);

        let mut actions: HashMap<NodeId, Vec<NodeContextAction>> = HashMap::new();
        for candidate in self.graph_run.sampling_overlay_candidates() {
            let selected = self
                .presentations
                .selected_sampling_overlays()
                .iter()
                .any(|selected| *selected == candidate.node_id());
            let mut action = NodeContextAction::new("sampling_overlay", "Sampling Points")
                .with_checkmark(selected);
            if !selected {
                action = action.with_icon("◆");
            }
            actions.insert(candidate.node_id(), vec![action]);
        }
        self.node_graph.set_node_context_actions(actions);
    }

    pub(crate) fn restore_sampling_overlay_setting(&mut self) {
        match saved_sampling_overlays(self.node_graph.graph()) {
            Ok((selected, migrated)) => {
                self.presentations
                    .replace_selected_sampling_overlays(selected);
                if migrated {
                    self.toasts.warning(
                        "Migrated the saved sampling-points selection to support multiple decoders",
                    );
                    self.persist_sampling_overlay_setting();
                }
            }
            Err(error) => {
                self.presentations.clear_selected_sampling_overlays();
                self.toasts.error(format!(
                    "Could not restore the graph's sampling-points setting: {error}"
                ));
            }
        }
        self.graph_run.clear_sampling_overlay_candidates();
        self.refresh_graph_output_selections();
        self.refresh_sampling_overlay_ui();
    }

    fn persist_sampling_overlay_setting(&mut self) {
        let result = save_sampling_overlays(
            self.node_graph.graph_mut(),
            self.presentations.selected_sampling_overlays(),
        );
        if let Err(error) = result {
            self.toasts.error(format!(
                "Could not save the graph's sampling-points setting: {error}"
            ));
        }
    }

    fn set_sampling_overlay_candidates(&mut self, candidates: Vec<plan::SamplingOverlayCandidate>) {
        self.graph_run
            .replace_sampling_overlay_candidates(candidates);
        if self
            .presentations
            .retain_sampling_overlay_candidates(self.graph_run.sampling_overlay_candidates())
        {
            self.persist_sampling_overlay_setting();
        }
        self.refresh_sampling_overlay_ui();
    }

    fn handle_node_context_action(&mut self, node_id: NodeId, action_id: &str) {
        if action_id != "sampling_overlay" {
            return;
        }
        let current_run_candidates = self
            .graph_run
            .run()
            .map(|run| run.sampling_overlays().to_vec())
            .or_else(|| {
                self.capture_analysis
                    .analysis()
                    .map(|run| run.sampling_overlays().to_vec())
            });
        if let Some(candidates) = current_run_candidates {
            self.graph_run
                .replace_sampling_overlay_candidates(candidates);
        }
        if !self
            .graph_run
            .sampling_overlay_candidates()
            .iter()
            .any(|candidate| candidate.node_id() == node_id)
        {
            return;
        }
        self.presentations.toggle_sampling_overlay(node_id);
        self.persist_sampling_overlay_setting();
        self.refresh_sampling_overlay_ui();
    }

    fn report_compile_errors(&mut self, errors: &[plan::ProcessingGraphError]) {
        for error in errors {
            if let Some(id) = error.node {
                self.node_graph
                    .set_node_badge(id, Some(NodeBadge::error(&error.message)));
                self.error_badges.push(id);
                let source = self.toast_source_for_node(id);
                self.toasts.error_from(source, &error.message);
            } else {
                self.toasts.error(&error.message);
            }
        }
        let summary = errors
            .first()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "compile failed".to_owned());
        let extra = errors.len().saturating_sub(1);
        let message = if extra > 0 {
            format!("{summary} (+{extra} more)")
        } else {
            summary
        };
        self.graph_run.set_run_message(message, true);
    }

    fn show_logic_analyzer_status(&mut self, ui: &mut egui::Ui) {
        self.show_capture_controls(ui);
        self.show_growing_waveform_controls(ui);
        ui.separator();
        ui.label(egui::RichText::new(self.logic_analyzer.status_summary()).weak());
        if let Some(progress) = self.logic_analyzer.index_progress_fraction() {
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_width(64.0)
                    .show_percentage(),
            );
        }
    }

    pub(crate) fn clear_derived_data_presentations(&mut self) {
        if let Some(mut run) = self.graph_run.take_run() {
            run.stop();
        }
        let lanes = self.presentations.clear_run_catalogs();
        self.logic_analyzer.set_derived_lanes(lanes);
        self.logic_analyzer
            .set_waveform_presentations(WaveformPresentationRegistry::new());
    }

    fn bind_run_data(&mut self, run_data: runtime::RunData) {
        let lanes = run_data.derived_lanes().clone();
        self.set_presented_derived_lanes(lanes.clone());
        self.presentations.replace_run_catalogs(&run_data);
        self.bind_catalog_presentations();
        self.set_sampling_overlay_candidates(run_data.sampling_overlays().to_vec());
    }

    fn bind_catalog_presentations(&mut self) {
        let outputs = self
            .presentations
            .visible_output_subscriptions(self.node_graph.graph());
        let tables = self
            .presentations
            .visible_table_subscriptions(self.node_graph.graph());
        match waveform_presentation_registry(&outputs) {
            Ok(presentations) => self
                .logic_analyzer
                .set_waveform_presentations(presentations),
            Err(error) => self.toasts.error(format!(
                "Could not bind collected output presentation: {error}"
            )),
        }
        match decoder_table_registry(&tables) {
            Ok(tables) => {
                let lanes = self.presentations.presented_derived_lanes().clone();
                self.presentations
                    .decoder_panels_mut()
                    .set_run_data(lanes, tables);
            }
            Err(error) => self.toasts.error(format!(
                "Could not bind decoder-table presentation: {error}"
            )),
        }
    }

    fn merge_current_run_presentation_catalog(&mut self) {
        let Some(run) = self.graph_run.run() else {
            return;
        };
        let outputs = run.output_subscriptions().to_vec();
        let tables = run.table_subscriptions().to_vec();
        self.presentations.merge_run_catalogs(&outputs, &tables);
        self.bind_catalog_presentations();
    }

    fn synchronize_presentation_graph_nodes(&mut self) -> bool {
        if !self
            .presentations
            .synchronize_graph_nodes(self.node_graph.graph())
        {
            return false;
        }
        self.bind_catalog_presentations();
        self.refresh_sampling_overlay_ui();
        true
    }

    fn restore_prepared_cached_derived_data(
        &mut self,
        revision: u64,
        compiled: plan::ProcessingGraph,
    ) {
        if self.graph_run.has_run()
            || self.capture_analysis.coordinator().is_active()
            || self.is_capture_analysis_active()
        {
            return;
        }
        self.graph_run.set_cached_preview_revision(revision);
        let mut ctx = runtime::GraphRunContext::default();
        self.supply_timeline_cursors(&mut ctx);
        match self
            .graph_run
            .service()
            .load_prepared_cached_data(compiled, &mut ctx)
        {
            Ok(true) => self.bind_run_data(ctx.run_data()),
            Ok(false) | Err(_) => self.clear_derived_data_presentations(),
        }
    }

    fn start_run(&mut self) {
        for id in self.error_badges.drain(..) {
            self.node_graph.set_node_badge(id, None);
        }
        self.node_graph.clear_node_statuses();
        self.graph_run.clear_run_message();

        let replay = match self.capture_analysis.coordinator().replay_source_node() {
            Some(source_node) if self.node_graph.graph().nodes.contains_key(&source_node) => {
                match self
                    .capture_analysis
                    .coordinator()
                    .create_replay_attachment()
                {
                    Ok(Some(replay)) => Some(replay),
                    Ok(None) => None,
                    Err(error) => {
                        let message = error.to_string();
                        self.graph_run.set_run_message(message.clone(), true);
                        self.toasts.error(message);
                        return;
                    }
                }
            }
            _ => None,
        };

        if replay.is_none()
            && matches!(
                capture_availability(
                    self.node_graph.graph(),
                    self.graph_run.service(),
                    self.capture_analysis
                        .coordinator()
                        .backend_unavailable_reason(),
                ),
                CaptureAvailability::Available { .. }
            )
        {
            let message = "Capture data before running a live-source graph".to_owned();
            self.graph_run.set_run_message(message.clone(), true);
            self.toasts.error(message);
            return;
        }

        // Reassert the complete host-owned plan at the execution boundary.
        // Cached-preview and panel discovery may both refresh the compiler
        // while no run exists; the persisted overlay must still be collected.
        self.refresh_graph_output_selections();

        // Run is an explicit fresh execution. Cached lanes are a pre-run
        // preview only, so release their mmap/query handles before removing
        // this graph's entries and creating the replacement stores.
        self.graph_run.clear_cached_preview_revision();
        self.clear_derived_data_presentations();
        let mut ctx = runtime::GraphRunContext::default();
        self.supply_timeline_cursors(&mut ctx);
        if replay.is_none()
            && let Err(error) = self.prepare_fresh_run_caches()
        {
            self.graph_run.set_run_message(error.clone(), true);
            self.toasts.error(error);
            return;
        }
        self.set_presented_derived_lanes(ctx.derived_lanes().clone());
        self.logic_analyzer
            .set_waveform_presentations(WaveformPresentationRegistry::new());
        self.presentations
            .decoder_panels_mut()
            .set_run_data(ctx.derived_lanes().clone(), DecoderTableRegistry::new());
        self.presentations
            .plugin_panels_mut()
            .set_run_data(ctx.derived_lanes().clone());

        let mut source_overrides = runtime::SourceProcessOverrides::new();
        if let Some(CaptureReplayAttachment {
            source_node,
            process,
        }) = replay
        {
            source_overrides.insert(source_node, process);
        }
        let started =
            self.graph_run
                .service()
                .start_run(self.node_graph.graph(), &mut ctx, source_overrides);
        match started {
            Ok(run) => {
                let run_data = ctx.run_data();
                self.bind_run_data(run_data);
                self.graph_run
                    .install_run(run, self.node_graph.graph().semantic_revision());
            }
            Err(errors) => {
                self.graph_run.clear_sampling_overlay_candidates();
                self.refresh_sampling_overlay_ui();
                self.report_compile_errors(&errors);
            }
        }
    }

    fn prepare_fresh_run_caches(&self) -> Result<(), String> {
        let Ok(inventory) = self
            .graph_run
            .service()
            .derived_cache_configs_by_node(self.node_graph.graph())
        else {
            // The ordinary start path reports compile errors with node
            // ownership and badges. Cache invalidation must not replace that
            // diagnostic boundary with a generic error.
            return Ok(());
        };
        let mut unique = std::collections::HashMap::new();
        for config in inventory.into_values().flatten() {
            unique.entry(config.cache_key).or_insert(config);
        }
        for config in unique.into_values() {
            self.graph_run
                .service()
                .clear_derived_cache_entry(&config)
                .map_err(|error| {
                    format!("Could not clear derived data cache before running: {error}")
                })?;
        }
        Ok(())
    }

    pub(crate) fn is_running(&self) -> bool {
        self.graph_run.is_running()
    }

    fn refresh_view_configuration_after_edit(&mut self) {
        if let Ok(Some(feature)) = self
            .graph_run
            .service()
            .discover_live_capture_feature(self.node_graph.graph())
        {
            self.logic_analyzer
                .set_visible_capture_channels(feature.visible_channels().iter().copied());
        }
        if !self.graph_run.has_run() {
            self.graph_run.clear_cached_preview_revision();
        }
    }

    fn is_stopping(&self) -> bool {
        self.graph_run.is_stopping()
    }

    /// Run/Stop menu items and their `Cmd+R`/`Cmd+.` accelerators (Phase
    /// 5.3) — guarded the same way the toolbar's own Run/Stop buttons
    /// already are (only one is ever shown at a time), so triggering either
    /// while it doesn't apply (Run while already running, Stop while not)
    /// is a safe no-op rather than double-starting or double-stopping.
    pub(crate) fn run_command(&mut self) {
        if !self.is_running()
            && !self.capture_analysis.coordinator().is_active()
            && !self.is_capture_analysis_active()
            && self.graph_run.cache_clear_task().is_none()
        {
            self.start_run();
        }
    }

    pub(crate) fn run_unavailable_reason(&self) -> Option<String> {
        if self.is_running() {
            return Some("The pipeline is already running".into());
        }
        if self.capture_analysis.coordinator().is_active() || self.is_capture_analysis_active() {
            return Some("Wait for live capture analysis to finish".into());
        }
        if self.graph_run.cache_clear_task().is_some() {
            return Some("Wait for derived data caches to be cleared".into());
        }
        match self.capture_analysis.availability() {
            CaptureAvailability::Available {
                source_node,
                source_title,
                ..
            } if self.capture_analysis.coordinator().replay_source_node() != Some(*source_node) => {
                Some(format!(
                    "Capture data from {source_title} before running the pipeline"
                ))
            }
            CaptureAvailability::Available { .. } | CaptureAvailability::Unavailable { .. } => None,
        }
    }

    pub(crate) fn stop_command(&mut self) {
        if self.is_running() && !self.is_stopping() {
            self.graph_run.stop_run();
        }
    }

    fn start_capture_command(&mut self, mode: signal_capture_session::CaptureStartMode) {
        if self.capture_analysis.coordinator().is_active()
            || self.is_running()
            || self.is_capture_analysis_active()
            || self
                .capture_analysis
                .coordinator()
                .export_status()
                .is_some()
        {
            return;
        }
        for id in self.error_badges.drain(..) {
            self.node_graph.set_node_badge(id, None);
        }
        self.node_graph.clear_node_statuses();
        self.graph_run.clear_run_message();
        self.node_graph.sync_node_states();
        let feature = match self
            .graph_run
            .service()
            .discover_live_capture_feature(self.node_graph.graph())
        {
            Ok(Some(feature)) => feature,
            Ok(None) => {
                self.toasts.error("The graph has no live capture source");
                return;
            }
            Err(error) => {
                self.toasts.error(error.to_string());
                return;
            }
        };
        self.logic_analyzer
            .set_visible_capture_channels(feature.visible_channels().iter().copied());
        let capture_cache_configs = self
            .capture_analysis
            .analysis()
            .map(|run| run.persistent_cache_configs())
            .unwrap_or_default();
        self.capture_analysis.begin_capture(
            self.node_graph.graph().clone(),
            self.node_graph.graph().semantic_revision(),
        );
        self.set_presented_derived_lanes(signal_derived::DerivedLanes::new());
        for config in &capture_cache_configs {
            if let Err(error) = self.graph_run.service().clear_derived_cache_entry(config) {
                self.capture_analysis.clear_capture_graph();
                self.toasts
                    .error(format!("Could not remove previous capture cache: {error}"));
                return;
            }
        }
        // Capture data is replaceable working state. Drop the viewer's index
        // handle before the coordinator removes the previous store and index.
        self.clear_capture_presentation();
        match self.capture_analysis.coordinator_mut().start_with_graph(
            feature,
            self.node_graph.graph(),
            mode,
        ) {
            Ok(()) => self.node_graph.set_editing_enabled(false),
            Err(error) => {
                self.capture_analysis.clear_capture_graph();
                self.toasts.error(error.to_string());
            }
        }
    }

    fn stop_capture_command(&mut self) {
        self.capture_analysis.coordinator_mut().request_stop();
    }

    fn abort_capture_command(&mut self) {
        if let Err(error) = self.capture_analysis.coordinator_mut().request_abort() {
            self.toasts.error(error.to_string());
        }
    }

    fn force_trigger_capture_command(&mut self) {
        if let Err(error) = self
            .capture_analysis
            .coordinator_mut()
            .request_force_trigger()
        {
            self.toasts.error(error.to_string());
        }
    }

    fn poll_capture(&mut self, ctx: &egui::Context) {
        let mut acquisition_provider =
            LiveCaptureProvider::new(self.capture_analysis.coordinator_mut(), None);
        if let Some(acquisition) = acquisition_provider.acquisition() {
            acquisition.poll();
        }
        if let Some(attachment) = self
            .capture_analysis
            .coordinator_mut()
            .take_analysis_attachment()
        {
            self.start_capture_analysis(attachment);
        }
        let readiness = self
            .capture_analysis
            .analysis()
            .map(|run| run.source_readiness().clone());
        let mut provider =
            LiveCaptureProvider::new(self.capture_analysis.coordinator_mut(), readiness);
        let poll = provider.poll();
        let _ = self.apply_capture_provider_poll(poll);
        self.sync_capture_analysis(ctx);
        let graph_processed = self.capture_analysis_progress();
        self.capture_analysis
            .coordinator_mut()
            .set_graph_processed_samples(graph_processed);
        let analysis_active = self.is_capture_analysis_active();
        self.node_graph
            .set_editing_enabled(self.capture_analysis.coordinator().graph_editing_enabled());
        self.logic_analyzer.set_simple_trigger_editing_enabled(
            !self.capture_analysis.coordinator().is_active() && !analysis_active,
        );
        if self.capture_analysis.coordinator().is_active()
            || analysis_active
            || self
                .capture_analysis
                .coordinator()
                .export_status()
                .is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if self.capture_analysis.analysis().is_none() {
            self.capture_analysis.clear_capture_graph();
        }
    }

    fn start_capture_analysis(&mut self, attachment: CaptureAnalysisAttachment) {
        let Some(graph) = self.capture_analysis.take_capture_graph() else {
            self.capture_analysis
                .fail_analysis("capture graph snapshot is unavailable");
            return;
        };
        self.refresh_graph_output_selections();
        let contains_source = match self
            .graph_run
            .service()
            .graph_contains_node(&graph, attachment.source_node)
        {
            Ok(contains_source) => contains_source,
            Err(_) => return,
        };
        if !contains_source {
            return;
        }
        let _ = self.presentations.clear_run_catalogs();
        let mut ctx = runtime::GraphRunContext::default();
        self.supply_timeline_cursors(&mut ctx);
        self.set_presented_derived_lanes(ctx.derived_lanes().clone());
        self.logic_analyzer
            .set_waveform_presentations(WaveformPresentationRegistry::new());
        self.presentations
            .decoder_panels_mut()
            .set_run_data(ctx.derived_lanes().clone(), DecoderTableRegistry::new());
        self.presentations
            .plugin_panels_mut()
            .set_run_data(ctx.derived_lanes().clone());
        let source = runtime::LiveAnalysisSource {
            source_node: attachment.source_node,
            process: attachment.process,
        };
        match self
            .graph_run
            .service()
            .start_live_analysis(&graph, &mut ctx, source)
        {
            Ok(run) => {
                let readiness = run.source_readiness().clone();
                self.bind_run_data(ctx.run_data());
                self.capture_analysis.install_analysis(run);
                LiveCaptureProvider::analysis_ready(readiness).publish();
            }
            Err(errors) => {
                self.report_compile_errors(&errors);
                let message = errors
                    .first()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| "live analysis could not start".into());
                self.capture_analysis.fail_analysis(message);
            }
        }
    }

    fn sync_capture_analysis(&mut self, ctx: &egui::Context) {
        if self.capture_analysis.analysis().is_none() {
            return;
        }
        self.capture_analysis
            .analysis_mut()
            .unwrap()
            .pump_for(256, std::time::Duration::from_millis(8));
        for (id, items) in self.capture_analysis.analysis().unwrap().progress() {
            let status = (items > 0).then(|| format_count(items));
            self.node_graph.set_node_status(id, status);
        }
        if !self.capture_analysis.analysis().unwrap().is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        for (node, event) in self
            .capture_analysis
            .analysis()
            .unwrap()
            .take_disconnected()
        {
            if let Some(id) = node {
                self.node_graph.set_node_badge(
                    id,
                    Some(NodeBadge::warning(format!(
                        "Disconnected during live analysis: can't keep up with {}.{}",
                        event.producer, event.port
                    ))),
                );
                self.error_badges.push(id);
            }
        }

        if let Some(preparation) = self
            .capture_analysis
            .coordinator_mut()
            .take_configuration_epoch_preparation()
        {
            self.capture_analysis.mark_epoch_request_finished();
            match preparation {
                Ok(prepared) => {
                    let result = self.graph_run.service().apply_configuration_epoch(
                        self.capture_analysis.analysis_mut().unwrap(),
                        &prepared.graph,
                        prepared.boundary,
                    );
                    let resolution = match result {
                        Ok(summary) => {
                            self.toasts.info(format!(
                                "configuration epoch {} applied at sample {} ({} node{})",
                                prepared.epoch_id,
                                prepared.source_sample,
                                summary.configured,
                                if summary.configured == 1 { "" } else { "s" }
                            ));
                            ConfigurationEpochResolution::Applied
                        }
                        Err(runtime::ApplyError::NeedsFullRestart(reason)) => {
                            self.toasts
                                .info(format!("live edit deferred to the next capture: {reason}"));
                            ConfigurationEpochResolution::Deferred(reason)
                        }
                        Err(runtime::ApplyError::Compile(errors)) => {
                            let message = errors
                                .first()
                                .map(|error| error.message.clone())
                                .unwrap_or_else(|| "the edited graph is invalid".into());
                            self.toasts
                                .error(format!("configuration epoch failed: {message}"));
                            ConfigurationEpochResolution::Failed(message)
                        }
                        Err(runtime::ApplyError::Apply(message)) => {
                            self.toasts
                                .error(format!("configuration epoch failed: {message}"));
                            ConfigurationEpochResolution::Failed(message)
                        }
                        Err(runtime::ApplyError::Materialization { source, .. }) => {
                            let message = source.to_string();
                            self.toasts
                                .error(format!("configuration epoch failed: {message}"));
                            ConfigurationEpochResolution::Failed(message)
                        }
                        Err(runtime::ApplyError::Runtime(error)) => {
                            let message = error.to_string();
                            self.toasts
                                .error(format!("configuration epoch failed: {message}"));
                            ConfigurationEpochResolution::Failed(message)
                        }
                    };
                    if let Err(error) = self
                        .capture_analysis
                        .coordinator_mut()
                        .resolve_configuration_epoch(prepared.epoch_id, resolution)
                    {
                        self.toasts.error(error.to_string());
                    }
                }
                Err(error) => self.toasts.error(error.to_string()),
            }
        }
        if let Some(Err(error)) = self
            .capture_analysis
            .coordinator_mut()
            .take_configuration_epoch_notice()
        {
            self.toasts.error(format!(
                "could not persist configuration epoch outcome: {error}"
            ));
        }

        const EDIT_QUIET_PERIOD_S: f64 = 0.25;
        let now = ctx.input(|input| input.time);
        let recording = self
            .capture_analysis
            .coordinator()
            .status()
            .is_some_and(|status| {
                status.state == signal_capture_session::CaptureSessionState::Recording
            });
        if !recording
            || self.capture_analysis.analysis().unwrap().is_finished()
            || self.capture_analysis.epoch_request_in_flight()
        {
            return;
        }
        let revision = self.node_graph.graph().semantic_revision();
        if self.capture_analysis.epoch_observed_revision() == Some(revision)
            || !self
                .graph_run
                .revision_is_quiet(revision, now, EDIT_QUIET_PERIOD_S)
        {
            return;
        }
        match self
            .capture_analysis
            .coordinator_mut()
            .request_configuration_epoch(self.node_graph.graph().clone())
        {
            Ok(()) => {
                self.capture_analysis.observe_epoch_revision(revision);
                self.capture_analysis.mark_epoch_request_started();
            }
            Err(error) => self.toasts.error(error.to_string()),
        }
    }

    pub(crate) fn is_capture_analysis_active(&self) -> bool {
        self.capture_analysis.is_analysis_active()
    }

    fn capture_analysis_progress(&self) -> Option<u64> {
        let source = self.capture_analysis.coordinator().status()?.source_node;
        self.capture_analysis
            .analysis()?
            .progress()
            .into_iter()
            .find_map(|(node, items)| (node == source).then_some(items))
    }

    fn show_capture_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(notice) = self.capture_analysis.coordinator_mut().take_export_notice() {
            match notice {
                Ok(completion) => {
                    self.toasts.info(format!(
                        "Exported raw capture to {}",
                        completion.destination.display()
                    ));
                    for warning in completion.warnings {
                        self.toasts.warning(format!("Export warning: {warning}"));
                    }
                }
                Err(crate::CaptureExportServiceError::Cancelled) => {
                    self.toasts.info("Capture export cancelled");
                }
                Err(error) => self.toasts.error(format!("Capture export failed: {error}")),
            }
        }
        let status = self.capture_analysis.coordinator().status().cloned();
        if self.capture_analysis.coordinator().is_active() {
            let state = status.as_ref().map(|status| status.state);
            let popup_open = ui.ctx().any_popup_open();
            if !popup_open
                && status.as_ref().is_some_and(|status| status.commands.abort)
                && self.input_bindings.consume_shortcut_once(
                    ui,
                    &["logic_analyzer.capture"],
                    "abort",
                )
            {
                self.abort_capture_command();
            } else if !popup_open
                && status.as_ref().is_some_and(|status| {
                    status.commands.force_trigger
                        && status.state == signal_capture_session::CaptureSessionState::Armed
                })
                && self.input_bindings.consume_shortcut_once(
                    ui,
                    &["logic_analyzer.capture"],
                    "force_trigger",
                )
            {
                self.force_trigger_capture_command();
            }
            if matches!(
                state,
                Some(
                    signal_capture_session::CaptureSessionState::Stopping
                        | signal_capture_session::CaptureSessionState::Error
                )
            ) {
                ui.add_enabled(false, egui::Button::new("⏹ Stop"));
                ui.spinner();
            } else {
                let stop_supported = status
                    .as_ref()
                    .is_none_or(|status| status.commands.orderly_stop);
                let stop = ui.add_enabled(stop_supported, egui::Button::new("⏹ Stop"));
                if !stop_supported {
                    stop.clone().on_disabled_hover_text(
                        "This capture source cannot stop before finite completion",
                    );
                }
                if stop.clicked() {
                    self.stop_capture_command();
                }
            }
            if let Some(status) = &status
                && (status.commands.abort || status.commands.force_trigger)
            {
                ui.menu_button("▾", |ui| {
                    if status.commands.force_trigger {
                        let shortcut = self
                            .input_bindings
                            .shortcut(&["logic_analyzer.capture"], "force_trigger");
                        let response = ui.add_enabled(
                            status.state == signal_capture_session::CaptureSessionState::Armed,
                            egui::Button::new("Force Trigger").shortcut_text(
                                shortcut
                                    .map(|shortcut| ui.ctx().format_shortcut(&shortcut))
                                    .unwrap_or_default(),
                            ),
                        );
                        if response.clicked() {
                            self.force_trigger_capture_command();
                            ui.close();
                        }
                    }
                    let abort_shortcut = self
                        .input_bindings
                        .shortcut(&["logic_analyzer.capture"], "abort");
                    if status.commands.abort
                        && ui
                            .add(
                                egui::Button::new("Abort").shortcut_text(
                                    abort_shortcut
                                        .map(|shortcut| ui.ctx().format_shortcut(&shortcut))
                                        .unwrap_or_default(),
                                ),
                            )
                            .clicked()
                    {
                        self.abort_capture_command();
                        ui.close();
                    }
                });
            }
        } else {
            let availability = if self
                .capture_analysis
                .coordinator()
                .export_status()
                .is_some()
            {
                CaptureAvailability::Unavailable {
                    reason: "Wait for Save Capture Data to finish".into(),
                }
            } else if self.is_running() {
                CaptureAvailability::Unavailable {
                    reason: "Stop the pipeline before starting capture".into(),
                }
            } else {
                self.capture_analysis.availability().clone()
            };
            let enabled = matches!(availability, CaptureAvailability::Available { .. });
            let response = ui.add_enabled(enabled, egui::Button::new("● Start"));
            match &availability {
                CaptureAvailability::Available {
                    source_node,
                    source_title,
                    ..
                } => {
                    response.clone().on_hover_text(format!(
                        "Start capture from {source_title} (node {})",
                        source_node.0
                    ));
                }
                CaptureAvailability::Unavailable { .. } => {
                    if let Some(reason) = availability.reason() {
                        response.clone().on_disabled_hover_text(reason);
                    }
                }
            }
            if response.clicked() {
                self.start_capture_command(signal_capture_session::CaptureStartMode::SavedPolicy);
                ui.ctx().request_repaint();
            }
            if let CaptureAvailability::Available {
                has_trigger_program,
                capabilities,
                ..
            } = &availability
                && capabilities.commands().capture_now
                && *has_trigger_program
            {
                ui.menu_button("▾", |ui| {
                    if ui.button("Capture Now").clicked() {
                        self.start_capture_command(
                            signal_capture_session::CaptureStartMode::CaptureNow,
                        );
                        ui.close();
                    }
                });
            }
        }

        if let Some(export) = self.capture_analysis.coordinator().export_status().cloned() {
            ui.separator();
            let fraction = if export.total_samples == 0 {
                0.0
            } else {
                export.samples_written as f32 / export.total_samples as f32
            };
            ui.add(
                egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .desired_width(120.0)
                    .text(format!(
                        "Exporting {} · {}/{} samples",
                        export.format_label, export.samples_written, export.total_samples
                    )),
            )
            .on_hover_text(export.destination.display().to_string());
            if ui
                .add_enabled(!export.cancelling, egui::Button::new("Cancel Export"))
                .clicked()
            {
                self.capture_analysis
                    .coordinator_mut()
                    .request_cancel_export();
            }
        }

        if let Some(status) = self.capture_analysis.coordinator().status() {
            let mut summary = capture_state_name(status.state).to_owned();
            if status.outcome.is_terminal() {
                summary = capture_outcome_name(status.outcome).into();
                if status.outcome.is_incomplete() {
                    summary.push_str(" · incomplete");
                }
            }
            if let Some(samples) = status.progress.captured_samples {
                summary.push_str(&format!(" · {samples} samples"));
            }
            if let Some(error) = &status.error {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 120, 120),
                    format!("Error · {error}"),
                );
            } else {
                ui.label(summary);
            }

            if status.health != signal_capture_session::CaptureHealth::default() {
                ui.menu_button("Health", |ui| {
                    if let Some(rate) = status.health.input_bytes_per_second {
                        ui.label(format!("Input: {}/s", format_bytes(rate)));
                    }
                    if let Some(rate) = status.health.write_bytes_per_second {
                        ui.label(format!("Store: {}/s", format_bytes(rate)));
                    }
                    if let Some(samples) = status.health.stored_samples {
                        ui.label(format!("Stored: {samples} samples"));
                    }
                    if let Some(samples) = status.health.summary_lag_samples {
                        ui.label(format!("Summary lag: {samples} samples"));
                    }
                    if let Some(samples) = status.health.graph_lag_samples {
                        ui.label(format!("Graph lag: {samples} samples"));
                    }
                });
            }

            if let Some(error) = self.capture_analysis.analysis_error() {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 120, 120),
                    format!("Analysis error · {error}"),
                );
            } else if let Some(processed) = self.capture_analysis_progress() {
                let captured = status
                    .progress
                    .captured_samples
                    .map(|captured| {
                        status
                            .recording_origin
                            .map(|origin| captured.saturating_sub(origin))
                            .unwrap_or(0)
                    })
                    .unwrap_or(processed);
                let lag = captured.saturating_sub(processed);
                if self
                    .capture_analysis
                    .analysis()
                    .is_some_and(|run| run.is_finished())
                {
                    ui.label(format!("Analysis complete · {processed} samples"));
                } else {
                    ui.label(format!("Analysis · {processed} samples · lag {lag}"));
                }
            } else if self.capture_analysis.coordinator().is_active() {
                ui.label("Analysis · waiting for committed data");
            }
        }
    }

    fn show_growing_waveform_controls(&mut self, ui: &mut egui::Ui) {
        if !self.logic_analyzer.has_growing_capture()
            || self.logic_analyzer.growing_capture_complete()
        {
            return;
        }

        ui.separator();
        let mut follow = self.logic_analyzer.follows_newest();
        if ui.checkbox(&mut follow, "Follow Newest").changed() {
            self.logic_analyzer.set_follow_newest(follow);
        }
        let paused = self.logic_analyzer.display_paused();
        if ui
            .small_button(if paused {
                "Resume Display"
            } else {
                "Pause Display"
            })
            .clicked()
        {
            self.logic_analyzer.toggle_pause_display();
        }
        if ui
            .add_enabled(paused || !follow, egui::Button::new("Go Live").small())
            .clicked()
        {
            self.logic_analyzer.go_live();
        }
    }

    /// Drives the run forward and submits a debounced immutable document revision for live
    /// reconciliation (`docs/architecture/processing_workflows.md`, live editing): taps, branch
    /// removals, hot property changes, and in-place restarts. Edits that need a full restart leave
    /// the run untouched and say so. Stale lowering completions are discarded by revision.
    ///
    /// `pump()` is called every frame — a no-op on the native threaded
    /// manager (its nodes run themselves in the background), but on wasm's
    /// cooperative manager it's what actually executes node `work()`, so it
    /// can't be gated behind the same throttle as the `apply()` diff below.
    fn sync_run(&mut self, ctx: &egui::Context) {
        const EDIT_QUIET_PERIOD_S: f64 = 0.25;
        const PROGRESS_UPDATE_INTERVAL_S: f64 = 0.5;

        if self.graph_run.cache_clear_task().is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }

        let now = ctx.input(|input| input.time);
        let revision = self.node_graph.graph().semantic_revision();
        self.graph_run.observe_document_revision(revision, now);
        if self.graph_run.revision_preparation_pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let prepared = self
            .graph_run
            .poll_revision_preparation()
            .filter(|prepared| prepared.revision == revision);

        if !self.graph_run.has_run() {
            if self.capture_analysis.coordinator().is_active() || self.is_capture_analysis_active()
            {
                return;
            }
            if let Some(prepared) = prepared {
                self.apply_idle_graph_revision(prepared);
            }
            if self.graph_run.cached_preview_revision() != Some(revision)
                && self
                    .graph_run
                    .should_prepare_revision(revision, now, EDIT_QUIET_PERIOD_S)
            {
                if let Err(error) = self
                    .graph_run
                    .start_revision_preparation(revision, self.node_graph.graph().clone())
                {
                    if error == platform_runtime::WorkExecutorError::QueueFull {
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    } else {
                        self.toasts
                            .error(format!("Could not prepare the edited graph: {error}"));
                    }
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                }
            }
            return;
        }

        let Some(GraphRunPoll {
            failure,
            synchronized,
            sampling_overlay_candidates,
            finished,
        }) = self.graph_run.poll_run(self.node_graph.graph())
        else {
            return;
        };
        if let Some(failure) = failure {
            let message = failure.to_string();
            self.graph_run.set_run_message(message.clone(), true);
            self.toasts.error(message);
        }
        match synchronized {
            Ok(true) => {
                self.set_sampling_overlay_candidates(
                    sampling_overlay_candidates.unwrap_or_default(),
                );
                self.merge_current_run_presentation_catalog();
            }
            Ok(false) => {}
            Err(errors) => self.report_compile_errors(&errors),
        }
        if !finished {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        if self
            .graph_run
            .progress_update_due(now, PROGRESS_UPDATE_INTERVAL_S)
        {
            self.graph_run.mark_progress_updated(now);
            for (id, items) in self
                .graph_run
                .run()
                .expect("run existence checked above")
                .progress()
            {
                let status = (items > 0).then(|| format_count(items));
                self.node_graph.set_node_status(id, status);
            }
        }

        if self.graph_run.run_is_finished_or_stopping() {
            return;
        }
        if let Some(prepared) = prepared {
            self.apply_live_graph_revision(prepared);
        }
        if self.graph_run.running_graph_revision() == Some(revision) {
            return;
        }
        if self
            .graph_run
            .should_prepare_revision(revision, now, EDIT_QUIET_PERIOD_S)
        {
            if let Err(error) = self
                .graph_run
                .start_revision_preparation(revision, self.node_graph.graph().clone())
            {
                if error == platform_runtime::WorkExecutorError::QueueFull {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                } else {
                    self.toasts
                        .error(format!("Could not prepare the edited graph: {error}"));
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
    }

    fn apply_idle_graph_revision(&mut self, prepared: PreparedGraphRevision) {
        let revision = prepared.revision;
        let candidates = prepared
            .processing_graph
            .as_ref()
            .map(|graph| graph.sampling_overlays.clone())
            .unwrap_or_default();
        match prepared.processing_graph {
            Ok(compiled) => self.restore_prepared_cached_derived_data(revision, compiled),
            Err(_) => {
                self.graph_run.set_cached_preview_revision(revision);
                self.clear_derived_data_presentations();
            }
        }

        let availability = capture_availability(
            self.node_graph.graph(),
            self.graph_run.service(),
            self.capture_analysis
                .coordinator()
                .backend_unavailable_reason(),
        );
        self.capture_analysis.set_availability(availability);
        self.refresh_trigger_configuration();
        self.set_sampling_overlay_candidates(candidates);
    }

    fn apply_live_graph_revision(&mut self, prepared: PreparedGraphRevision) {
        let revision = prepared.revision;
        let compiled = match prepared.processing_graph {
            Ok(compiled) => compiled,
            Err(_) => {
                // Mid-edit graphs are often momentarily invalid. Keep the active pipeline and
                // wait for the next semantic revision instead of retrying unchanged input.
                return;
            }
        };

        let mut refresh_run_presentations = false;
        match self
            .graph_run
            .apply_prepared_run(compiled)
            .expect("run existence checked above")
        {
            Ok(summary) if summary.is_empty() => {
                self.graph_run.set_running_graph_revision(revision);
                refresh_run_presentations = true;
            }
            Ok(summary) => {
                self.graph_run.set_running_graph_revision(revision);
                refresh_run_presentations = true;
                self.toasts.info(format!(
                    "live: +{} −{} cfg {} restart {}",
                    summary.added, summary.removed, summary.configured, summary.restarted
                ));
            }
            Err(runtime::ApplyError::Compile(_)) => {}
            Err(runtime::ApplyError::NeedsFullRestart(reason)) => {
                self.graph_run.set_running_graph_revision(revision);
                self.graph_run
                    .set_run_message(format!("stop & rerun to apply: {reason}"), false);
            }
            Err(runtime::ApplyError::Apply(message)) => {
                self.toasts.error(format!("live edit failed: {message}"));
            }
            Err(runtime::ApplyError::Materialization { source, .. }) => {
                self.toasts.error(format!("live edit failed: {source}"));
            }
            Err(runtime::ApplyError::Runtime(error)) => {
                self.toasts.error(format!("live edit failed: {error}"));
            }
        }

        let disconnected = self
            .graph_run
            .run()
            .expect("run existence checked above")
            .take_disconnected();
        for (node, event) in disconnected {
            if let Some(id) = node {
                self.node_graph.set_node_badge(
                    id,
                    Some(NodeBadge::warning(format!(
                        "Disconnected: can't keep up with {}.{}",
                        event.producer, event.port
                    ))),
                );
                self.error_badges.push(id);
            }
        }
        if refresh_run_presentations {
            let candidates = self
                .graph_run
                .run()
                .expect("run existence checked above")
                .sampling_overlays()
                .to_vec();
            self.set_sampling_overlay_candidates(candidates);
            self.merge_current_run_presentation_catalog();
            if let Ok(Some(feature)) = self
                .graph_run
                .service()
                .discover_live_capture_feature(self.node_graph.graph())
            {
                self.logic_analyzer
                    .set_visible_capture_channels(feature.visible_channels().iter().copied());
            }
        }
    }
    fn show_status_bar(&mut self, ui: &mut egui::Ui, actions: &[StatusAction]) {
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 30));
        ui.painter().line_segment(
            [rect.left_top(), rect.right_top()],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(78, 78, 78)),
        );
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            for action in actions {
                status_input_badge(ui, &action.input);
                ui.weak(action.label.as_str());
                ui.add_space(8.0);
            }

            // Right-aligned: `<zoom%> <selection>`, reading left to
            // right. `right_to_left` places each widget to the left of the
            // previous one, so they're added in reverse of that order —
            // `selection_summary` ends up flush with the right edge.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let about_button = ui.add_sized([52.0, 20.0], egui::Button::new("About"));
                if about_button.clicked() {
                    self.about.open();
                }
                let output_count = self.host_service.pending_output_downloads().len();
                if output_count != 0 && ui.button(format!("Downloads ({output_count})")).clicked() {
                    self.output_downloads.open();
                }
                ui.weak(self.node_graph.selection_summary());
                ui.weak(format!("{}%", self.node_graph.zoom_percent()));
            });
        });
    }

    fn show_run_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let running = self.is_running();
        let stopping = self.is_stopping();
        if running && stopping {
            // Wind-down signalled; threads are finishing their current work.
            ui.spinner();
            ui.label("Stopping…");
        } else if running {
            if ui.small_button("⏹ Stop").clicked() {
                self.stop_command();
            }
            ui.spinner();
            ui.label("Running");
        } else if self.graph_run.cache_clear_task().is_some() {
            ui.spinner();
            ui.label("Clearing derived data caches…");
        } else if self.is_capture_analysis_active() {
            ui.spinner();
            ui.label("Analyzing capture…");
        } else {
            let unavailable = self.run_unavailable_reason();
            let run = ui.add_enabled(unavailable.is_none(), egui::Button::new("▶ Run").small());
            if let Some(reason) = unavailable {
                run.clone().on_disabled_hover_text(reason);
            }
            if run.clicked() {
                self.run_command();
            }
            if self.graph_run.has_run() {
                ui.label("Finished");
            }
        }
        if let Some((message, is_error)) = self.graph_run.run_message() {
            let color = if *is_error {
                egui::Color32::from_rgb(230, 120, 120)
            } else {
                egui::Color32::from_rgb(180, 180, 180)
            };
            ui.colored_label(color, message);
        }
    }

    fn show_placeholder_panel(ui: &mut egui::Ui, title: &str) {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(format!("{title} panel"))
                    .size(16.0)
                    .weak(),
            );
        });
    }

    fn show_trigger_panel(&mut self, ui: &mut egui::Ui) {
        let Some(configuration) = self.capture_analysis.trigger_configuration() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(
                        self.capture_analysis
                            .trigger_configuration_error()
                            .unwrap_or("No trigger configuration is available"),
                    )
                    .weak(),
                );
            });
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Source:");
            ui.strong(&configuration.source_title);
        });
        ui.separator();
        let channels: Vec<_> = configuration
            .feature
            .channels()
            .iter()
            .filter(|channel| channel.enabled)
            .map(|channel| TriggerEditorChannel {
                id: channel.channel_id.clone(),
                label: channel.name.clone(),
            })
            .collect();
        let response = TriggerEditor::new(
            configuration.feature.schema(),
            &channels,
            configuration.feature.program(),
        )
        .enabled(
            self.node_graph.editing_enabled()
                && !self.capture_analysis.coordinator().is_active()
                && !self.is_capture_analysis_active(),
        )
        .show(ui);
        if let Some(error) = response.error {
            self.toasts
                .error_from(ToastSource::panel("Triggers"), error);
        }
        if let Some(program) = response.program {
            self.apply_trigger_program_edit(program);
        }
    }

    pub(crate) fn show_auxiliary_panel(&mut self, content_id: &str) {
        let order = self.auxiliary_panel_order();
        self.panel_layout.ensure_right_column_content(
            content_id,
            &order.iter().map(String::as_str).collect::<Vec<_>>(),
            RIGHT_COLUMN_LAYOUT_FRACTION,
        );
    }

    pub(crate) fn show_primary_panel(&mut self, content_id: &str) {
        let (anchor, content_first) = match content_id {
            "logic_analyzer" => ("node_graph", true),
            "node_graph" => ("logic_analyzer", false),
            _ => return,
        };
        self.panel_layout.ensure_adjacent_content(
            content_id,
            anchor,
            panel_layout::SplitAxis::Horizontal,
            content_first,
            DEFAULT_ANALYZER_SPLIT,
        );
    }

    pub(crate) fn available_auxiliary_panels(
        &self,
    ) -> Vec<(String, String, panel_layout::PanelIcon)> {
        let mut panels = vec![
            (
                "Log".to_owned(),
                "log".to_owned(),
                LOG_PANEL_ICON.panel_icon(),
            ),
            (
                "Memory".to_owned(),
                "memory".to_owned(),
                MEMORY_PANEL_ICON.panel_icon(),
            ),
            (
                "Watches".to_owned(),
                "watches".to_owned(),
                WATCHES_PANEL_ICON.panel_icon(),
            ),
            (
                "Triggers".to_owned(),
                "triggers".to_owned(),
                TRIGGERS_PANEL_ICON.panel_icon(),
            ),
            (
                "Decoder".to_owned(),
                "decoder".to_owned(),
                DECODER_PANEL_ICON.panel_icon(),
            ),
        ];
        panels.extend(
            self.presentations
                .plugin_panels()
                .definitions()
                .into_iter()
                .map(|panel| (panel.title, panel.stable_id, panel_icon(panel.icon))),
        );
        panels
    }

    fn auxiliary_panel_order(&self) -> Vec<String> {
        self.available_auxiliary_panels()
            .into_iter()
            .map(|(_, content_id, _)| content_id)
            .collect()
    }

    fn show_memory_panel(&mut self, ui: &mut egui::Ui) {
        // Repository inspection and graph cache inventory are diagnostics, never interaction
        // critical. Defer a due refresh until the pointer is released so the Memory panel cannot
        // introduce a periodic hitch while a node, panel boundary, cursor, or waveform is moving.
        let pointer_interaction_active = ui.input(|input| input.pointer.any_down());
        if self.memory_panel.refresh_due() && !pointer_interaction_active {
            let derived_lanes = self
                .presentations
                .presented_derived_lanes()
                .opaque_lanes()
                .into_iter()
                .map(|lane| DerivedSignalStorageSnapshot {
                    name: lane.name().to_owned(),
                    payload_id: lane.payload().stable_id().to_owned(),
                    storage: lane.storage_snapshot(),
                })
                .collect::<Vec<_>>();
            let graph_state = if let Some(run) = self.graph_run.run() {
                if run.is_finished() {
                    "Finished"
                } else {
                    "Running"
                }
            } else if !derived_lanes.is_empty() {
                "Cached preview"
            } else {
                "Idle"
            };
            let mut services = vec![MemoryServiceSnapshot {
                name: "Processing graph".to_owned(),
                state: graph_state.to_owned(),
                detail: format!("{} retained lane(s)", derived_lanes.len()),
                used_bytes: None,
                budget_bytes: None,
            }];
            let capture_service = self.capture_analysis.coordinator().status().map_or_else(
                || MemoryServiceSnapshot {
                    name: "Capture service".to_owned(),
                    state: "Idle".to_owned(),
                    detail: "No active capture session".to_owned(),
                    used_bytes: None,
                    budget_bytes: None,
                },
                |status| MemoryServiceSnapshot {
                    name: "Capture service".to_owned(),
                    state: capture_state_name(status.state).to_owned(),
                    detail: status.source_title.clone(),
                    used_bytes: None,
                    budget_bytes: None,
                },
            );
            services.push(capture_service);
            let mut capture = self.capture_analysis.storage().cloned();
            if let (Some(capture), Some(status)) =
                (&mut capture, self.capture_analysis.coordinator().status())
                && let Some(stored_samples) = status.health.stored_samples
            {
                capture.total_samples = Some(stored_samples);
                capture.data_bytes = Some(
                    stored_samples
                        .div_ceil(8)
                        .saturating_mul(capture.channels as u64),
                );
            }
            let cache = self.cache_memory_snapshot();
            services.extend(cache.services);
            self.memory_panel.replace_snapshot(MemoryPanelSnapshot {
                services,
                capture,
                derived_lanes,
                persistent_caches: cache.persistent_caches,
            });
        }
        self.memory_panel.show(ui);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub(crate) fn reset_panel_layout(&mut self) {
        self.panel_layout = Self::default_panel_layout();
        self.presentations
            .replace_decoder_panels(DecoderPanels::default());
        self.presentations.plugin_panels_mut().reset_state();
    }

    fn status_actions(
        &self,
        boundary_interaction: Option<BoundaryInteraction>,
        boundary_break_available: bool,
        over_panel_title: bool,
        viewer_context: Option<&str>,
        graph_context: Option<&str>,
        modifiers: egui::Modifiers,
    ) -> Vec<StatusAction> {
        let contexts = if matches!(
            boundary_interaction,
            Some(BoundaryInteraction::Dragging | BoundaryInteraction::DraggingWithParallelBoundary)
        ) {
            let mut contexts = Vec::new();
            if boundary_break_available {
                contexts.push("panel_boundary.dragging.break");
            }
            if boundary_interaction == Some(BoundaryInteraction::DraggingWithParallelBoundary) {
                contexts.push("panel_boundary.dragging.extend");
            }
            contexts.extend(["panel_boundary.dragging", "global"]);
            contexts
        } else if let Some(graph_context) = self.node_graph.active_input_context() {
            vec![graph_context, "global"]
        } else if boundary_interaction == Some(BoundaryInteraction::Hovered) {
            vec!["panel_boundary", "global"]
        } else if over_panel_title {
            vec!["panel_title", "global"]
        } else if let Some(graph_context) = graph_context {
            vec![graph_context, "global"]
        } else if let Some(viewer_context) = viewer_context {
            vec![viewer_context, "logic_analyzer", "global"]
        } else {
            vec!["global"]
        };
        let mut actions: Vec<_> = self
            .input_bindings
            .status_bindings(&contexts, modifiers)
            .into_iter()
            .filter_map(|binding| {
                StatusAction::from_binding(
                    binding,
                    modifiers,
                    self.host_ui_capabilities.modifier_key_labels,
                )
            })
            .collect();
        actions.sort_by_key(|action| action.input.sort_group());
        actions
    }
}

const STATUS_BAR_HEIGHT: f32 = 28.0;
const DEFAULT_ANALYZER_SPLIT: f32 = 0.42;
const RIGHT_COLUMN_LAYOUT_FRACTION: f32 = 0.82;
fn panel_icon(icon: PluginPanelIcon) -> PanelIcon {
    match icon {
        PluginPanelIcon::Panel => PanelIcon::Panel,
        PluginPanelIcon::Image => PanelIcon::Image,
        PluginPanelIcon::List => PanelIcon::List,
        PluginPanelIcon::Table => PanelIcon::Table,
    }
}

#[derive(Clone, Copy)]
enum MouseButtonHint {
    Left,
    Middle,
    Right,
    Wheel,
}

#[derive(Clone)]
enum StatusInput {
    Mouse {
        button: MouseButtonHint,
        gesture: Option<PointerGesture>,
    },
    Modifier {
        key: String,
        active: bool,
    },
    Key(String),
}

impl StatusInput {
    fn sort_group(&self) -> u8 {
        match self {
            Self::Mouse { .. } => 0,
            Self::Modifier { .. } => 1,
            Self::Key(_) => 2,
        }
    }
}

#[derive(Clone)]
struct StatusAction {
    input: StatusInput,
    label: String,
}

impl StatusAction {
    fn from_binding(
        binding: &input_bindings::Binding,
        active_modifiers: egui::Modifiers,
        modifier_labels: crate::ModifierKeyLabels,
    ) -> Option<Self> {
        if binding.status_modifier_only {
            let (key, active) = if binding.modifiers.control {
                ("Ctrl", active_modifiers.ctrl)
            } else if binding.modifiers.shift {
                ("Shift", active_modifiers.shift)
            } else if binding.modifiers.alt {
                (modifier_labels.alternate, active_modifiers.alt)
            } else if binding.modifiers.command {
                (modifier_labels.command, active_modifiers.command)
            } else {
                return None;
            };
            return Some(Self {
                input: StatusInput::Modifier {
                    key: key.to_owned(),
                    active,
                },
                label: binding.label.clone(),
            });
        }
        let input = match &binding.trigger {
            Trigger::Pointer { button, gesture } => StatusInput::Mouse {
                button: match button {
                    PointerButtonName::Primary => MouseButtonHint::Left,
                    PointerButtonName::Middle => MouseButtonHint::Middle,
                    PointerButtonName::Secondary => MouseButtonHint::Right,
                    PointerButtonName::Extra1 | PointerButtonName::Extra2 => return None,
                },
                gesture: Some(*gesture),
            },
            Trigger::Wheel { .. } | Trigger::Zoom => StatusInput::Mouse {
                button: MouseButtonHint::Wheel,
                gesture: None,
            },
            Trigger::Key { key } => StatusInput::Key(key_name(key)),
        };
        Some(Self {
            input,
            label: binding.label.clone(),
        })
    }
}

fn key_name(key: &str) -> String {
    match key {
        "arrow_down" => "↓".to_owned(),
        "arrow_left" => "←".to_owned(),
        "arrow_right" => "→".to_owned(),
        "arrow_up" => "↑".to_owned(),
        other if other.len() == 1 => other.to_ascii_uppercase(),
        other => other.replace('_', " "),
    }
}

fn status_input_badge(ui: &mut egui::Ui, input: &StatusInput) {
    match input {
        StatusInput::Mouse { button, gesture } => draw_mouse_badge(ui, *button, *gesture),
        StatusInput::Modifier { key, active } => draw_modifier_badge(ui, key, *active),
        StatusInput::Key(key) => draw_key_badge(ui, key),
    }
}

fn draw_modifier_badge(ui: &mut egui::Ui, key: &str, active: bool) {
    let width = (key.chars().count() as f32 * 7.0 + 10.0).max(22.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::hover());
    if active {
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_rgb(72, 92, 118));
    }
    ui.painter().rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(
            1.2,
            if active {
                egui::Color32::from_rgb(150, 190, 235)
            } else {
                egui::Color32::from_rgb(145, 145, 145)
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        key,
        egui::FontId::proportional(11.0),
        if active {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(200, 200, 200)
        },
    );
}

fn draw_mouse_badge(ui: &mut egui::Ui, button: MouseButtonHint, gesture: Option<PointerGesture>) {
    let gesture_marker = match gesture {
        Some(PointerGesture::DoubleClick) => Some("2×"),
        Some(PointerGesture::Press) => Some("↓"),
        Some(PointerGesture::Release) => Some("↑"),
        Some(PointerGesture::Hold) => Some("…"),
        Some(PointerGesture::Click | PointerGesture::Drag) | None => None,
    };
    let width = if gesture == Some(PointerGesture::Drag) || gesture_marker.is_some() {
        38.0
    } else {
        22.0
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
    let mouse = egui::Rect::from_min_size(rect.min, egui::vec2(22.0, 22.0)).shrink(1.0);
    let divider_y = mouse.top() + 8.0;
    let fill = egui::Color32::from_rgb(155, 155, 155);
    match button {
        MouseButtonHint::Left => ui.painter().rect_filled(
            egui::Rect::from_min_max(mouse.min, egui::pos2(mouse.center().x, divider_y)),
            3.0,
            fill,
        ),
        MouseButtonHint::Right => ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(mouse.center().x, mouse.top()),
                egui::pos2(mouse.right(), divider_y),
            ),
            3.0,
            fill,
        ),
        MouseButtonHint::Middle | MouseButtonHint::Wheel => ui.painter().rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(mouse.center().x, mouse.top() + 5.0),
                egui::vec2(3.5, 7.0),
            ),
            2.0,
            fill,
        ),
    };
    let stroke = egui::Stroke::new(1.2, egui::Color32::from_rgb(165, 165, 165));
    ui.painter()
        .rect_stroke(mouse, 5.0, stroke, egui::StrokeKind::Inside);
    ui.painter().line_segment(
        [
            egui::pos2(mouse.left(), divider_y),
            egui::pos2(mouse.right(), divider_y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(mouse.center().x, mouse.top()),
            egui::pos2(mouse.center().x, divider_y),
        ],
        stroke,
    );
    if gesture == Some(PointerGesture::Drag) {
        let start = egui::pos2(mouse.right() + 3.0, rect.center().y);
        let end = egui::pos2(rect.right() - 2.0, rect.center().y);
        ui.painter().line_segment([start, end], stroke);
        for (tip, direction) in [(start, -1.0), (end, 1.0)] {
            ui.painter()
                .line_segment([tip, tip + egui::vec2(-direction * 3.0, -3.0)], stroke);
            ui.painter()
                .line_segment([tip, tip + egui::vec2(-direction * 3.0, 3.0)], stroke);
        }
    } else if let Some(marker) = gesture_marker {
        ui.painter().text(
            egui::pos2(mouse.right() + 8.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            marker,
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(200, 200, 200),
        );
    }
}

fn draw_key_badge(ui: &mut egui::Ui, key: &str) {
    let width = (key.chars().count() as f32 * 7.0 + 10.0).max(22.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::hover());
    ui.painter().rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.2, egui::Color32::from_rgb(145, 145, 145)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        key,
        egui::FontId::proportional(11.0),
        egui::Color32::from_rgb(200, 200, 200),
    );
}

/// Compact item-count formatting for node headers: 950 → "950", 12_345 →
/// "12.3k", 5_600_000 → "5.6M", 2_100_000_000 → "2.1G".
fn format_count(items: u64) -> String {
    match items {
        0..=999 => items.to_string(),
        1_000..=999_999 => format!("{:.1}k", items as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", items as f64 / 1_000_000.0),
        _ => format!("{:.1}G", items as f64 / 1_000_000_000.0),
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        KIB..=1_048_575 => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        MIB..=1_073_741_823 => format!("{:.1} MiB", bytes as f64 / MIB as f64),
        _ => format!("{:.1} GiB", bytes as f64 / GIB as f64),
    }
}

fn capture_state_name(state: signal_capture_session::CaptureSessionState) -> &'static str {
    match state {
        signal_capture_session::CaptureSessionState::Preparing => "Preparing",
        signal_capture_session::CaptureSessionState::Prepared => "Prepared",
        signal_capture_session::CaptureSessionState::Armed => "Armed",
        signal_capture_session::CaptureSessionState::Triggered => "Triggered",
        signal_capture_session::CaptureSessionState::Recording => "Recording",
        signal_capture_session::CaptureSessionState::Stopping => "Stopping…",
        signal_capture_session::CaptureSessionState::Complete => "Complete",
        signal_capture_session::CaptureSessionState::Error => "Error",
    }
}

fn capture_outcome_name(outcome: signal_capture_session::CaptureSessionOutcome) -> &'static str {
    match outcome {
        signal_capture_session::CaptureSessionOutcome::InProgress => "In progress",
        signal_capture_session::CaptureSessionOutcome::Complete => "Complete",
        signal_capture_session::CaptureSessionOutcome::Stopped => "Stopped",
        signal_capture_session::CaptureSessionOutcome::CancelledBeforeTrigger => {
            "Cancelled before trigger"
        }
        signal_capture_session::CaptureSessionOutcome::Incomplete => "Incomplete",
        signal_capture_session::CaptureSessionOutcome::Aborted => "Aborted",
        signal_capture_session::CaptureSessionOutcome::Corrupt => "Corrupt",
    }
}

/// Adds symbol-font fallbacks for menu and control glyphs that egui's default
/// fonts don't cover. Bundled Noto faces take priority so every host uses
/// consistent symbol metrics; host-supplied system faces remain last-resort
/// fallbacks.
fn install_fonts(ctx: &egui::Context, host_symbol_fonts: Vec<egui::FontData>) {
    let mut fonts = egui::FontDefinitions::default();
    for (index, font_data) in bundled_symbol_fonts()
        .into_iter()
        .chain(host_symbol_fonts)
        .enumerate()
    {
        let font_name = format!("system-symbols-{index}");
        fonts
            .font_data
            .insert(font_name.clone(), std::sync::Arc::new(font_data));
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .push(font_name.clone());
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .push(font_name);
    }
    ctx.set_fonts(fonts);
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.presentations
            .decoder_panels_mut()
            .filter_raw_input(raw_input);
        self.node_graph.filter_modal_raw_input(raw_input);
        self.platform_raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.platform_logic(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.platform_save(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.platform_before_ui(ui);

        for catalog in &mut self.node_catalogs {
            let snapshot = catalog.snapshot();
            if let Some(templates) = catalog.take_templates() {
                self.node_graph
                    .replace_node_templates(&snapshot.namespace, templates);
            }
            if snapshot.scanning {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let viewport_rect = ui.available_rect_before_wrap();
        self.poll_capture(ui.ctx());
        if self.platform_sync_capture() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.sync_run(ui.ctx());
        let plugin_panel_definitions = self.presentations.plugin_panels().definitions();
        let mut specs = vec![
            PanelSpec::new("logic_analyzer", "Logic Analyzer", 160.0)
                .icon(LOGIC_ANALYZER_PANEL_ICON.panel_icon())
                .minimum_width(220.0)
                .singleton(),
            PanelSpec::new("node_graph", "Node Graph", 160.0)
                .icon(NODE_GRAPH_PANEL_ICON.panel_icon())
                .minimum_width(220.0)
                .singleton(),
            PanelSpec::new("log", "Log", 160.0)
                .icon(LOG_PANEL_ICON.panel_icon())
                .minimum_width(240.0),
            PanelSpec::new("memory", "Memory", 160.0)
                .icon(MEMORY_PANEL_ICON.panel_icon())
                .minimum_width(320.0)
                .singleton(),
            PanelSpec::new("watches", "Watches", 120.0)
                .icon(WATCHES_PANEL_ICON.panel_icon())
                .minimum_width(180.0),
            PanelSpec::new("triggers", "Triggers", 120.0)
                .icon(TRIGGERS_PANEL_ICON.panel_icon())
                .minimum_width(180.0),
            PanelSpec::new("decoder", "Decoder", 120.0)
                .icon(DECODER_PANEL_ICON.panel_icon())
                .minimum_width(220.0),
        ];
        specs.extend(plugin_panel_definitions.iter().map(|panel| {
            let spec = PanelSpec::new(
                panel.stable_id.as_str(),
                panel.title.as_str(),
                panel.minimum_height,
            )
            .icon(panel_icon(panel.icon))
            .minimum_width(panel.minimum_width);
            if panel.singleton {
                spec.singleton()
            } else {
                spec
            }
        }));
        let mut panel_layout = std::mem::take(&mut self.panel_layout);
        panel_layout
            .set_maximize_shortcut(self.input_bindings.shortcut(&["panel"], "toggle_maximize"));
        let mut viewer_subscriptions_changed = false;
        let layout_response = panel_layout.show(
            ui,
            viewport_rect,
            STATUS_BAR_HEIGHT,
            &specs,
            |slot, panel_ui| match slot {
                PanelSlot::TitleBar {
                    content_id: "logic_analyzer",
                    ..
                } => {
                    self.show_logic_analyzer_status(panel_ui);
                }
                PanelSlot::TitleBar {
                    content_id: "node_graph",
                    ..
                } => self.show_run_controls(panel_ui),
                PanelSlot::Body {
                    content_id: "logic_analyzer",
                    ..
                } => {
                    self.refresh_timeline_markers();
                    self.logic_analyzer
                        .set_timeline_marker_editing_enabled(self.node_graph.editing_enabled());
                    self.logic_analyzer.show(panel_ui);
                    self.sync_viewer_lane_order();
                    self.sync_viewer_lane_heights();
                    if let Some(edit) = self.logic_analyzer.take_simple_trigger_edit() {
                        self.apply_simple_trigger_edit(edit);
                    }
                    if let Some(edit) = self.logic_analyzer.take_timeline_marker_edit() {
                        self.apply_timeline_marker_edit(edit);
                    }
                }
                PanelSlot::Body {
                    content_id: "node_graph",
                    ..
                } => {
                    self.platform_before_graph();
                    let panel_data = self.refresh_graph_output_selections();
                    let panel_actions = self.node_graph.show_with_panel_data(panel_ui, &panel_data);
                    if let Some(message) = self.node_graph.take_io_status() {
                        self.toasts
                            .info_from(ToastSource::panel("Node Graph"), message);
                    }
                    if let Some((node_id, action_id)) = self.node_graph.take_node_context_action() {
                        self.handle_node_context_action(node_id, &action_id);
                        // The analyzer panel can already have been painted earlier in this
                        // frame. A completed run no longer schedules periodic frames, so
                        // explicitly draw the newly selected sampling overlay on a later frame.
                        panel_ui
                            .ctx()
                            .request_repaint_after(std::time::Duration::from_millis(16));
                    }
                    for action in panel_actions {
                        let node_id = action.node();
                        if action.panel_id() == VIEWER_OUTPUT_PANEL_ID
                            && let Ok(ViewerOutputPanelAction::SetSelected { id, selected }) =
                                action.downcast::<ViewerOutputPanelAction>()
                        {
                            if let Err(error) = set_viewer_output_selected(
                                self.node_graph.graph_mut(),
                                node_id,
                                &id,
                                selected,
                            ) {
                                let source = self.toast_source_for_socket(node_id, id);
                                self.toasts.error_from(
                                    source,
                                    format!("Could not update the viewer selection: {error}"),
                                );
                            } else {
                                self.synchronize_payload_subscription_manifest(false);
                                viewer_subscriptions_changed = true;
                            }
                        }
                    }
                    self.platform_after_graph();
                }
                PanelSlot::Body {
                    content_id: "log", ..
                } => self.toasts.show_history(panel_ui),
                PanelSlot::Body {
                    content_id: "memory",
                    ..
                } => self.show_memory_panel(panel_ui),
                PanelSlot::Body {
                    content_id: "watches",
                    ..
                } => Self::show_placeholder_panel(panel_ui, "Watches"),
                PanelSlot::Body {
                    content_id: "triggers",
                    ..
                } => self.show_trigger_panel(panel_ui),
                PanelSlot::Body {
                    panel_id,
                    content_id: "decoder",
                    ..
                } => self
                    .presentations
                    .decoder_panels_mut()
                    .show(panel_id, panel_ui),
                PanelSlot::Body {
                    panel_id,
                    content_id,
                    ..
                } => {
                    if let Some(warning) = self
                        .presentations
                        .plugin_panels_mut()
                        .show(content_id, panel_id, panel_ui)
                    {
                        self.toasts
                            .warning_from(ToastSource::panel(content_id), warning.to_string());
                    }
                }
                PanelSlot::TitleBar { .. } => {}
            },
        );
        self.panel_layout = panel_layout;

        let presentation_ownership_changed = self.synchronize_presentation_graph_nodes();
        let view_panel_state_changed = self.node_graph.take_contributed_panel_state_changed();
        if viewer_subscriptions_changed || view_panel_state_changed {
            self.refresh_view_configuration_after_edit();
            ui.ctx().request_repaint();
        }
        if presentation_ownership_changed {
            ui.ctx().request_repaint();
        }

        let viewer_cursors_changed = self.sync_timeline_cursor_setting();
        self.synchronize_timeline_marker_references(viewer_cursors_changed);
        self.sync_timeline_cursor_setting();

        let viewer = layout_response.content_panel("logic_analyzer");
        let graph = layout_response.content_panel("node_graph");

        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let modifiers = ui.input(|i| i.modifiers);
        let over_panel_title = pointer_pos.is_some_and(|pos| {
            layout_response.panels.iter().any(|panel| {
                panel
                    .title_interaction_rect
                    .is_some_and(|rect| rect.contains(pos))
            })
        });
        let status_actions = self.status_actions(
            layout_response.boundary_interaction,
            layout_response.boundary_break_available,
            over_panel_title,
            viewer
                .filter(|viewer| pointer_pos.is_some_and(|pos| viewer.body_rect.contains(pos)))
                .map(|_| self.logic_analyzer.hovered_input_context()),
            graph
                .filter(|graph| pointer_pos.is_some_and(|pos| graph.body_rect.contains(pos)))
                .and_then(|_| self.node_graph.hovered_input_context()),
            modifiers,
        );
        let mut status_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("application-status-bar")
                .max_rect(layout_response.footer_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        status_ui.set_clip_rect(layout_response.footer_rect);
        self.show_status_bar(&mut status_ui, &status_actions);

        self.about.show(ui.ctx());
        self.preferences.show(ui.ctx(), &mut self.node_catalogs);

        for error in self
            .output_downloads
            .show(ui.ctx(), self.host_service.as_mut())
        {
            self.toasts
                .error(format!("Could not download output: {error}"));
        }

        self.platform_after_ui(ui.ctx());

        self.toasts.show(ui.ctx());
    }
}

#[cfg(test)]
mod font_tests {
    use std::collections::HashSet;

    use logic_analyzer_graph_capabilities::node_support::{
        PortKind, ResolvedInput, TimelineMarkerReference, TimelineMarkerReferenceBindingDescriptor,
        TimelineMarkerReferenceChoice,
    };
    use logic_analyzer_graph_plan::{
        CollectedOutputLane, CollectedOutputSubscription, CollectedTableSubscription,
    };
    use node_graph::api::{
        GraphState, Node, NodeId, Socket, SocketIndicatorPresentation, SocketShape,
    };
    use signal_derived::Word;

    use super::{
        PluginPanelsState, SavedViewerRow, StatusAction, TIMELINE_CURSORS_EXTENSION,
        ViewerSocketIndicator, bundled_symbol_fonts, install_fonts, save_panel_layout,
        save_sampling_overlays, save_timeline_cursors, save_viewer_lane_heights,
        save_viewer_lane_order, saved_panel_layout, saved_sampling_overlays,
        saved_timeline_cursors, saved_viewer_lane_heights, saved_viewer_lane_order,
        timeline_cursor_schema_version,
    };
    use crate::panel_presentation::{
        DECODER_PANEL_ICON, LOG_PANEL_ICON, LOGIC_ANALYZER_PANEL_ICON, MEMORY_PANEL_ICON,
        NODE_GRAPH_PANEL_ICON, TRIGGERS_PANEL_ICON, WATCHES_PANEL_ICON,
    };
    use crate::plugin_panel::PluginPanelRegistry;
    use crate::presentation_catalogs::PresentationCatalogs;
    use crate::timeline_marker_bindings::TimelineMarkerBindings;

    #[test]
    fn built_in_panels_have_unique_icons() {
        let icons = [
            LOGIC_ANALYZER_PANEL_ICON,
            NODE_GRAPH_PANEL_ICON,
            LOG_PANEL_ICON,
            MEMORY_PANEL_ICON,
            WATCHES_PANEL_ICON,
            TRIGGERS_PANEL_ICON,
            DECODER_PANEL_ICON,
        ];
        for (index, icon) in icons.iter().enumerate() {
            assert!(
                !icons[..index].contains(icon),
                "built-in panel icon {icon:?} is assigned more than once"
            );
        }
    }

    fn output_socket() -> Socket {
        Socket {
            schema_id: "out".to_owned(),
            name: "Out".to_owned(),
            type_name: "Word".to_owned(),
            color: node_graph::api::GraphColor::from_rgb(255, 255, 255),
            shape: SocketShape::Circle,
            allowed: Vec::new(),
            resolved_type: None,
            def_index: 0,
            variadic: None,
            visible: true,
            editor_visible: true,
            hidden: false,
            has_control: false,
            extensions: [("show_in_view".to_owned(), serde_json::Value::Bool(true))]
                .into_iter()
                .collect(),
        }
    }

    fn output_subscription(source_node: NodeId) -> CollectedOutputSubscription {
        CollectedOutputSubscription {
            runtime_name: "collector".to_owned(),
            lanes: vec![CollectedOutputLane {
                member: 0,
                lane_name: "Decoder.Out".to_owned(),
                source_label: "Decoder".to_owned(),
                input: ResolvedInput {
                    kind: PortKind::of::<Word>(),
                    source: "decoder.out".to_owned(),
                    source_node,
                    source_output: 0,
                    source_node_title: "Decoder".to_owned(),
                    source_output_title: "Out".to_owned(),
                    word_display_format: None,
                    lane_presentation: None,
                    default_lane_presentation: None,
                    decoder_table_column: None,
                    capture_channel: None,
                },
            }],
        }
    }

    #[test]
    fn empty_cursor_choices_are_a_stable_synchronized_state() {
        let binding = TimelineMarkerReferenceBindingDescriptor {
            id: "cursor".into(),
            selected: None,
            timestamp_ns: 250_000_000,
            choices: Vec::new(),
        };

        assert!(TimelineMarkerBindings::reference_binding_is_synchronized(
            &binding,
            &[]
        ));
    }

    #[test]
    fn cursor_time_changes_require_reference_resynchronization() {
        let reference = TimelineMarkerReference::Cursor { number: 1 };
        let previous = TimelineMarkerReferenceChoice::new(reference, "Cursor 1", 10);
        let binding = TimelineMarkerReferenceBindingDescriptor {
            id: "cursor".into(),
            selected: Some(reference),
            timestamp_ns: 10,
            choices: vec![previous],
        };
        let moved = TimelineMarkerReferenceChoice::new(reference, "Cursor 1", 20);

        assert!(!TimelineMarkerBindings::reference_binding_is_synchronized(
            &binding,
            &[moved]
        ));
    }

    #[test]
    fn application_input_bindings_are_valid() {
        let bindings =
            input_bindings::InputBindings::from_json(include_str!("../config/input_bindings.json"))
                .expect("invalid application input binding configuration");
        assert_eq!(
            bindings.shortcut(&["panel"], "toggle_maximize"),
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::Space,
            ))
        );
        assert_eq!(
            bindings.shortcut(&["node_graph"], "delete"),
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::X,
            ))
        );
        assert_eq!(
            bindings
                .bindings(&["node_graph"], "delete")
                .into_iter()
                .map(|binding| binding.to_string())
                .collect::<Vec<_>>(),
            ["X", "Delete"]
        );
        assert_eq!(
            bindings.pointer_trigger(
                &["logic_analyzer"],
                "measure_edge_delta",
                egui::Modifiers::NONE,
            ),
            Some((
                egui::PointerButton::Primary,
                input_bindings::PointerGesture::Click,
            ))
        );
    }

    #[test]
    fn viewer_socket_indicator_scales_with_graph_zoom() {
        let indicator = ViewerSocketIndicator;

        assert_eq!(indicator.size(0.5), egui::vec2(6.0, 3.7));
        assert_eq!(indicator.size(3.0), egui::vec2(36.0, 22.2));
    }

    #[test]
    fn sampling_overlay_selections_round_trip_and_migrate_the_single_selection() {
        let mut graph = GraphState::default();
        save_sampling_overlays(&mut graph, &[NodeId(17), NodeId(23)]).unwrap();

        let json = serde_json::to_string(&graph).unwrap();
        let mut restored: GraphState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            saved_sampling_overlays(&restored).unwrap(),
            (vec![NodeId(17), NodeId(23)], false)
        );

        restored
            .set_extension(super::SAMPLING_OVERLAY_EXTENSION, NodeId(31))
            .unwrap();
        assert_eq!(
            saved_sampling_overlays(&restored).unwrap(),
            (vec![NodeId(31)], true)
        );

        save_sampling_overlays(&mut restored, &[]).unwrap();
        assert_eq!(saved_sampling_overlays(&restored).unwrap(), (vec![], false));
    }

    #[test]
    fn sampling_overlay_toggles_do_not_replace_other_decoder_selections() {
        let mut catalogs =
            PresentationCatalogs::new(PluginPanelRegistry::standard().unwrap(), HashSet::new());
        catalogs.replace_selected_sampling_overlays(vec![NodeId(17)]);

        catalogs.toggle_sampling_overlay(NodeId(23));
        assert_eq!(
            catalogs.selected_sampling_overlays(),
            [NodeId(17), NodeId(23)]
        );

        catalogs.toggle_sampling_overlay(NodeId(17));
        assert_eq!(catalogs.selected_sampling_overlays(), [NodeId(23)]);
    }

    #[test]
    fn derived_lane_visibility_follows_node_delete_and_undo_without_losing_catalog_data() {
        let mut graph = GraphState::default();
        let node_id = graph.next_id();
        let mut node = Node::blank(
            node_id,
            "Test Decoder",
            node_graph::api::GraphPosition::ZERO,
        );
        node.outputs.push(output_socket());
        graph.add_node(node.clone());
        let catalog = vec![output_subscription(node_id)];
        let table_catalog = vec![CollectedTableSubscription {
            collector: NodeId(99),
            lanes: catalog[0].lanes.clone(),
        }];
        let mut catalogs = PresentationCatalogs::new(
            PluginPanelRegistry::standard().unwrap(),
            graph.nodes.keys().copied().collect(),
        );
        catalogs.replace_catalogs(catalog, table_catalog);

        assert_eq!(catalogs.visible_output_subscriptions(&graph).len(), 1);
        assert_eq!(catalogs.visible_table_subscriptions(&graph).len(), 1);

        graph.remove_node(node_id);
        assert!(catalogs.visible_output_subscriptions(&graph).is_empty());
        assert!(catalogs.visible_table_subscriptions(&graph).is_empty());
        catalogs.merge_run_catalogs(&[], &[]);

        graph.add_node(node);
        let visible = catalogs.visible_output_subscriptions(&graph);
        assert_eq!(visible.len(), 1);
        assert_eq!(catalogs.visible_table_subscriptions(&graph).len(), 1);
        assert_eq!(visible[0].lanes[0].lane_name, "Decoder.Out");
    }

    #[test]
    fn timeline_cursors_round_trip_with_the_graph_document() {
        let mut graph = GraphState::default();
        let cursors = vec![logic_analyzer_viewer::TimeCursor {
            number: 2,
            time_us: 123.5,
        }];
        save_timeline_cursors(&mut graph, &cursors).unwrap();

        let json = serde_json::to_string(&graph).unwrap();
        let mut restored: GraphState = serde_json::from_str(&json).unwrap();
        assert_eq!(saved_timeline_cursors(&restored).unwrap(), cursors);

        save_timeline_cursors(&mut restored, &[]).unwrap();
        assert!(saved_timeline_cursors(&restored).unwrap().is_empty());
    }

    #[test]
    fn unsupported_timeline_cursor_extension_is_preserved_unchanged() {
        let mut graph = GraphState::default();
        let future = serde_json::json!({
            "version": timeline_cursor_schema_version() + 1,
            "cursors": [{"number": 4, "time_us": 12.5}],
            "future_owner_data": {"keep": true}
        });
        graph
            .set_extension(TIMELINE_CURSORS_EXTENSION, &future)
            .unwrap();

        assert!(saved_timeline_cursors(&graph).is_err());
        assert!(save_timeline_cursors(&mut graph, &[]).is_err());
        assert_eq!(
            graph
                .extension::<serde_json::Value>(TIMELINE_CURSORS_EXTENSION)
                .unwrap(),
            Some(future)
        );
    }

    #[test]
    fn viewer_lane_order_round_trips_with_the_graph_document() {
        let mut graph = GraphState::default();
        let order = vec![
            SavedViewerRow::Derived("node-7:decoded.words".to_owned()),
            SavedViewerRow::Channel(3),
            SavedViewerRow::Channel(0),
        ];
        save_viewer_lane_order(&mut graph, &order).unwrap();

        let json = serde_json::to_string(&graph).unwrap();
        let mut restored: GraphState = serde_json::from_str(&json).unwrap();
        assert_eq!(saved_viewer_lane_order(&restored).unwrap(), order);

        save_viewer_lane_order(&mut restored, &[]).unwrap();
        assert!(saved_viewer_lane_order(&restored).unwrap().is_empty());
    }

    #[test]
    fn viewer_lane_heights_round_trip_with_the_graph_document() {
        let mut graph = GraphState::default();
        let settings = logic_analyzer_viewer::ViewerRowHeightSettings {
            global_scale: 1.25,
            rows: vec![logic_analyzer_viewer::ViewerRowHeight {
                row: logic_analyzer_viewer::ViewerRowId::Channel(3),
                scale: 1.5,
            }],
        };
        save_viewer_lane_heights(&mut graph, &settings).unwrap();

        let json = serde_json::to_string(&graph).unwrap();
        let mut restored: GraphState = serde_json::from_str(&json).unwrap();
        assert_eq!(saved_viewer_lane_heights(&restored).unwrap(), settings);

        save_viewer_lane_heights(
            &mut restored,
            &logic_analyzer_viewer::ViewerRowHeightSettings {
                global_scale: 1.0,
                rows: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            saved_viewer_lane_heights(&restored).unwrap(),
            logic_analyzer_viewer::ViewerRowHeightSettings {
                global_scale: 1.0,
                rows: Vec::new(),
            }
        );
    }

    #[test]
    fn panel_layout_round_trips_with_the_graph_document() {
        let mut graph = GraphState::default();
        let mut layout =
            panel_layout::PanelLayout::new([("logic_analyzer", 0.42), ("node_graph", 0.58)]);
        layout.ensure_right_column_content_count(
            "decoder",
            2,
            &["watches", "triggers", "decoder"],
            0.82,
        );
        let expected_layout = serde_json::to_value(layout.state()).unwrap();

        save_panel_layout(
            &mut graph,
            layout.state().clone(),
            crate::decoder_panel::DecoderPanelsState::default(),
            PluginPanelsState::default(),
        )
        .unwrap();

        let json = serde_json::to_string(&graph).unwrap();
        let restored: GraphState = serde_json::from_str(&json).unwrap();
        let saved = saved_panel_layout(&restored).unwrap().unwrap();
        assert_eq!(serde_json::to_value(saved.layout).unwrap(), expected_layout);
    }

    #[test]
    fn interaction_status_bindings_change_during_panel_and_node_drags() {
        let bindings =
            input_bindings::InputBindings::from_json(include_str!("../config/input_bindings.json"))
                .expect("invalid application input binding configuration");

        let boundary: Vec<_> = bindings
            .status_bindings(&["panel_boundary"], egui::Modifiers::NONE)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(boundary, ["Resize Panels", "Panel Options"]);

        let viewer: Vec<_> = bindings
            .status_bindings(&["logic_analyzer"], egui::Modifiers::NONE)
            .into_iter()
            .filter_map(|binding| {
                StatusAction::from_binding(
                    binding,
                    egui::Modifiers::NONE,
                    crate::ModifierKeyLabels::default(),
                )
            })
            .collect();
        assert!(viewer.iter().any(|action| {
            action.label == "Measure Edge Delta"
                && matches!(
                    &action.input,
                    super::StatusInput::Mouse {
                        gesture: Some(super::PointerGesture::Click),
                        ..
                    }
                )
        }));
        assert!(viewer.iter().any(|action| {
            action.label == "Pan View"
                && matches!(
                    &action.input,
                    super::StatusInput::Mouse {
                        gesture: Some(super::PointerGesture::Drag),
                        ..
                    }
                )
        }));

        let resizing: Vec<_> = bindings
            .status_bindings(&["panel_boundary.dragging"], egui::Modifiers::NONE)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(resizing, ["Finish Resize", "Snap to Grid / Boundaries"]);

        let snapping_panels: Vec<_> = bindings
            .status_bindings(&["panel_boundary.dragging"], egui::Modifiers::CTRL)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(
            snapping_panels,
            ["Finish Resize", "Snap to Grid / Boundaries"]
        );

        let extending_panels: Vec<_> = bindings
            .status_bindings(
                &["panel_boundary.dragging.extend", "panel_boundary.dragging"],
                egui::Modifiers::NONE,
            )
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(
            extending_panels,
            ["Extend", "Finish Resize", "Snap to Grid / Boundaries"]
        );

        let mut rendered_extending_panels: Vec<_> = bindings
            .status_bindings(
                &["panel_boundary.dragging.extend", "panel_boundary.dragging"],
                egui::Modifiers::NONE,
            )
            .into_iter()
            .filter_map(|binding| {
                StatusAction::from_binding(
                    binding,
                    egui::Modifiers::NONE,
                    crate::ModifierKeyLabels::default(),
                )
            })
            .collect();
        rendered_extending_panels.sort_by_key(|action| action.input.sort_group());
        let rendered_labels: Vec<_> = rendered_extending_panels
            .iter()
            .map(|action| action.label.as_str())
            .collect();
        assert_eq!(
            rendered_labels,
            ["Finish Resize", "Extend", "Snap to Grid / Boundaries"]
        );

        let mut rendered_breaking_panels: Vec<_> = bindings
            .status_bindings(
                &[
                    "panel_boundary.dragging.break",
                    "panel_boundary.dragging.extend",
                    "panel_boundary.dragging",
                ],
                egui::Modifiers::NONE,
            )
            .into_iter()
            .filter_map(|binding| {
                StatusAction::from_binding(
                    binding,
                    egui::Modifiers::NONE,
                    crate::ModifierKeyLabels::default(),
                )
            })
            .collect();
        rendered_breaking_panels.sort_by_key(|action| action.input.sort_group());
        let rendered_labels: Vec<_> = rendered_breaking_panels
            .iter()
            .map(|action| action.label.as_str())
            .collect();
        assert_eq!(
            rendered_labels,
            [
                "Finish Resize",
                "Break",
                "Extend",
                "Snap to Grid / Boundaries",
            ]
        );

        let title_bar: Vec<_> = bindings
            .status_bindings(&["panel_title"], egui::Modifiers::NONE)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(title_bar, ["Maximize / Restore Area", "Area Options"]);

        let dragging: Vec<_> = bindings
            .status_bindings(&["node_graph.drag_node"], egui::Modifiers::NONE)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(
            dragging,
            ["Confirm", "Cancel", "Snap to Grid", "X Axis", "Y Axis"]
        );

        let snapping: Vec<_> = bindings
            .status_bindings(&["node_graph.drag_node"], egui::Modifiers::CTRL)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(
            snapping,
            ["Confirm", "Cancel", "Snap to Grid", "X Axis", "Y Axis"]
        );

        let wire_drag: Vec<_> = bindings
            .status_bindings(&["node_graph.drag_wire"], egui::Modifiers::NONE)
            .into_iter()
            .map(|binding| binding.label.as_str())
            .collect();
        assert_eq!(wire_drag, ["Drag Node-link", "Confirm Link", "Cancel"]);
    }

    #[test]
    fn menu_icon_glyphs_are_available() {
        assert!(
            !bundled_symbol_fonts().is_empty(),
            "missing bundled symbol font"
        );
        let ctx = egui::Context::default();
        install_fonts(&ctx, Vec::new());
        #[cfg(debug_assertions)]
        assert!(
            ctx.style_of(egui::Theme::Dark)
                .debug
                .warn_if_rect_changes_id
        );
        // `set_fonts` only takes effect at the start of the *next* pass.
        ctx.begin_pass(Default::default());
        let _ = ctx.end_pass();
        ctx.begin_pass(Default::default());
        let font_id = egui::FontId::proportional(14.0);
        ctx.fonts_mut(|fonts| {
            const MENU_GLYPHS: &[char] = &[
                '⇧', '⌘', '⌥', '⇪', '⏎', '↶', '↷', '⌧', '⎘', '⧉', '▣', '▼', '▾',
            ];
            for c in MENU_GLYPHS {
                assert!(
                    fonts.has_glyph(&font_id, *c),
                    "missing glyph for {c:?} (U+{:04X})",
                    *c as u32
                );
            }
        });
        let _ = ctx.end_pass();
    }
}
