//! Runtime builder for `Event Control`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::logic::event_control::EventControl;
use node_graph::api::Socket;
use signal_processing::{ProcessNode, Trigger};

#[derive(Default)]
pub(crate) struct EventControlBuilder;

impl RuntimeBuilder for EventControlBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }

    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("events".to_owned()),
            1 => Some("rearm".to_owned()),
            _ => None,
        }
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("events".to_owned())
    }

    fn input_required(&self, socket: &Socket, _state: &Value) -> bool {
        socket.def_index == 0
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
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

        assert!(builder.input_required(&node.inputs[0], &node.state));
        assert!(!builder.input_required(&node.inputs[1], &node.state));
    }
}
