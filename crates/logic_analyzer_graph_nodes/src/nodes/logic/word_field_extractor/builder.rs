//! Runtime builder for `Word Field Extractor`.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::logic::word_field_extractor::WordFieldExtractor;
use node_graph::api::Socket;
use signal_processing::{ProcessNode, Word};

#[derive(Default)]
pub(crate) struct WordFieldExtractorBuilder;

impl RuntimeBuilder for WordFieldExtractorBuilder {
    fn execution_state(&self, state: &Value) -> Value {
        crate::presentation::without_display_format(state)
    }

    fn word_display_format(&self, socket: &Socket, state: &Value) -> Option<String> {
        (socket.def_index == 0)
            .then(|| parse_state::<super::definition::WordFieldExtractorState>(state).ok())
            .flatten()
            .map(|state| state.display_format.selected().to_owned())
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn input_port(
        &self,
        socket: &Socket,
        _member_index: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        (socket.def_index == 0).then(|| "words".to_owned())
    }

    fn output_port(&self, socket: &Socket, _state: &Value, _kind: PortKind) -> Option<String> {
        (socket.def_index == 0).then(|| "field".to_owned())
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::WordFieldExtractorState = parse_state(state)?;
        let first_bit = usize::try_from(state.first_bit.value.max(0))
            .map_err(|_| "start bit is outside the supported range")?;
        let bit_count = usize::try_from(state.bit_count.value.max(1))
            .map_err(|_| "field width is outside the supported range")?;
        Ok(Box::new(
            WordFieldExtractor::new(first_bit, bit_count).with_name(name),
        ))
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;

    use super::super::definition::WordFieldExtractor as WordFieldExtractorDef;
    use super::*;

    #[test]
    fn display_format_is_attached_to_the_word_output() {
        let builder = WordFieldExtractorBuilder;
        let mut state = WordFieldExtractorDef::state();
        state.display_format.select("Binary");
        let state = serde_json::to_value(state).unwrap();
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(WordFieldExtractorDef::name(), egui::Pos2::ZERO)
            .unwrap();
        let output = &widget.graph().nodes[&node].outputs[0];

        assert_eq!(
            builder.word_display_format(output, &state).as_deref(),
            Some("Binary")
        );
    }
}
