//! Runtime builder for `String Formatter`.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_derived::{NumberSample, TextSample};
use signal_runtime::{ConfigValue, NodeConfig, ProcessNode};
use signal_transforms::text_formatter::TextFormatter;

#[derive(Default)]
pub(crate) struct FormatterBuilder;

impl GraphNodeSemantics for FormatterBuilder {
    fn accepted_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<NumberSample>()]
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TextSample>()]
    }
    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        // First value keeps the historic port name.
        Some(if socket.member_index() == 0 {
            "value".into()
        } else {
            format!("value{}", socket.member_index())
        })
    }
    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        Some("text".into())
    }
}

impl RuntimeMaterializer for FormatterBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::StringFormatterState = parse_state(state)?;
        let values = resolved.member_count(0).max(1);
        Ok(Box::new(
            TextFormatter::with_num_values(state.template.value.clone(), values).with_name(name),
        ))
    }

    fn hot_config(&self, state: &Value) -> Option<NodeConfig> {
        let state: super::definition::StringFormatterState = parse_state(state).ok()?;
        let mut config = NodeConfig::new();
        config.insert(
            "template".into(),
            ConfigValue::Text(state.template.value.clone()),
        );
        Some(config)
    }
}
