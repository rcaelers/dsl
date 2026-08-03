//! Runtime builder for `SPI Decoder`.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    DecoderTableColumnDescriptor, LanePresentationDescriptor, NodeBuildContext, PortKind,
    ResolvedInputs, SamplingOverlayDescriptor, ViewerOutputControl, parse_state,
};
use logic_analyzer_processing::nodes::decoders::spi_decoder::{
    SPI_TRANSACTION_PROTOCOL_ID, SpiDecoder, SpiMode,
};
use logic_analyzer_processing::types::{BitOrder, CsPolarity};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, ProtocolPacket, Sample, Word};

#[derive(Default)]
pub(crate) struct SpiDecoderBuilder;

impl SpiDecoderBuilder {
    fn parsed(state: &Value) -> Result<super::definition::SpiDecoderState, String> {
        parse_state(state)
    }
    fn cs_polarity(state: &super::definition::SpiDecoderState) -> CsPolarity {
        match state.cs_polarity.selected() {
            "Active high" => CsPolarity::ActiveHigh,
            "Disabled" => CsPolarity::Disabled,
            _ => CsPolarity::ActiveLow,
        }
    }
}

impl RuntimeBuilder for SpiDecoderBuilder {
    fn execution_state(&self, state: &Value) -> Value {
        crate::presentation::without_display_format(state)
    }

    fn viewer_output_control(
        &self,
        socket: &Socket,
        _state: &Value,
    ) -> Option<ViewerOutputControl> {
        match socket.def_index {
            2 | 3 => Some(ViewerOutputControl::new(false, [0])),
            4 | 5 => Some(ViewerOutputControl::new(false, [1])),
            6 => Some(ViewerOutputControl::new(false, [6])),
            _ => Some(ViewerOutputControl::Hidden),
        }
    }

    fn lane_presentation(
        &self,
        socket: &Socket,
        _state: &Value,
    ) -> Option<LanePresentationDescriptor> {
        super::presentation::spi_output_presentation(socket.def_index)
    }

    fn decoder_table_column(
        &self,
        socket: &Socket,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        super::presentation::spi_table_column(socket.def_index)
    }

    fn word_display_format(&self, socket: &Socket, state: &Value) -> Option<String> {
        if !matches!(socket.def_index, 3 | 5) {
            return None;
        }
        Self::parsed(state)
            .ok()
            .map(|state| state.display_format.selected().to_string())
    }

    fn sampling_overlay(&self, state: &Value) -> Option<SamplingOverlayDescriptor> {
        Self::parsed(state).ok()?;
        Some(SamplingOverlayDescriptor {
            clock_input: 0,
            sampled_input_groups: vec![1, 2],
            retained_word_source: None,
        })
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn offered_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        if socket.def_index == 6 {
            vec![PortKind::of_named::<ProtocolPacket>("Protocol Packet")]
        } else {
            vec![PortKind::of::<Word>()]
        }
    }
    fn offered_connection_contracts(&self, socket: &Socket, _state: &Value) -> Vec<String> {
        (socket.def_index == 6)
            .then(|| SPI_TRANSACTION_PROTOCOL_ID.to_owned())
            .into_iter()
            .collect()
    }
    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("clk".into()),
            1 => Some("mosi".into()),
            2 => Some("miso".into()),
            3 => Some("cs".into()),
            _ => None,
        }
    }
    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        if socket.def_index == 6 && kind == PortKind::of_named::<ProtocolPacket>("Protocol Packet")
        {
            return Some("transactions".to_owned());
        }
        if kind == PortKind::of::<Word>() {
            return match socket.def_index {
                0 => Some("mosi_words".into()),
                1 => Some("miso_words".into()),
                2 => Some("mosi_bits".into()),
                3 => Some("mosi_data".into()),
                4 => Some("miso_bits".into()),
                5 => Some("miso_data".into()),
                _ => None,
            };
        }
        None
    }
    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        let Ok(state) = Self::parsed(state) else {
            return true;
        };
        match socket.def_index {
            0 | 1 => true,
            2 => false,
            3 => Self::cs_polarity(&state) != CsPolarity::Disabled,
            _ => false,
        }
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state = Self::parsed(state)?;
        let mode = match (state.cpol.selected(), state.cpha.selected()) {
            ("0", "0") => SpiMode::Mode0,
            ("0", "1") => SpiMode::Mode1,
            ("1", "0") => SpiMode::Mode2,
            ("1", "1") => SpiMode::Mode3,
            _ => return Err("invalid CPOL/CPHA".into()),
        };
        let bit_order = if state.bit_order.selected() == "LSB first" {
            BitOrder::LsbFirst
        } else {
            BitOrder::MsbFirst
        };
        let mut decoder = SpiDecoder::with_cs_polarity(
            mode,
            state.word_size.value.clamp(1, 64) as usize,
            true,
            resolved.member_count(2) > 0,
            Self::cs_polarity(&state),
        )
        .with_bit_order(bit_order)
        .with_name(name);
        if let Some(points) = ctx.sampling_points(name) {
            decoder = decoder.with_sampling_points(points);
        }
        Ok(Box::new(decoder))
    }
}

#[cfg(test)]
mod tests {
    use node_graph::NodeDef;

    use super::super::definition::SpiDecoder;
    use super::*;

    #[test]
    fn sampling_overlay_identifies_clock_and_data_inputs_for_all_spi_modes() {
        let builder = SpiDecoderBuilder;
        let mut state = SpiDecoder::state();
        for (cpol, cpha) in [("0", "0"), ("0", "1"), ("1", "0"), ("1", "1")] {
            state.cpol.select(cpol);
            state.cpha.select(cpha);
            let descriptor = builder
                .sampling_overlay(&serde_json::to_value(&state).unwrap())
                .unwrap();
            assert_eq!(descriptor.clock_input, 0);
            assert_eq!(descriptor.sampled_input_groups, [1, 2]);
        }
    }

    #[test]
    fn miso_is_optional_and_its_ports_are_offered_without_a_node_toggle() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(SpiDecoder::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node_id];
        let builder = SpiDecoderBuilder;
        let state = serde_json::to_value(SpiDecoder::state()).unwrap();

        assert!(node.inputs[2].visible);
        assert!(node.outputs[1].visible);
        assert!(!builder.input_required(&node.inputs[2], &state));
        assert_eq!(
            builder.output_port(&node.outputs[1], &state, PortKind::of::<Word>()),
            Some("miso_words".to_owned())
        );
    }
}
