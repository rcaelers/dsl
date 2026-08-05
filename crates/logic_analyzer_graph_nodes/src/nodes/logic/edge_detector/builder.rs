//! Runtime builder for `Edge Detector`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::RuntimeBuilder;
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::logic::edge_detector::{EdgeDetector, EdgeMode};
use node_graph::api::Socket;
use signal_capture::Sample;
use signal_derived::Trigger;
use signal_runtime::ProcessNode;

#[derive(Default)]
pub(crate) struct EdgeDetectorBuilder;

impl RuntimeBuilder for EdgeDetectorBuilder {
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Trigger>()]
    }

    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        Some("signal".to_owned())
    }

    fn output_port(&self, _socket: &Socket, _: &Value, _: PortKind) -> Option<String> {
        Some("trigger".to_owned())
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::EdgeDetectorState = parse_state(state)?;
        let mode = match state.edge.selected() {
            "Falling" => EdgeMode::Falling,
            "Both" => EdgeMode::Both,
            _ => EdgeMode::Rising,
        };
        Ok(Box::new(
            EdgeDetector::new(
                mode,
                (state.debounce_us.value.max(0) as u64).saturating_mul(1_000),
                (state.minimum_pulse_width_us.value.max(0) as u64).saturating_mul(1_000),
            )
            .with_name(name),
        ))
    }
}
