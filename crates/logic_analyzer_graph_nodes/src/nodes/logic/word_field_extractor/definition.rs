//! `Word Field Extractor` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InputDef, IntValue, NodeDef, NodePanelDef, OutputDef, PanelMetadata, PanelSection,
    PropDef, PropertyPanelPresentation, Socket,
};

use crate::sockets::{COLOR_LOGIC, Words};

const MAX_FIELD_BITS: i32 = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WordFieldExtractorState {
    pub(crate) first_bit: IntValue,
    pub(crate) bit_count: IntValue,
    #[serde(default = "crate::presentation::default_word_display_format")]
    pub(crate) display_format: EnumValue,
}

pub(crate) struct WordFieldExtractor;

impl NodeDef for WordFieldExtractor {
    type State = WordFieldExtractorState;

    fn name() -> &'static str {
        "Word Field Extractor"
    }

    fn category() -> &'static str {
        "Logic"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<Words>("Words").stable_id("words")]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Words>("Field").stable_id("field")]
    }

    fn state() -> Self::State {
        WordFieldExtractorState {
            first_bit: IntValue::new(0, 0, MAX_FIELD_BITS - 1),
            bit_count: IntValue::new(8, 1, MAX_FIELD_BITS),
            display_format: crate::presentation::default_word_display_format(),
        }
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Field",
            vec![
                PropDef::control("first_bit", "Start bit", |state| &mut state.first_bit),
                PropDef::control("bit_count", "Width", |state| &mut state.bit_count),
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
                            |state: &mut WordFieldExtractorState| &mut state.display_format,
                        )],
                    )],
                ),
            )
            .metadata(PanelMetadata::default().preferred_height(130.0)),
        ]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        state.first_bit.min = 0;
        state.first_bit.max = MAX_FIELD_BITS - 1;
        state.first_bit.value = state.first_bit.value.clamp(0, MAX_FIELD_BITS - 1);
        state.bit_count.min = 1;
        state.bit_count.max = MAX_FIELD_BITS;
        state.bit_count.value = state.bit_count.value.clamp(1, MAX_FIELD_BITS);
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn socket_names_are_owned_by_the_definition() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(WordFieldExtractor::name(), egui::Pos2::ZERO)
            .unwrap();
        let output = &widget.graph().nodes[&node].outputs[0];

        assert_eq!(output.name, "Field");
        assert_eq!(output.schema_id, "field");
        assert!(WordFieldExtractor::props().is_empty());
    }

    #[test]
    fn restored_ranges_are_normalized_to_supported_limits() {
        let mut state = WordFieldExtractor::state();
        state.first_bit.value = -1;
        state.bit_count.value = 0;

        WordFieldExtractor::on_update(&mut state, &mut [], &mut []);

        assert_eq!(state.first_bit.value, 0);
        assert_eq!(state.bit_count.value, 1);
        assert_eq!(state.bit_count.max, MAX_FIELD_BITS);
    }
}
