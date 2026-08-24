//! Generic waveform and derived-lane presentation.
//!
//! The viewer renders generic capture and derived-data contracts, including lanes,
//! sampling overlays, cursor interactions, and renderer registrations. Concrete
//! nodes provide explicit presentation metadata and stable renderer keys; this
//! crate never infers protocol behavior from node names, labels, or payload values.
//!
//! # Getting started
//!
//! Create a [`LogicAnalyzerViewer`] and call [`LogicAnalyzerViewer::show`] once per
//! egui frame. It owns viewport interaction, cursor and edge measurements, row
//! renaming/reordering, visible-window sampling, and painting; a host owns source
//! preparation and repaint policy for active processing.
//!
//! # Concepts and terminology
//!
//! The viewer renders three independently supplied row kinds in one reorderable
//! list:
//!
//! - An **indexed capture** is a host-prepared generic [`signal_capture::CaptureIndex`]
//!   attached with `set_prepared_capture`. The viewer never opens a file, chooses a
//!   repository, or builds that index.
//! - An **in-memory channel** is a [`ChannelSignal`] containing an initial level and
//!   increasing `(time_us, level)` transitions, attached with `set_channels`.
//! - A **derived lane** is a runtime-published entry in
//!   [`signal_derived::DerivedLanes`], attached with `set_derived_lanes`.
//!
//! A lane's presentation is explicit. The host builds a
//! [`WaveformPresentationRegistry`] from stable renderer registrations and neutral
//! lane descriptors. A [`ViewerLaneGroup`] may combine several payload lanes in one
//! row; its [`ViewerLaneRenderer`] receives a bounded immutable snapshot and never
//! executes while the runtime lane catalog is locked. Renderer keys and payload
//! identities—not node display names or protocol labels—select the presentation.
//!
//! # Host responsibilities
//!
//! Prepare and attach finite or growing captures before showing them. Use a fresh
//! [`signal_derived::DerivedLanes`] store for each run to clear old derived output
//! atomically, and set the corresponding presentation registry. Concrete sources,
//! decoders, file formats, cache policy, and target-specific acquisition remain
//! outside this reusable widget. The same API and source compile on native and wasm;
//! the host decides which prepared sources are available.

#[cfg(test)]
mod architecture_tests;
mod channel;
mod cursor;
mod derived_snapshot;
mod draw;
mod edge_measurement;
mod format;
mod input;
mod lanes;
mod renderer_registration;
mod sampling;
mod sampling_overlay;
mod scrollbar;
mod simple_trigger;
mod timeline_marker;
mod types;
mod viewer;

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
pub use sampling_overlay::SamplingOverlay;
pub use simple_trigger::{SimpleTriggerEdit, SimpleTriggerLane};
pub use timeline_marker::{TimelineMarker, TimelineMarkerEdit};
pub use types::{ColorProfile, TimeCursor, ViewerRowHeight, ViewerRowHeightSettings, ViewerRowId};
pub use viewer::{ChannelSignal, LogicAnalyzerViewer, ViewerUiPrefs};
