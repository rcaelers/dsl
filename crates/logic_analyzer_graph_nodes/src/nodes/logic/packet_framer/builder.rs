//! Runtime builder for `Packet Framer`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodePresentation, GraphNodeSemantics, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    DecoderTableColumnDescriptor, NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_protocol_decoders::packet_framer::{
    GatePolarity, PACKET_FRAME_PROTOCOL_ID, PacketFramer,
};
use logic_analyzer_protocol_decoders::types::ProtocolPacket;
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_derived::{TimestampEvent, Word};
use signal_runtime::ProcessNode;

#[derive(Default)]
pub(crate) struct PacketFramerBuilder;

impl PacketFramerBuilder {
    fn parsed(state: &Value) -> Result<super::definition::PacketFramerState, String> {
        parse_state(state).map_err(|error| error.to_string())
    }

    fn delimiter(state: &super::definition::PacketFramerState) -> Result<Option<u64>, String> {
        let delimiter = state.delimiter.value.trim();
        if delimiter.is_empty() {
            Ok(None)
        } else {
            super::super::word_value::parse_hex_u64(delimiter).map(Some)
        }
    }
}

impl GraphNodeSemantics for PacketFramerBuilder {
    fn accepted_kinds(&self, socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        match socket.definition_index() {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<TimestampEvent>()],
            2 => vec![PortKind::of::<Sample>()],
            _ => Vec::new(),
        }
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of_named::<ProtocolPacket>("Protocol Packet")]
    }

    fn offered_connection_contracts(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
    ) -> Vec<String> {
        (socket.definition_index() == 0)
            .then(|| PACKET_FRAME_PROTOCOL_ID.to_owned())
            .into_iter()
            .collect()
    }

    fn input_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        match socket.definition_index() {
            0 if kind == PortKind::of::<Word>() => Some("words".to_owned()),
            1 if kind == PortKind::of::<TimestampEvent>() => Some("boundary".to_owned()),
            2 if kind == PortKind::of::<Sample>() => Some("gate".to_owned()),
            _ => None,
        }
    }

    fn output_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        (socket.definition_index() == 0
            && kind == PortKind::of_named::<ProtocolPacket>("Protocol Packet"))
        .then(|| "packets".to_owned())
    }

    fn input_required(&self, socket: SocketReference<'_>, _state: &Value) -> bool {
        socket.definition_index() == 0
    }
}

impl RuntimeMaterializer for PacketFramerBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state = Self::parsed(state)?;
        let fixed_word_count = usize::try_from(state.words_per_packet.value.max(0))
            .map_err(|_| "words-per-packet is outside the supported range")?;
        let maximum_words = usize::try_from(state.maximum_words.value.max(1))
            .map_err(|_| "hard word limit is outside the supported range")?;
        let maximum_gap_ns = (state.maximum_gap_us.value > 0)
            .then(|| (state.maximum_gap_us.value as u64).saturating_mul(1_000));
        let gate_polarity = resolved.kind(2).map(|_| {
            if state.gate_polarity.selected() == "Active low" {
                GatePolarity::ActiveLow
            } else {
                GatePolarity::ActiveHigh
            }
        });
        Ok(Box::new(
            PacketFramer::new()
                .with_name(name)
                .with_fixed_word_count((fixed_word_count > 0).then_some(fixed_word_count))
                .with_delimiter(
                    Self::delimiter(&state)?,
                    state.delimiter_policy.selected() == "Include",
                )
                .with_maximum_gap_ns(maximum_gap_ns)
                .with_maximum_words(maximum_words.max(fixed_word_count))
                .with_boundary_input(resolved.kind(1).is_some())
                .with_gate_input(gate_polarity),
        ))
    }
}

impl GraphNodePresentation for PacketFramerBuilder {
    fn decoder_table_column(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        super::presentation::packet_table_column(socket.definition_index())
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;
    use node_graph_document::SocketDirection;

    use super::super::definition::PacketFramer;
    use super::*;

    #[test]
    fn only_words_are_required_but_every_socket_has_a_runtime_port() {
        let builder = PacketFramerBuilder;
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(PacketFramer::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node];

        assert!(builder.input_required(
            node.inputs[0].reference(SocketDirection::Input, 0),
            &node.state
        ));
        assert!(!builder.input_required(
            node.inputs[1].reference(SocketDirection::Input, 0),
            &node.state
        ));
        assert!(!builder.input_required(
            node.inputs[2].reference(SocketDirection::Input, 0),
            &node.state
        ));
        assert_eq!(
            node.inputs
                .iter()
                .zip([
                    PortKind::of::<Word>(),
                    PortKind::of::<TimestampEvent>(),
                    PortKind::of::<Sample>()
                ])
                .map(|(socket, kind)| {
                    builder.input_port(
                        socket.reference(SocketDirection::Input, 0),
                        &node.state,
                        kind,
                    )
                })
                .collect::<Vec<_>>(),
            [
                Some("words".to_owned()),
                Some("boundary".to_owned()),
                Some("gate".to_owned())
            ]
        );
    }

    #[test]
    fn packet_output_declares_its_generic_protocol_contract() {
        let builder = PacketFramerBuilder;
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(PacketFramer::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node];

        assert_eq!(
            builder.offered_connection_contracts(
                node.outputs[0].reference(SocketDirection::Output, 0),
                &node.state
            ),
            [PACKET_FRAME_PROTOCOL_ID]
        );
    }
}
