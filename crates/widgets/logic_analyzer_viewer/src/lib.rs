#[cfg(test)]
mod architecture_tests;
mod channel;
mod cursor;
mod draw;
mod edge_measurement;
mod format;
mod input;
mod lanes;
mod renderer_registration;
mod sampling;
mod sampling_overlay;
mod simple_trigger;
mod timeline_marker;
mod types;
mod viewer;
std::cfg_select! {
    target_arch = "wasm32" => {
        #[path = "worker_wasm.rs"]
        mod worker;
    }
    _ => {
        mod worker;
    }
}

pub use draw::{
    default_annotation_visual, draw_annotation_presence, draw_annotation_snapshot,
    draw_digital_activity, draw_digital_snapshot, draw_event_snapshot, draw_span_snapshot,
    draw_trigger_activity, draw_trigger_snapshot, draw_value_activity, draw_value_snapshot,
};
pub use lanes::{
    AnnotationVisual, DefaultViewerLaneRenderer, DerivedLaneId, OpaqueLaneDrawContext,
    ViewerLaneBadge, ViewerLaneGroup, ViewerLaneGroupId, ViewerLaneInteraction,
    ViewerLaneInteractionContext, ViewerLaneRenderer, ViewerLaneTheme, ViewerLaneTrack,
    ViewerLaneTrackId, WaveformPresentationRegistry,
};
pub use renderer_registration::{ViewerLaneRendererRegistration, viewer_lane_renderer};
pub use sampling_overlay::{SamplingEdge, SamplingOverlay, SamplingQualifier};
pub use simple_trigger::{SimpleTriggerEdit, SimpleTriggerLane};
pub use timeline_marker::{TimelineMarker, TimelineMarkerEdit};
pub use types::{ColorProfile, TimeCursor, ViewerRowHeight, ViewerRowHeightSettings, ViewerRowId};
pub use viewer::{ChannelSignal, LogicAnalyzerViewer};
