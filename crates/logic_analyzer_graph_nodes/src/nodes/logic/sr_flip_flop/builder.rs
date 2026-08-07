//! Runtime builder for `SR Flip-Flop`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_derived::TimestampEvent;
use signal_runtime::ProcessNode;
use signal_transforms::sr_latch::SrLatch;

#[derive(Default)]
pub(crate) struct SrFlipFlopBuilder;

impl GraphNodeSemantics for SrFlipFlopBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TimestampEvent>()]
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        match socket.definition_index() {
            0 => Some("set".into()),
            1 => Some("reset".into()),
            _ => None,
        }
    }
    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some("q".into())
    }
}

impl RuntimeMaterializer for SrFlipFlopBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::SrFlipFlopState = parse_state(state)?;
        Ok(Box::new(SrLatch::new(state.initial.value).with_name(name)))
    }
}
