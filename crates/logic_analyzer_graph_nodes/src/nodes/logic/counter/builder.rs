//! Runtime builder for `Counter`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_derived::{NumberSample, Trigger};
use signal_runtime::ProcessNode;
use signal_transforms::trigger_counter::TriggerCounter;

#[derive(Default)]
pub(crate) struct CounterBuilder;

impl GraphNodeSemantics for CounterBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<NumberSample>()]
    }
    fn input_port(&self, _: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        Some("trigger".into())
    }
    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some("count".into())
    }
}

impl RuntimeMaterializer for CounterBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::CounterState = parse_state(state)?;
        Ok(Box::new(
            TriggerCounter::new(state.start.value as i64, state.step.value as i64).with_name(name),
        ))
    }
}
