//! `Packet Framer` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InputDef, IntValue, NodeBadge, NodeDef, NodePanelDef, OutputDef, PanelSection,
    PropDef, Socket, StringValue,
};

use crate::sockets::{COLOR_LOGIC, ProtocolPackets, Signal, Trigger, Words};

const MAX_PACKET_WORDS: i32 = 65_536;
const MAXIMUM_GAP_US: i32 = 2_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PacketFramerState {
    pub(crate) words_per_packet: IntValue,
    pub(crate) delimiter: StringValue,
    pub(crate) delimiter_policy: EnumValue,
    pub(crate) maximum_gap_us: IntValue,
    pub(crate) gate_polarity: EnumValue,
    pub(crate) maximum_words: IntValue,
}

pub(crate) struct PacketFramer;

impl NodeDef for PacketFramer {
    type State = PacketFramerState;

    fn name() -> &'static str {
        "Packet Framer"
    }

    fn category() -> &'static str {
        "Logic"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Words>("Words").stable_id("words"),
            InputDef::new::<Trigger>("Boundary").stable_id("boundary"),
            InputDef::new::<Signal>("Gate").stable_id("gate"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<ProtocolPackets>("Packets").stable_id("packets")]
    }

    fn state() -> Self::State {
        PacketFramerState {
            words_per_packet: IntValue::new(4, 0, MAX_PACKET_WORDS),
            delimiter: StringValue::new(""),
            delimiter_policy: EnumValue::new(0, &["Include", "Exclude"]),
            maximum_gap_us: IntValue::new(0, 0, MAXIMUM_GAP_US),
            gate_polarity: EnumValue::new(0, &["Active high", "Active low"]),
            maximum_words: IntValue::new(4_096, 1, MAX_PACKET_WORDS),
        }
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Framing",
            vec![
                PropDef::control(
                    "words_per_packet",
                    "Words per packet (0 disables)",
                    |state| &mut state.words_per_packet,
                ),
                PropDef::control("delimiter", "Delimiter hex (empty disables)", |state| {
                    &mut state.delimiter
                }),
                PropDef::control("delimiter_policy", "Delimiter", |state| {
                    &mut state.delimiter_policy
                }),
                PropDef::control("maximum_gap_us", "Maximum gap µs (0 disables)", |state| {
                    &mut state.maximum_gap_us
                }),
                PropDef::control("gate_polarity", "Gate polarity", |state| {
                    &mut state.gate_polarity
                }),
                PropDef::control("maximum_words", "Hard word limit", |state| {
                    &mut state.maximum_words
                }),
            ],
        )]
    }

    fn panels() -> Vec<NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        state.words_per_packet.min = 0;
        state.words_per_packet.max = MAX_PACKET_WORDS;
        state.words_per_packet.value = state.words_per_packet.value.clamp(0, MAX_PACKET_WORDS);
        state.maximum_gap_us.min = 0;
        state.maximum_gap_us.max = MAXIMUM_GAP_US;
        state.maximum_gap_us.value = state.maximum_gap_us.value.clamp(0, MAXIMUM_GAP_US);
        state.maximum_words.min = 1;
        state.maximum_words.max = MAX_PACKET_WORDS;
        state.maximum_words.value = state
            .maximum_words
            .value
            .clamp(state.words_per_packet.value.max(1), MAX_PACKET_WORDS);
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        let delimiter = state.delimiter.value.trim();
        if !delimiter.is_empty() && super::super::word_value::parse_hex_u64(delimiter).is_err() {
            Some(NodeBadge::error("Invalid hexadecimal delimiter"))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn framing_controls_do_not_hide_optional_boundary_sockets() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(PacketFramer::name(), egui::Pos2::ZERO)
            .unwrap();

        assert_eq!(
            widget.graph().nodes[&node]
                .inputs
                .iter()
                .map(|input| (input.name.as_str(), input.visible))
                .collect::<Vec<_>>(),
            [("Words", true), ("Boundary", true), ("Gate", true)]
        );
    }

    #[test]
    fn hard_limit_is_never_smaller_than_the_fixed_packet_size() {
        let mut state = PacketFramer::state();
        state.words_per_packet.value = 100;
        state.maximum_words.value = 10;

        PacketFramer::on_update(&mut state, &mut [], &mut []);

        assert_eq!(state.maximum_words.value, 100);
    }

    #[test]
    fn invalid_nonempty_delimiter_is_visible() {
        let mut state = PacketFramer::state();
        state.delimiter.value = "not hex".to_owned();

        assert!(PacketFramer::badge(&state).is_some());
        state.delimiter.value.clear();
        assert!(PacketFramer::badge(&state).is_none());
    }
}
