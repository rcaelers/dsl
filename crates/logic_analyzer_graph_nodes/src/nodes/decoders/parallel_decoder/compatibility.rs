//! Saved-graph compatibility definition for the former `Binary Decoder`.

use egui::Color32;

use node_graph::{InputDef, NodeBadge, NodeDef, NodePanelDef, OutputDef, PanelSection};

use super::definition::{ParallelDecoder, ParallelDecoderState};
use crate::sockets::{COLOR_DECODERS, Signal, Words};

pub(crate) struct BinaryDecoder;

impl NodeDef for BinaryDecoder {
    type State = ParallelDecoderState;

    fn name() -> &'static str {
        "Binary Decoder"
    }

    fn category() -> &'static str {
        "Decoders"
    }

    fn add_menu_visible() -> bool {
        false
    }

    fn color() -> Color32 {
        COLOR_DECODERS
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Signal>("Clock").stable_id("strobe"),
            InputDef::new::<Signal>("D").stable_id("data").variadic(32),
            InputDef::new::<Signal>("CS").stable_id("cs"),
            InputDef::new::<Signal>("Enable").stable_id("enable"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Words>("Words").stable_id("words")]
    }

    fn state() -> Self::State {
        let mut state = ParallelDecoder::state();
        state.word_size.max = 8;
        state
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        ParallelDecoder::panel()
    }

    fn panels() -> Vec<NodePanelDef<Self::State>> {
        ParallelDecoder::panels()
    }

    fn badge(_state: &Self::State) -> Option<NodeBadge> {
        Some(NodeBadge::warning(
            "Legacy Binary Decoder; replace it with Parallel Decoder",
        ))
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn compatibility_node_stays_loadable_but_is_hidden_from_the_add_menu() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(BinaryDecoder::name(), egui::Pos2::ZERO)
            .unwrap();
        let data = &widget.graph().nodes[&node_id].inputs[1];

        assert!(!BinaryDecoder::add_menu_visible());
        assert!(BinaryDecoder::badge(&BinaryDecoder::state()).is_some());
        assert_eq!(data.variadic.as_ref().map(|info| info.max), Some(32));
    }
}
