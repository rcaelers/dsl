//! Runtime builder for `Word Field Extractor`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodePresentation, GraphNodeSemantics, RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_derived::Word;
use signal_runtime::ProcessNode;
use signal_transforms::word_field_extractor::WordFieldExtractor;

#[derive(Default)]
pub(crate) struct WordFieldExtractorBuilder;

impl GraphNodeSemantics for WordFieldExtractorBuilder {
    fn execution_state(&self, state: &Value) -> Value {
        crate::presentation::without_display_format(state)
    }

    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn input_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        (socket.definition_index() == 0).then(|| "words".to_owned())
    }

    fn output_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        (socket.definition_index() == 0).then(|| "field".to_owned())
    }
}

impl RuntimeMaterializer for WordFieldExtractorBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
        let state: super::definition::WordFieldExtractorState = parse_state(state)?;
        let first_bit = usize::try_from(state.first_bit.value.max(0)).map_err(|_| {
            RuntimeMaterializationError::configuration("start bit is outside the supported range")
        })?;
        let bit_count = usize::try_from(state.bit_count.value.max(1)).map_err(|_| {
            RuntimeMaterializationError::configuration("field width is outside the supported range")
        })?;
        Ok(Box::new(
            WordFieldExtractor::new(first_bit, bit_count).with_name(name),
        ))
    }
}

impl GraphNodePresentation for WordFieldExtractorBuilder {
    fn word_display_format(&self, socket: SocketReference<'_>, state: &Value) -> Option<String> {
        (socket.definition_index() == 0)
            .then(|| parse_state::<super::definition::WordFieldExtractorState>(state).ok())
            .flatten()
            .map(|state| state.display_format.selected().to_owned())
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;
    use node_graph_document::SocketDirection;

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
            builder
                .word_display_format(output.reference(SocketDirection::Output, 0), &state)
                .as_deref(),
            Some("Binary")
        );
    }
}
