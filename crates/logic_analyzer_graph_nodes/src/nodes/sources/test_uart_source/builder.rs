//! Runtime builder for `Test UART Source` — generates a fixed UART byte sequence
//! in-memory. Available on every target (no file/USB access needed).

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, GraphNodePresentation, GraphNodeSemantics, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_generators::synthetic_uart_source::SyntheticUartSource;
use signal_runtime::ProcessNode;

#[derive(Default)]
pub(crate) struct TestUartSourceBuilder;

impl GraphNodeSemantics for TestUartSourceBuilder {
    fn is_source(&self) -> bool {
        true
    }
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>()]
    }
    fn input_port(&self, _: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        (kind == PortKind::of::<Sample>()).then(|| "rx".into())
    }
    fn input_required(&self, _: SocketReference<'_>, _: &Value) -> bool {
        false
    }
}

impl RuntimeMaterializer for TestUartSourceBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::TestUartSourceState = parse_state(state)?;
        let source = SyntheticUartSource::new(
            state.message.value.into_bytes(),
            state.baud_rate.value.max(1) as u64,
        )
        .with_name(name);
        Ok(Box::new(source))
    }
}

impl CaptureSourceFeature for TestUartSourceBuilder {
    fn capture_presentation(&self, _state: &Value) -> Result<Option<CapturePresentation>, String> {
        Ok(Some(CapturePresentation::Channels(vec![(0, "RX".into())])))
    }
}

impl GraphNodePresentation for TestUartSourceBuilder {
    fn viewer_channel_origin(&self, _socket: SocketReference<'_>, _state: &Value) -> Option<usize> {
        Some(0)
    }
}
