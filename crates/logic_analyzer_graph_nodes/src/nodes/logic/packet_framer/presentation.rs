//! Protocol-owned and decoder-table presentation for framed packets.

use std::fmt::Write;

use logic_analyzer_graph_capabilities::node::ProtocolPacketDisplay;
use logic_analyzer_graph_capabilities::node_support::{
    DecoderTableCellMode, DecoderTableColumnDescriptor,
};
use logic_analyzer_graph_registry::ProtocolPacketPresentationRegistration;
use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};
use signal_derived::{ProtocolPacket, ProtocolValue};
use signal_transforms::packet_framer::PACKET_FRAME_PROTOCOL_ID;

const PACKET_TABLE_RENDERER: &str = "org.logicconduit.renderer.packet-frame-table/v1";
const MAX_LABEL_WORDS: usize = 8;
const MAX_LABEL_BYTES: usize = 16;

pub(crate) fn packet_table_column(def_index: usize) -> Option<DecoderTableColumnDescriptor> {
    (def_index == 0).then(|| {
        DecoderTableColumnDescriptor::new(
            "frames",
            "packet",
            "Packet",
            0,
            true,
            DecoderTableCellMode::Single,
            "packet",
            PACKET_TABLE_RENDERER,
        )
    })
}

inventory::submit! {
    ProtocolPacketPresentationRegistration::new(
        PACKET_FRAME_PROTOCOL_ID,
        packet_display,
    )
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(PACKET_TABLE_RENDERER, || {
        std::sync::Arc::new(DefaultViewerLaneRenderer)
    })
}

fn packet_display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
    ProtocolPacketDisplay::new(packet_label(packet))
}

fn packet_label(packet: &ProtocolPacket) -> String {
    let ProtocolValue::List(words) = &packet.value else {
        return packet.display_text();
    };
    let mut label = String::from("[");
    for (index, word) in words.iter().take(MAX_LABEL_WORDS).enumerate() {
        if index > 0 {
            label.push(' ');
        }
        label.push_str(&word_label(word));
    }
    if words.len() > MAX_LABEL_WORDS {
        let _ = write!(label, " … +{}", words.len() - MAX_LABEL_WORDS);
    }
    label.push(']');
    label
}

fn word_label(word: &ProtocolValue) -> String {
    let ProtocolValue::Mapping(fields) = word else {
        return "?".to_owned();
    };
    match fields.get("payload") {
        Some(ProtocolValue::Bytes(bytes)) => {
            let mut label = String::from("0x");
            for byte in bytes.iter().take(MAX_LABEL_BYTES) {
                let _ = write!(label, "{byte:02X}");
            }
            if bytes.len() > MAX_LABEL_BYTES {
                label.push('…');
            }
            label
        }
        Some(ProtocolValue::String(text)) => format!("\"{text}\""),
        _ => match fields.get("value") {
            Some(ProtocolValue::Integer(value)) if *value >= 0 => format!("0x{value:X}"),
            Some(ProtocolValue::Integer(value)) => value.to_string(),
            _ => "?".to_owned(),
        },
    }
}

#[cfg(test)]
mod presentation_tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn packets_are_an_explicit_table_source() {
        assert!(packet_table_column(1).is_none());
        let table = packet_table_column(0).unwrap();
        assert_eq!(table.source_key, "frames");
        assert_eq!(table.column_key, "packet");
        assert!(table.row_anchor);
    }

    #[test]
    fn packet_protocol_registers_its_value_aware_display() {
        let packet = ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 10,
            end_time_ns: 20,
            protocol_id: PACKET_FRAME_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::List(vec![ProtocolValue::Mapping(BTreeMap::from([(
                "value".to_owned(),
                ProtocolValue::Integer(0x12),
            )]))]),
        };

        assert_eq!(
            logic_analyzer_graph_registry::protocol_packet_display(&packet)
                .unwrap()
                .label(),
            "[0x12]"
        );
    }

    #[test]
    fn packet_labels_show_numeric_byte_and_text_word_values() {
        let word = |value, payload| {
            ProtocolValue::Mapping(BTreeMap::from([
                ("value".to_owned(), ProtocolValue::Integer(value)),
                ("payload".to_owned(), payload),
            ]))
        };
        let packet = ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 10,
            end_time_ns: 20,
            protocol_id: PACKET_FRAME_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::List(vec![
                ProtocolValue::Mapping(BTreeMap::from([(
                    "value".to_owned(),
                    ProtocolValue::Integer(0x12),
                )])),
                word(0, ProtocolValue::Bytes(vec![0xAB, 0xCD].into())),
                word(7, ProtocolValue::String("ACK".to_owned())),
            ]),
        };

        assert_eq!(packet_label(&packet), "[0x12 0xABCD \"ACK\"]");
    }

    #[test]
    fn long_packet_labels_are_bounded() {
        let packet = ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 10,
            end_time_ns: 20,
            protocol_id: PACKET_FRAME_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::List(
                (0..10)
                    .map(|value| {
                        ProtocolValue::Mapping(BTreeMap::from([(
                            "value".to_owned(),
                            ProtocolValue::Integer(value),
                        )]))
                    })
                    .collect(),
            ),
        };

        assert_eq!(
            packet_label(&packet),
            "[0x0 0x1 0x2 0x3 0x4 0x5 0x6 0x7 … +2]"
        );
    }
}
