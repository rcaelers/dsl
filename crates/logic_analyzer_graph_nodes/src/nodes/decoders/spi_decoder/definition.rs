//! `SPI Decoder` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    BoolValue, EnumValue, InputDef, IntValue, NodeBadge, NodeDef, NodePanelDef, OutputDef,
    PanelMetadata, PanelSection, PropDef, PropertyPanelPresentation, Socket,
};

use crate::nodes::registry::{COLOR_DECODERS, Signal, Words};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpiDecoderState {
    #[serde(flatten)]
    pub(crate) metadata: SpiDecoderMetadata,
    #[serde(default = "super::super::display_format::default_display_format")]
    pub(crate) display_format: EnumValue,
    pub(crate) word_size: IntValue,
    pub(crate) cpol: EnumValue,
    pub(crate) cpha: EnumValue,
    pub(crate) bit_order: EnumValue,
    pub(crate) cs_polarity: EnumValue,
    pub(crate) has_miso: BoolValue,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SpiDecoderMetadata {
    #[serde(default)]
    schema_version: u8,
    #[serde(skip)]
    compatibility_warning: Option<String>,
}

impl SpiDecoderMetadata {
    pub(crate) fn current() -> Self {
        Self {
            schema_version: 1,
            compatibility_warning: None,
        }
    }
}

pub(crate) struct SpiDecoder;
impl NodeDef for SpiDecoder {
    type State = SpiDecoderState;

    fn name() -> &'static str {
        "SPI Decoder"
    }
    fn category() -> &'static str {
        "Decoders"
    }
    fn color() -> Color32 {
        COLOR_DECODERS
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Signal>("CLK"),
            InputDef::new::<Signal>("MOSI"),
            InputDef::new::<Signal>("MISO"),
            InputDef::new::<Signal>("CS#"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![
            OutputDef::new::<Words>("MOSI Words"),
            OutputDef::new::<Words>("MISO Words"),
            OutputDef::new::<Words>("MOSI Bits").editor_visible(false),
            OutputDef::new::<Words>("MOSI Data").editor_visible(false),
            OutputDef::new::<Words>("MISO Bits").editor_visible(false),
            OutputDef::new::<Words>("MISO Data").editor_visible(false),
        ]
    }

    fn state() -> Self::State {
        SpiDecoderState {
            metadata: SpiDecoderMetadata::current(),
            display_format: super::super::display_format::default_display_format(),
            word_size: IntValue::new(8, 1, 64),
            cpol: EnumValue::new(0, &["0", "1"]),
            cpha: EnumValue::new(0, &["0", "1"]),
            bit_order: EnumValue::new(0, &["MSB first", "LSB first"]),
            cs_polarity: EnumValue::new(0, &["Active low", "Active high", "Disabled"]),
            has_miso: BoolValue::new(true),
        }
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Options",
            vec![
                PropDef::control("word_size", "Word size", |state| &mut state.word_size),
                PropDef::control("cpol", "CPOL", |state| &mut state.cpol),
                PropDef::control("cpha", "CPHA", |state| &mut state.cpha),
                PropDef::control("bit_order", "Bit order", |state| &mut state.bit_order),
                PropDef::control("cs_polarity", "CS# polarity", |state| {
                    &mut state.cs_polarity
                }),
                PropDef::control("has_miso", "Has MISO", |state| &mut state.has_miso),
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
                            |state: &mut SpiDecoderState| &mut state.display_format,
                        )],
                    )],
                ),
            )
            .metadata(PanelMetadata::default().preferred_height(130.0)),
        ]
    }

    fn on_update(state: &mut Self::State, inputs: &mut [Socket], outputs: &mut [Socket]) {
        if state.metadata.schema_version == 0 {
            for (legacy, bits, data) in [(0, 2, 3), (1, 4, 5)] {
                let was_in_view = outputs
                    .get(legacy)
                    .and_then(|output| output.extensions.get("show_in_view"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if was_in_view {
                    if let Some(output) = outputs.get_mut(bits) {
                        output
                            .extensions
                            .insert("show_in_view".to_owned(), serde_json::Value::Bool(true));
                    }
                    if let Some(output) = outputs.get_mut(data) {
                        output
                            .extensions
                            .insert("show_in_view".to_owned(), serde_json::Value::Bool(true));
                    }
                    if let Some(output) = outputs.get_mut(legacy) {
                        output.extensions.remove("show_in_view");
                    }
                }
            }
            state.metadata.schema_version = 1;
            state.metadata.compatibility_warning = Some(
                "Upgraded SPI viewer outputs to Bits/Data; existing explicit Words connections were preserved"
                    .to_owned(),
            );
        }
        // The node editor exposes the connectable word outputs. Bits/Data
        // remain available to the generic View panel and compiler through
        // definition-owned presentation metadata.
        if let Some(miso_words) = outputs.get_mut(1) {
            miso_words.visible = state.has_miso.value;
        }
        if let Some(miso) = inputs.get_mut(2) {
            miso.visible = state.has_miso.value;
        }
        for index in [4, 5] {
            if let Some(miso_output) = outputs.get_mut(index) {
                miso_output.visible = state.has_miso.value;
            }
        }
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        state
            .metadata
            .compatibility_warning
            .as_ref()
            .map(NodeBadge::warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_state_without_display_format_defaults_to_hex() {
        let mut value = serde_json::to_value(SpiDecoder::state()).unwrap();
        value.as_object_mut().unwrap().remove("display_format");

        let state: SpiDecoderState = serde_json::from_value(value).unwrap();
        assert_eq!(state.display_format.selected(), "Hex");
    }

    #[test]
    fn node_editor_shows_only_connectable_word_outputs() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(SpiDecoder::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node_id];

        assert!(node.outputs[0].editor_visible);
        assert!(node.outputs[1].editor_visible);
        for output in &node.outputs[2..] {
            assert!(!output.editor_visible);
        }
    }

    #[test]
    fn legacy_view_selection_migrates_to_bits_and_data_with_a_warning() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(SpiDecoder::name(), egui::Pos2::ZERO)
            .unwrap();
        let mut legacy = serde_json::to_value(SpiDecoder::state()).unwrap();
        legacy.as_object_mut().unwrap().remove("schema_version");
        let mut graph = widget.graph().clone();
        let saved = graph.nodes.get_mut(&node).unwrap();
        saved.outputs.truncate(2);
        saved.outputs[0]
            .extensions
            .insert("show_in_view".to_owned(), serde_json::json!(true));
        saved.state = legacy;

        widget.set_graph(graph);

        let node = &widget.graph().nodes[&node];
        assert!(!node.outputs[0].extensions.contains_key("show_in_view"));
        assert_eq!(
            node.outputs[2].extensions.get("show_in_view"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            node.outputs[3].extensions.get("show_in_view"),
            Some(&serde_json::json!(true))
        );
        assert!(
            node.badge
                .as_ref()
                .is_some_and(|badge| badge.text.contains("Bits/Data"))
        );
    }
}
