//! Runtime builder for `Edge Detector`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeSemantics, RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_derived::TimestampEvent;
use signal_runtime::ProcessNode;
use signal_transforms::edge_detector::{EdgeDetector, EdgeMode};

#[derive(Default)]
pub(crate) struct EdgeDetectorBuilder;

impl GraphNodeSemantics for EdgeDetectorBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }

    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TimestampEvent>()]
    }

    fn input_port(&self, _socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        Some("signal".to_owned())
    }

    fn output_port(&self, _socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        Some("event".to_owned())
    }
}

impl RuntimeMaterializer for EdgeDetectorBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
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
