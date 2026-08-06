//! Runtime builder for `Event Gate`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_derived::Trigger;
use signal_runtime::ProcessNode;
use signal_transforms::event_gate::{EventGate, GatePolarity};

#[derive(Default)]
pub(crate) struct EventGateBuilder;

impl GraphNodeSemantics for EventGateBuilder {
    fn accepted_kinds(&self, socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        match socket.definition_index() {
            0 => vec![PortKind::of::<Trigger>()],
            1 => vec![PortKind::of::<Sample>()],
            _ => Vec::new(),
        }
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }

    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        match socket.definition_index() {
            0 => Some("events".to_owned()),
            1 => Some("gate".to_owned()),
            _ => None,
        }
    }

    fn output_port(&self, _socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        Some("events".to_owned())
    }
}

impl RuntimeMaterializer for EventGateBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::EventGateState = parse_state(state)?;
        let polarity = if state.polarity.selected() == "Active low" {
            GatePolarity::ActiveLow
        } else {
            GatePolarity::ActiveHigh
        };
        Ok(Box::new(EventGate::new(polarity).with_name(name)))
    }
}
