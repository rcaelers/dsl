use std::sync::Arc;

use egui::{Color32, Stroke};

use logic_analyzer_graph_api::node::PayloadRegistration;
use logic_analyzer_viewer::{
    AnnotationVisual, DerivedLaneId, ViewerLaneInteraction, ViewerLaneInteractionContext,
    ViewerLaneRenderer, ViewerLaneTheme, ViewerLaneTrack, ViewerLaneTrackId,
};
use signal_processing::{
    DigitalLaneSnapshot, NumberSample, OpaqueCollectedLaneSnapshot, Sample, TextSample, Trigger,
    TriggerLaneSnapshot, Word,
};

use super::digital::DigitalSnapshotRenderer;
use super::trigger::TriggerSnapshotRenderer;
use super::word::WordSnapshotRenderer;

struct SemanticRenderer;

#[test]
fn every_built_in_lane_payload_supports_persistent_restoration() {
    let registrations = inventory::iter::<PayloadRegistration>
        .into_iter()
        .collect::<Vec<_>>();

    for type_id in [
        std::any::TypeId::of::<Sample>(),
        std::any::TypeId::of::<Word>(),
        std::any::TypeId::of::<Trigger>(),
        std::any::TypeId::of::<NumberSample>(),
        std::any::TypeId::of::<TextSample>(),
    ] {
        let registration = registrations
            .iter()
            .find(|registration| registration.kind().type_id() == type_id)
            .expect("built-in lane payload registration");
        assert!(registration.persistent_cache());
    }
}

impl ViewerLaneRenderer for SemanticRenderer {
    fn annotation_visual(
        &self,
        _track: &ViewerLaneTrackId,
        _theme: &ViewerLaneTheme,
        value: u64,
        mut default: AnnotationVisual,
    ) -> AnnotationVisual {
        default.label = format!("semantic-{value}");
        default
    }
}

#[test]
fn word_snapshot_renderer_requests_snapshots_and_delegates_semantics() {
    let renderer = WordSnapshotRenderer::new(Arc::new(SemanticRenderer));
    let track = ViewerLaneTrack::new("data", DerivedLaneId::new("words"), 1.0);
    let default = AnnotationVisual {
        label: "default".to_owned(),
        fill: Color32::BLACK,
        border: Stroke::new(1.0, Color32::WHITE),
    };
    assert_eq!(
        renderer
            .annotation_visual(&track.id, &test_theme(), 42, default)
            .label,
        "semantic-42"
    );
}

#[test]
fn digital_snapshot_projects_payload_neutral_interaction() {
    let renderer = DigitalSnapshotRenderer;
    let track = ViewerLaneTrack::new("signal", DerivedLaneId::new("signal"), 1.0);
    let snapshot = OpaqueCollectedLaneSnapshot::new(Arc::new(DigitalLaneSnapshot::Exact {
        samples: vec![Sample::new(true, 10), Sample::new(false, 20)],
        initial: false,
    }));

    assert_eq!(
        renderer.interaction(&track, Some(&snapshot), interaction_context()),
        Some(ViewerLaneInteraction {
            initial: false,
            transitions: vec![(10, true), (20, false)],
            event: false,
        })
    );
}

#[test]
fn trigger_snapshot_projects_event_interaction() {
    let renderer = TriggerSnapshotRenderer;
    let track = ViewerLaneTrack::new("trigger", DerivedLaneId::new("trigger"), 1.0);
    let snapshot =
        OpaqueCollectedLaneSnapshot::new(Arc::new(TriggerLaneSnapshot::Exact(vec![10, 20, 30])));

    assert_eq!(
        renderer.interaction(&track, Some(&snapshot), interaction_context()),
        Some(ViewerLaneInteraction {
            initial: false,
            transitions: vec![(10, true), (20, false), (30, true)],
            event: true,
        })
    );
}

fn test_theme() -> ViewerLaneTheme {
    ViewerLaneTheme::from_visuals(&egui::Visuals::dark(), Color32::LIGHT_BLUE)
}

fn interaction_context() -> ViewerLaneInteractionContext {
    ViewerLaneInteractionContext {
        visible_start_ns: 0,
        visible_end_ns: 100,
        max_items: 100,
        hovered: false,
        pointer_time_ns: None,
    }
}
