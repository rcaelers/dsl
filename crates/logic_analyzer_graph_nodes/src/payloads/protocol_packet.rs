use std::sync::Arc;

use logic_analyzer_graph_api::node::PayloadRegistration;
use logic_analyzer_graph_api::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, PortKind,
};
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTrack,
    draw_span_snapshot,
};
use signal_processing::{
    OpaqueCollectedLaneSnapshot, ProtocolPacketLaneSnapshot, protocol_packet_payload_adapter,
};

const RENDERER: &str = "org.logicconduit.renderer.protocol-packet/v1";

struct ProtocolPacketRenderer;

impl ViewerLaneRenderer for ProtocolPacketRenderer {
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        let Some(snapshot) =
            snapshot.and_then(|snapshot| snapshot.value::<ProtocolPacketLaneSnapshot>())
        else {
            return false;
        };
        let values = snapshot
            .packets()
            .iter()
            .map(|packet| {
                (
                    packet.start_time_ns,
                    packet.end_time_ns,
                    packet.display_text(),
                )
            })
            .collect::<Vec<_>>();
        draw_span_snapshot(&context, &values, context.theme.accent);
        if !snapshot.activity_spans().is_empty() {
            let activity = snapshot
                .activity_spans()
                .iter()
                .map(|&(start, end)| (start, end, String::new()))
                .collect::<Vec<_>>();
            draw_span_snapshot(&context, &activity, context.theme.accent);
        }
        true
    }
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("P", [175, 120, 205]), RENDERER)
}

fn kind() -> PortKind {
    PortKind::of_named::<signal_processing::ProtocolPacket>("Protocol Packet")
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(RENDERER, || Arc::new(ProtocolPacketRenderer))
}

inventory::submit! {
    PayloadRegistration::subscribable_kind(
        "org.logicconduit.protocol-packet/v1",
        kind,
        protocol_packet_payload_adapter,
        presentation,
    )
}
