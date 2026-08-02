use std::sync::Arc;

use logic_analyzer_graph_api::node::PayloadRegistration;
use logic_analyzer_graph_api::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
};
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneInteraction, ViewerLaneInteractionContext, ViewerLaneRenderer,
    ViewerLaneRendererRegistration, ViewerLaneTrack, draw_trigger_activity, draw_trigger_snapshot,
};
use signal_processing::{OpaqueCollectedLaneSnapshot, TriggerLaneSnapshot};

const RENDERER: &str = "org.logicconduit.renderer.trigger/v1";

pub(crate) struct TriggerSnapshotRenderer;

impl ViewerLaneRenderer for TriggerSnapshotRenderer {
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) = snapshot.and_then(|snapshot| snapshot.value::<TriggerLaneSnapshot>())
        else {
            return false;
        };
        match snapshot.as_ref() {
            TriggerLaneSnapshot::Exact(markers) => draw_trigger_snapshot(&context, markers),
            TriggerLaneSnapshot::Activity(records) => draw_trigger_activity(&context, records),
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
        let snapshot = snapshot?.value::<TriggerLaneSnapshot>()?;
        let timestamps: Vec<u64> = match snapshot.as_ref() {
            TriggerLaneSnapshot::Exact(markers) => markers.clone(),
            TriggerLaneSnapshot::Activity(records) => {
                records.iter().map(|record| record.start_ns).collect()
            }
        };
        let mut value = false;
        let transitions = timestamps
            .into_iter()
            .map(|timestamp| {
                value = !value;
                (timestamp, value)
            })
            .collect();
        Some(ViewerLaneInteraction {
            initial: false,
            transitions,
            event: true,
        })
    }
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("T", [230, 190, 80]), RENDERER)
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || Arc::new(TriggerSnapshotRenderer))
}

inventory::submit! {
    PayloadRegistration::subscribable_with_persistent_cache::<signal_processing::Trigger>(
        "org.logicconduit.trigger/v1",
        signal_processing::trigger_payload_adapter,
        presentation,
    )
}
