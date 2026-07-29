//! Protocol-owned presentation for native I²C decoder packets.

use std::sync::Arc;

use logic_analyzer_graph_api::node::{
    ProtocolPacketDisplay, ProtocolPacketPresentationRegistration,
};
use logic_analyzer_graph_api::node_support::{DecoderTableCellMode, DecoderTableColumnDescriptor};
use logic_analyzer_processing::nodes::decoders::i2c_decoder::I2C_PROTOCOL_ID;
use logic_analyzer_viewer::{DefaultViewerLaneRenderer, ViewerLaneRendererRegistration};
use signal_processing::{ProtocolPacket, ProtocolValue};

const I2C_TABLE_RENDERER: &str = "org.logicconduit.renderer.i2c-table/v1";

pub(crate) fn i2c_table_column(def_index: usize) -> Option<DecoderTableColumnDescriptor> {
    (def_index == 1).then(|| {
        DecoderTableColumnDescriptor::new(
            "i2c",
            "event",
            "I²C",
            0,
            true,
            DecoderTableCellMode::Single,
            "event",
            I2C_TABLE_RENDERER,
        )
    })
}

inventory::submit! {
    ProtocolPacketPresentationRegistration::new(I2C_PROTOCOL_ID, i2c_packet_display)
}

inventory::submit! {
    ViewerLaneRendererRegistration::new(I2C_TABLE_RENDERER, || {
        Arc::new(DefaultViewerLaneRenderer)
    })
}

fn i2c_packet_display(packet: &ProtocolPacket) -> ProtocolPacketDisplay {
    let command = match &packet.value {
        ProtocolValue::List(values) => match values.first() {
            Some(ProtocolValue::String(command)) => Some(command.as_str()),
            _ => None,
        },
        _ => None,
    };
    match command {
        Some("BITS") => ProtocolPacketDisplay::hidden(),
        Some("START" | "START REPEAT" | "STOP" | "ACK" | "NACK") => {
            ProtocolPacketDisplay::marker(i2c_packet_label(packet))
        }
        _ => ProtocolPacketDisplay::new(i2c_packet_label(packet)),
    }
}

fn i2c_packet_label(packet: &ProtocolPacket) -> String {
    let ProtocolValue::List(values) = &packet.value else {
        return packet.display_text();
    };
    let Some(ProtocolValue::String(command)) = values.first() else {
        return packet.display_text();
    };
    let integer = values.get(1).and_then(|value| match value {
        ProtocolValue::Integer(value) if *value >= 0 => Some(*value),
        _ => None,
    });
    match (command.as_str(), integer) {
        ("START", _) => "Start".to_owned(),
        ("START REPEAT", _) => "Repeated start".to_owned(),
        ("STOP", _) => "Stop".to_owned(),
        ("ACK", _) => "ACK".to_owned(),
        ("NACK", _) => "NACK".to_owned(),
        ("ADDRESS WRITE", Some(address)) => format!("Address 0x{address:02X} · Write"),
        ("ADDRESS READ", Some(address)) => format!("Address 0x{address:02X} · Read"),
        ("DATA WRITE", Some(data)) => format!("Write 0x{data:02X}"),
        ("DATA READ", Some(data)) => format!("Read 0x{data:02X}"),
        ("BITS", _) => "Bits".to_owned(),
        _ => command.clone(),
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    fn packet(command: &str, value: ProtocolValue) -> ProtocolPacket {
        ProtocolPacket {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: 10,
            end_time_ns: 20,
            protocol_id: I2C_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::List(vec![ProtocolValue::String(command.to_owned()), value]),
        }
    }

    #[test]
    fn presentation_distinguishes_address_direction_data_and_repeated_start() {
        assert_eq!(
            i2c_packet_label(&packet("ADDRESS WRITE", ProtocolValue::Integer(0x50))),
            "Address 0x50 · Write"
        );
        assert_eq!(
            i2c_packet_label(&packet("DATA READ", ProtocolValue::Integer(0xab))),
            "Read 0xAB"
        );
        assert_eq!(
            i2c_packet_label(&packet("START REPEAT", ProtocolValue::Null)),
            "Repeated start"
        );
        assert!(!i2c_packet_display(&packet("BITS", ProtocolValue::List(Vec::new()))).is_visible());
        assert!(i2c_packet_display(&packet("ACK", ProtocolValue::Null)).is_marker());
        assert_eq!(i2c_table_column(1).unwrap().source_key, "i2c");
    }
}
