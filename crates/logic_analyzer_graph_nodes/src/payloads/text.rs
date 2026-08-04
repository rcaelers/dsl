use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
};
use logic_analyzer_graph_registry::PayloadRegistration;
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTrack,
    draw_value_activity, draw_value_snapshot,
};
use signal_processing::{OpaqueCollectedLaneSnapshot, TextLaneSnapshot};

const RENDERER: &str = "org.logicconduit.renderer.text/v1";

pub(crate) struct TextSnapshotRenderer;

impl ViewerLaneRenderer for TextSnapshotRenderer {
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) = snapshot.and_then(|snapshot| snapshot.value::<TextLaneSnapshot>())
        else {
            return false;
        };
        let color = context.theme.accent;
        match snapshot.as_ref() {
            TextLaneSnapshot::Exact(samples) => {
                let values = samples
                    .iter()
                    .map(|sample| (sample.start_time_ns, sample.value.clone()))
                    .collect::<Vec<_>>();
                draw_value_snapshot(&context, &values, color);
            }
            TextLaneSnapshot::Activity(records) => draw_value_activity(&context, records, color),
        }
        true
    }
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("TXT", [215, 150, 170]),
        RENDERER,
    )
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || Arc::new(TextSnapshotRenderer))
}

inventory::submit! {
    PayloadRegistration::subscribable_with_persistent_cache::<signal_processing::TextSample>(
        "org.logicconduit.text-sample/v1",
        signal_processing::text_payload_adapter,
        presentation,
    )
}
