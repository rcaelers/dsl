use std::fmt::Write;
use std::sync::Arc;

use logic_analyzer_graph_capabilities::node::{ProtocolPacketDisplay, protocol_packet_display};
use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, PortKind,
};
use logic_analyzer_graph_registry::PayloadRegistration;
use logic_analyzer_viewer::{
    OpaqueLaneDrawContext, ViewerLaneRenderer, ViewerLaneRendererRegistration, ViewerLaneTrack,
    draw_event_snapshot, draw_span_snapshot,
};
use signal_processing::{
    OpaqueCollectedLaneSnapshot, ProtocolPacket, ProtocolPacketLaneSnapshot, ProtocolValue,
    protocol_packet_payload_adapter,
};

const RENDERER: &str = "org.logicconduit.renderer.protocol-packet/v1";
const MAX_COLLECTION_ITEMS: usize = 8;
const MAX_DEPTH: usize = 4;
const MAX_BYTES: usize = 16;
const MAX_TEXT_CHARS: usize = 48;

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
        if !snapshot.activity_spans().is_empty() {
            let activity = snapshot
                .activity_spans()
                .iter()
                .map(|&(start, end)| (start, end, "…".to_owned()))
                .collect::<Vec<_>>();
            draw_span_snapshot(&context, &activity, context.theme.accent);
        }
        let mut spans = Vec::new();
        let mut events = Vec::new();
        for packet in snapshot.packets() {
            let display = packet_display(packet);
            if !display.is_visible() {
                continue;
            }
            if display.is_marker() {
                events.push((packet.start_time_ns, display.label().to_owned()));
            } else {
                spans.push((
                    packet.start_time_ns,
                    packet.end_time_ns,
                    display.label().to_owned(),
                ));
            }
        }
        draw_span_snapshot(&context, &spans, context.theme.accent);
        draw_event_snapshot(&context, &events, context.theme.accent);
        true
    }
}

fn packet_display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
    protocol_packet_display(packet).unwrap_or_else(|| {
        ProtocolPacketDisplay::new(format!(
            "{} · {}",
            packet.protocol_id,
            generic_value_label(&packet.value, 0)
        ))
    })
}

fn generic_value_label(value: &ProtocolValue, depth: usize) -> String {
    if depth >= MAX_DEPTH {
        return "…".to_owned();
    }
    match value {
        ProtocolValue::Null => "null".to_owned(),
        ProtocolValue::Bool(value) => value.to_string(),
        ProtocolValue::Integer(value) => value.to_string(),
        ProtocolValue::Float(value) => value.to_string(),
        ProtocolValue::String(value) => {
            let truncated = value.chars().take(MAX_TEXT_CHARS).collect::<String>();
            let suffix = (value.chars().count() > MAX_TEXT_CHARS).then_some("…");
            serde_json::to_string(&(truncated + suffix.unwrap_or_default())).unwrap()
        }
        ProtocolValue::Bytes(value) => {
            let mut label = String::from("h'");
            for byte in value.iter().take(MAX_BYTES) {
                let _ = write!(label, "{byte:02X}");
            }
            if value.len() > MAX_BYTES {
                label.push('…');
            }
            label.push('\'');
            label
        }
        ProtocolValue::List(values) => collection_label("[", "]", values, depth),
        ProtocolValue::Tuple(values) => collection_label("(", ")", values, depth),
        ProtocolValue::Mapping(values) => {
            let mut entries = values
                .iter()
                .take(MAX_COLLECTION_ITEMS)
                .map(|(key, value)| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(key).unwrap(),
                        generic_value_label(value, depth + 1)
                    )
                })
                .collect::<Vec<_>>();
            if values.len() > MAX_COLLECTION_ITEMS {
                entries.push(format!("… +{}", values.len() - MAX_COLLECTION_ITEMS));
            }
            format!("{{{}}}", entries.join(", "))
        }
    }
}

fn collection_label(
    opening: &str,
    closing: &str,
    values: &[ProtocolValue],
    depth: usize,
) -> String {
    let mut items = values
        .iter()
        .take(MAX_COLLECTION_ITEMS)
        .map(|value| generic_value_label(value, depth + 1))
        .collect::<Vec<_>>();
    if values.len() > MAX_COLLECTION_ITEMS {
        items.push(format!("… +{}", values.len() - MAX_COLLECTION_ITEMS));
    }
    format!("{opening}{}{closing}", items.join(", "))
}

fn presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(LaneBadgeDescriptor::new("P", [175, 120, 205]), RENDERER)
}

fn kind() -> PortKind {
    PortKind::of_named::<ProtocolPacket>("Protocol Packet")
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

#[cfg(test)]
mod protocol_packet_tests {
    use std::collections::BTreeMap;

    use super::*;

    fn packet(protocol_id: &str, value: ProtocolValue) -> ProtocolPacket {
        ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 0,
            end_time_ns: 1,
            protocol_id: protocol_id.to_owned(),
            value,
        }
    }

    #[test]
    fn unknown_protocol_uses_a_bounded_value_fallback() {
        let value = ProtocolValue::Mapping(BTreeMap::from([
            (
                "bytes".to_owned(),
                ProtocolValue::Bytes(vec![0x12, 0xAB].into()),
            ),
            (
                "data".to_owned(),
                ProtocolValue::List(vec![
                    ProtocolValue::String("ACK".to_owned()),
                    ProtocolValue::Integer(42),
                ]),
            ),
        ]));

        assert_eq!(
            packet_display(&packet("org.example.unknown/v1", value)).label(),
            "org.example.unknown/v1 · {\"bytes\": h'12AB', \"data\": [\"ACK\", 42]}"
        );
    }

    #[test]
    fn deeply_nested_values_are_bounded() {
        let value =
            ProtocolValue::List(vec![ProtocolValue::List(vec![ProtocolValue::List(vec![
                ProtocolValue::List(vec![ProtocolValue::Integer(1)]),
            ])])]);

        assert_eq!(generic_value_label(&value, 0), "[[[[…]]]]");
    }
}
