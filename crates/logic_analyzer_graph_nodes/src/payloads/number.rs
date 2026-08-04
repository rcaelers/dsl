use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
};
use logic_analyzer_graph_registry::PayloadRegistration;
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTrack,
    draw_value_activity, draw_value_snapshot,
};
use signal_processing::{NumberLaneSnapshot, OpaqueCollectedLaneSnapshot};

const RENDERER: &str = "org.logicconduit.renderer.number/v1";

pub(crate) struct NumberSnapshotRenderer;

impl ViewerLaneRenderer for NumberSnapshotRenderer {
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) = snapshot.and_then(|snapshot| snapshot.value::<NumberLaneSnapshot>())
        else {
            return false;
        };
        let color = context.theme.accent;
        match snapshot.as_ref() {
            NumberLaneSnapshot::Exact(samples) => {
                let values = samples
                    .iter()
                    .map(|sample| (sample.start_time_ns, sample.value.to_string()))
                    .collect::<Vec<_>>();
                draw_value_snapshot(&context, &values, color);
            }
            NumberLaneSnapshot::Activity(records) => draw_value_activity(&context, records, color),
        }
        true
    }
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("N", [95, 145, 210]), RENDERER)
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || Arc::new(NumberSnapshotRenderer))
}

inventory::submit! {
    PayloadRegistration::subscribable_with_persistent_cache::<signal_processing::NumberSample>(
        "org.logicconduit.number-sample/v1",
        signal_processing::number_payload_adapter,
        presentation,
    )
}
