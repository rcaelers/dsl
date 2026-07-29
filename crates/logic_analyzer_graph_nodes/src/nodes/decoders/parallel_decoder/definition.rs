//! `Parallel Decoder` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InputDef, IntValue, NodeDef, NodePanelDef, OutputDef, PanelMetadata, PanelSection,
    PropDef, PropertyPanelPresentation, Socket,
};

use crate::sockets::{COLOR_DECODERS, Signal, Words};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ParallelDecoderState {
    #[serde(default = "super::super::display_format::default_display_format")]
    pub(crate) display_format: EnumValue,
    pub(crate) sample_on: EnumValue,
    #[serde(default = "default_input_strategy")]
    pub(crate) input_strategy: EnumValue,
    pub(crate) word_size: IntValue,
    pub(crate) endianness: EnumValue,
    pub(crate) cs_polarity: EnumValue,
}

pub(crate) fn default_input_strategy() -> EnumValue {
    EnumValue::new(0, &["Auto", "Packed stream", "Indexed"])
}

pub(crate) struct ParallelDecoder;

impl NodeDef for ParallelDecoder {
    type State = ParallelDecoderState;

    fn name() -> &'static str {
        "Parallel Decoder"
    }

    fn category() -> &'static str {
        "Decoders"
    }

    fn color() -> Color32 {
        COLOR_DECODERS
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Signal>("Strobe").stable_id("strobe"),
            InputDef::new::<Signal>("D").stable_id("data").variadic(64),
            InputDef::new::<Signal>("CS").stable_id("cs"),
            InputDef::new::<Signal>("Enable").stable_id("enable"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Words>("Words").stable_id("words")]
    }

    fn state() -> Self::State {
        ParallelDecoderState {
            display_format: super::super::display_format::default_display_format(),
            sample_on: EnumValue::new(
                0,
                &[
                    "Rising (SDR)",
                    "Falling (SDR)",
                    "Both (DDR)",
                    "High level",
                    "Low level",
                ],
            ),
            input_strategy: default_input_strategy(),
            word_size: IntValue::new(1, 1, 64),
            endianness: EnumValue::new(0, &["Little", "Big"]),
            cs_polarity: EnumValue::new(0, &["Disabled", "Active low", "Active high"]),
        }
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Options",
            vec![
                PropDef::control("sample_on", "Sample on", |state| &mut state.sample_on),
                PropDef::control("input_strategy", "Input strategy", |state| {
                    &mut state.input_strategy
                }),
                PropDef::control("word_size", "Cycles per word", |state| &mut state.word_size),
                PropDef::control("endianness", "Cycle order", |state| &mut state.endianness),
                PropDef::control("cs_polarity", "CS polarity", |state| &mut state.cs_polarity),
            ],
        )]
    }

    fn panels() -> Vec<NodePanelDef<Self::State>> {
        vec![
            crate::presentation::viewer_outputs_panel(),
            NodePanelDef::new(
                "presentation",
                "view",
                PropertyPanelPresentation::new(
                    "Presentation",
                    vec![PanelSection::new(
                        "Format",
                        vec![PropDef::control(
                            "display_format",
                            "Data display",
                            |state: &mut ParallelDecoderState| &mut state.display_format,
                        )],
                    )],
                ),
            )
            .metadata(PanelMetadata::default().preferred_height(130.0)),
        ]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        state.word_size.min = 1;
        state.word_size.max = 64;
        state.word_size.value = state.word_size.value.clamp(1, 64);
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn definition_exposes_sixty_four_parallel_data_inputs() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(ParallelDecoder::name(), egui::Pos2::ZERO)
            .unwrap();
        let data = &widget.graph().nodes[&node_id].inputs[1];

        assert_eq!(data.schema_id, "data");
        assert_eq!(data.variadic.as_ref().map(|info| info.max), Some(64));
    }

    #[test]
    fn older_state_without_input_strategy_defaults_to_auto() {
        let mut value = serde_json::to_value(ParallelDecoder::state()).unwrap();
        value.as_object_mut().unwrap().remove("input_strategy");

        let state: ParallelDecoderState = serde_json::from_value(value).unwrap();
        assert_eq!(state.input_strategy.selected(), "Auto");
    }

    #[test]
    fn restored_word_size_uses_the_current_sixty_four_cycle_range() {
        let mut state = ParallelDecoder::state();
        state.word_size.max = 8;

        ParallelDecoder::on_update(&mut state, &mut [], &mut []);

        assert_eq!(state.word_size.max, 64);
    }
}
