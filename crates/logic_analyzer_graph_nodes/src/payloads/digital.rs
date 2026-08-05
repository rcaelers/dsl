use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
};
use logic_analyzer_graph_registry::PayloadRegistration;
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneInteraction, ViewerLaneInteractionContext, ViewerLaneRenderer,
    ViewerLaneRendererRegistration, ViewerLaneTrack, draw_digital_activity, draw_digital_snapshot,
};
use signal_derived::{DigitalLaneSnapshot, OpaqueCollectedLaneSnapshot};

const RENDERER: &str = "org.logicconduit.renderer.digital/v1";

pub(crate) struct DigitalSnapshotRenderer;

impl ViewerLaneRenderer for DigitalSnapshotRenderer {
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) = snapshot.and_then(|snapshot| snapshot.value::<DigitalLaneSnapshot>())
        else {
            return false;
        };
        match snapshot.as_ref() {
            DigitalLaneSnapshot::Exact { samples, initial } => {
                draw_digital_snapshot(&context, samples, *initial)
            }
            DigitalLaneSnapshot::Activity { records, initial } => {
                draw_digital_activity(&context, records, *initial)
            }
        }
        true
    }

    fn supports_interaction(&self) -> bool {
        true
    }

    fn interaction(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        _context: ViewerLaneInteractionContext,
    ) -> Option<ViewerLaneInteraction> {
        let snapshot = snapshot?.value::<DigitalLaneSnapshot>()?;
        let (initial, transitions) = match snapshot.as_ref() {
            DigitalLaneSnapshot::Exact { samples, initial } => (
                *initial,
                samples
                    .iter()
                    .map(|sample| (sample.start_time_ns, sample.value))
                    .collect(),
            ),
            DigitalLaneSnapshot::Activity { records, initial } => {
                (*initial, activity_transitions(records))
            }
        };
        Some(ViewerLaneInteraction {
            initial,
            transitions,
            event: false,
        })
    }
}

fn activity_transitions(records: &[signal_derived::MipmapRecord]) -> Vec<(u64, bool)> {
    let mut transitions = Vec::with_capacity(records.len().saturating_mul(2));
    for record in records {
        let Some((first, last)) = record.level_hint else {
            continue;
        };
        transitions.push((record.start_ns, first));
        if first != last {
            transitions.push((record.end_ns, last));
        }
    }
    transitions
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("S", [95, 175, 95]), RENDERER)
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || Arc::new(DigitalSnapshotRenderer))
}

inventory::submit! {
    PayloadRegistration::subscribable_with_persistent_cache::<signal_capture::Sample>(
        "org.logicconduit.digital-sample/v1",
        signal_derived::digital_payload_adapter,
        presentation,
    )
}
