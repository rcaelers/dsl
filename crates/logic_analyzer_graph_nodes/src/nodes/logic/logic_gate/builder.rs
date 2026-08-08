//! Runtime builder for `Logic Gate`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeSemantics, RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_runtime::ProcessNode;
use signal_transforms::logic_gate::{GateOp, LogicGate};

#[derive(Default)]
pub(crate) struct LogicGateBuilder;

impl GraphNodeSemantics for LogicGateBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn input_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some(format!("in{}", socket.member_index()))
    }
    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some("out".into())
    }
}

impl RuntimeMaterializer for LogicGateBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
        let state: super::definition::LogicGateState = parse_state(state)?;
        let inputs = resolved.member_count(0);
        if inputs == 0 {
            return Err(RuntimeMaterializationError::unavailable(
                "no inputs connected",
            ));
        }
        let op = match state.op.selected() {
            "NOT" => GateOp::Not,
            "NAND" => GateOp::Nand,
            "OR" => GateOp::Or,
            "NOR" => GateOp::Nor,
            "XOR" => GateOp::Xor,
            "XNOR" => GateOp::Xnor,
            _ => GateOp::And,
        };
        if op == GateOp::Not && inputs != 1 {
            return Err(RuntimeMaterializationError::configuration(
                "NOT takes exactly one input",
            ));
        }
        Ok(Box::new(LogicGate::new(op, inputs).with_name(name)))
    }
}
