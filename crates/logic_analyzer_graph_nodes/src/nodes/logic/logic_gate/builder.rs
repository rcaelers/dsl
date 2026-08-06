//! Runtime builder for `Logic Gate`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph::api::Socket;
use signal_capture::Sample;
use signal_runtime::ProcessNode;
use signal_transforms::logic_gate::{GateOp, LogicGate};

#[derive(Default)]
pub(crate) struct LogicGateBuilder;

impl GraphNodeSemantics for LogicGateBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn input_port(
        &self,
        _socket: &Socket,
        member_index: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some(format!("in{member_index}"))
    }
    fn output_port(&self, _socket: &Socket, _state: &Value, _kind: PortKind) -> Option<String> {
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
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::LogicGateState = parse_state(state)?;
        let inputs = resolved.member_count(0);
        if inputs == 0 {
            return Err("no inputs connected".into());
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
            return Err("NOT takes exactly one input".into());
        }
        Ok(Box::new(LogicGate::new(op, inputs).with_name(name)))
    }
}
