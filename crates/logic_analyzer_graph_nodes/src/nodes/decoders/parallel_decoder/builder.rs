//! Runtime builder for `Parallel Decoder` and its saved `Binary Decoder` alias.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    DecoderTableColumnDescriptor, NodeBuildContext, PortKind, ResolvedInputs,
    RetainedWordSamplingSource, SamplingOverlayDescriptor, parse_state,
};
use logic_analyzer_processing::nodes::decoders::parallel_decoder::{
    ParallelDecoder as ProcessingParallelDecoder, ParallelInputStrategy, StrobeMode,
};
use logic_analyzer_processing::types::{CsPolarity, Endianness};
use node_graph::api::Socket;
use signal_processing::{Sample, SampleBlock, Word};
use signal_runtime::ProcessNode;

#[derive(Default)]
pub(crate) struct ParallelDecoderBuilder;

impl ParallelDecoderBuilder {
    fn parsed(state: &Value) -> Result<super::definition::ParallelDecoderState, String> {
        parse_state(state)
    }

    fn cs_polarity(state: &super::definition::ParallelDecoderState) -> CsPolarity {
        match state.cs_polarity.selected() {
            "Active low" => CsPolarity::ActiveLow,
            "Active high" => CsPolarity::ActiveHigh,
            _ => CsPolarity::Disabled,
        }
    }

    fn cycles_per_word(
        state: &super::definition::ParallelDecoderState,
        data_bits: usize,
    ) -> Result<usize, String> {
        let cycles = state.word_size.value.clamp(1, 64) as usize;
        if cycles.saturating_mul(data_bits) > u64::BITS as usize {
            return Err(format!(
                "{cycles} cycles of {data_bits} data bits require {} bits; parallel words are limited to {} bits",
                cycles * data_bits,
                u64::BITS
            ));
        }
        Ok(cycles)
    }
}

impl RuntimeBuilder for ParallelDecoderBuilder {
    fn execution_state(&self, state: &Value) -> Value {
        crate::presentation::without_display_format(state)
    }

    fn decoder_table_column(
        &self,
        socket: &Socket,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        super::presentation::parallel_table_column(socket.def_index)
    }

    fn sampling_overlay(&self, state: &Value) -> Option<SamplingOverlayDescriptor> {
        let state = Self::parsed(state).ok()?;
        match state.sample_on.selected() {
            "Rising (SDR)" | "Falling (SDR)" | "Both (DDR)" => {}
            _ => return None,
        }
        let retained_word_source = match (state.word_size.value, state.sample_on.selected()) {
            (1, "Rising (SDR)") => Some(RetainedWordSamplingSource {
                output: 0,
                clock_high: true,
            }),
            (1, "Falling (SDR)") => Some(RetainedWordSamplingSource {
                output: 0,
                clock_high: false,
            }),
            _ => None,
        };
        Some(SamplingOverlayDescriptor {
            clock_input: 0,
            sampled_input_groups: vec![1],
            retained_word_source,
        })
    }

    fn word_display_format(&self, _socket: &Socket, state: &Value) -> Option<String> {
        Self::parsed(state)
            .ok()
            .map(|state| state.display_format.selected().to_owned())
    }

    fn accepted_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            3 => vec![PortKind::of::<Sample>()],
            _ => vec![PortKind::of::<SampleBlock>()],
        }
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn input_port(
        &self,
        socket: &Socket,
        member_index: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        match socket.def_index {
            0 => Some("strobe".into()),
            1 => Some(format!("d{member_index}")),
            2 => Some("cs".into()),
            3 => Some("enable_signal".into()),
            _ => None,
        }
    }

    fn output_port(&self, _socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        (kind == PortKind::of::<Word>()).then(|| "words".into())
    }

    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        match socket.def_index {
            2 => Self::parsed(state)
                .map(|state| Self::cs_polarity(&state) != CsPolarity::Disabled)
                .unwrap_or(false),
            3 => false,
            _ => true,
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
        let data_bits = resolved.member_count(1);
        if data_bits == 0 {
            return Err("no data channels connected".into());
        }
        let cycles = Self::cycles_per_word(&state, data_bits)?;
        let strobe_mode = match state.sample_on.selected() {
            "Falling (SDR)" => StrobeMode::FallingEdge,
            "Both (DDR)" => StrobeMode::AnyEdge,
            "High level" => StrobeMode::HighLevel,
            "Low level" => StrobeMode::LowLevel,
            _ => StrobeMode::RisingEdge,
        };
        let endianness = if state.endianness.selected() == "Big" {
            Endianness::Big
        } else {
            Endianness::Little
        };
        let mut decoder =
            ProcessingParallelDecoder::new(data_bits, strobe_mode, Self::cs_polarity(&state))
                .with_name(name)
                .with_input_strategy(match state.input_strategy.selected() {
                    "Packed stream" => ParallelInputStrategy::PackedStream,
                    "Indexed" => ParallelInputStrategy::Indexed,
                    _ => ParallelInputStrategy::Auto,
                })
                .with_word_assembly(cycles, endianness)
                .with_work_executor(ctx.work_executor());
        if let Some(points) = ctx.sampling_points(name) {
            decoder = decoder.with_sampling_points(points);
        }
        Ok(Box::new(decoder))
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;

    use super::super::definition::ParallelDecoder;
    use super::*;

    #[test]
    fn sampling_overlay_is_available_only_for_edge_modes() {
        let builder = ParallelDecoderBuilder;
        let mut state = ParallelDecoder::state();
        for (mode, expected) in [
            ("Rising (SDR)", true),
            ("Falling (SDR)", true),
            ("Both (DDR)", true),
            ("High level", false),
            ("Low level", false),
        ] {
            state.sample_on.select(mode);
            let descriptor = builder.sampling_overlay(&serde_json::to_value(&state).unwrap());
            assert_eq!(descriptor.is_some(), expected);
        }
    }

    #[test]
    fn single_cycle_sdr_sampling_reuses_the_retained_word_lane() {
        let builder = ParallelDecoderBuilder;
        let mut state = ParallelDecoder::state();
        for (mode, expected_clock) in [("Rising (SDR)", true), ("Falling (SDR)", false)] {
            state.sample_on.select(mode);
            let descriptor = builder
                .sampling_overlay(&serde_json::to_value(&state).unwrap())
                .unwrap();
            assert_eq!(
                descriptor.retained_word_source,
                Some(RetainedWordSamplingSource {
                    output: 0,
                    clock_high: expected_clock,
                })
            );
        }

        state.sample_on.select("Both (DDR)");
        assert!(
            builder
                .sampling_overlay(&serde_json::to_value(&state).unwrap())
                .unwrap()
                .retained_word_source
                .is_none()
        );
        state.word_size.value = 2;
        state.sample_on.select("Rising (SDR)");
        assert!(
            builder
                .sampling_overlay(&serde_json::to_value(&state).unwrap())
                .unwrap()
                .retained_word_source
                .is_none()
        );
    }

    #[test]
    fn word_assembly_rejects_values_wider_than_the_runtime_word() {
        let mut state = ParallelDecoder::state();
        state.word_size.value = 9;

        assert!(ParallelDecoderBuilder::cycles_per_word(&state, 8).is_err());
        state.word_size.value = 8;
        assert_eq!(ParallelDecoderBuilder::cycles_per_word(&state, 8), Ok(8));
    }
}
