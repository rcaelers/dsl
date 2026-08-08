//! Runtime builder for `Event Control`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeSemantics, RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_derived::TimestampEvent;
use signal_runtime::ProcessNode;
use signal_transforms::event_control::EventControl;

#[derive(Default)]
pub(crate) struct EventControlBuilder;

impl GraphNodeSemantics for EventControlBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TimestampEvent>()]
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TimestampEvent>()]
    }

    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        match socket.definition_index() {
            0 => Some("events".to_owned()),
            1 => Some("rearm".to_owned()),
            _ => None,
        }
    }

    fn output_port(&self, _socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        Some("events".to_owned())
    }

    fn input_required(&self, socket: SocketReference<'_>, _state: &Value) -> bool {
        socket.definition_index() == 0
    }
}

impl RuntimeMaterializer for EventControlBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
        let state: super::definition::EventControlState = parse_state(state)?;
        Ok(Box::new(
            EventControl::new(
                (state.delay_us.value.max(0) as u64).saturating_mul(1_000),
                (state.holdoff_us.value.max(0) as u64).saturating_mul(1_000),
                resolved.kind(1).is_some(),
            )
            .with_name(name),
        ))
    }
}

#[cfg(test)]
mod builder_tests {
    use node_graph::NodeDef;
    use node_graph_document::SocketDirection;

    use super::super::definition::EventControl;
    use super::*;

    #[test]
    fn event_input_is_required_but_rearm_is_optional() {
        let builder = EventControlBuilder;
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node_id = widget
            .add_node_at(EventControl::name(), egui::Pos2::ZERO)
            .unwrap();
        let node = &widget.graph().nodes[&node_id];

        assert!(builder.input_required(
            node.inputs[0].reference(SocketDirection::Input, 0),
            &node.state
        ));
        assert!(!builder.input_required(
            node.inputs[1].reference(SocketDirection::Input, 0),
            &node.state
        ));
    }
}
