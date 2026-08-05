//! Runtime builder for `Text File Writer` (e.g. `TGCK Recorder`'s CSV rows).

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeCapabilityOverride, GraphNodeSemantics, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{NodeBuildContext, PortKind, ResolvedInputs};
use logic_analyzer_processing::ProcessNodeConstruction;
use logic_analyzer_processing::nodes::sinks::OutputOrigin;
use logic_analyzer_processing::nodes::sinks::text_file_writer::{
    TextFileWriterFactory, unavailable_writer_factory,
};
use node_graph::api::Socket;
use signal_derived::TextSample;
use signal_runtime::ProcessNode;

pub(crate) struct TextFileWriterBuilder {
    writer_factory: Arc<dyn TextFileWriterFactory>,
}

impl Default for TextFileWriterBuilder {
    fn default() -> Self {
        Self {
            writer_factory: unavailable_writer_factory(),
        }
    }
}

impl TextFileWriterBuilder {
    pub(crate) fn with_writer_factory(writer_factory: Arc<dyn TextFileWriterFactory>) -> Self {
        Self { writer_factory }
    }
}

pub(crate) fn capability_override(
    writer_factory: Arc<dyn TextFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    GraphNodeCapabilityOverride::capabilities(
        "org.logicconduit.graph-node.sinks.text-file-writer/v1",
    )
    .with_materializer(Box::new(TextFileWriterBuilder::with_writer_factory(
        writer_factory,
    )))
}

impl GraphNodeSemantics for TextFileWriterBuilder {
    fn is_sink(&self) -> bool {
        true
    }
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TextSample>()]
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("lines".into()),
            1 => Some("filename".into()),
            _ => None,
        }
    }
    fn output_port(&self, _: &Socket, _: &Value, _: PortKind) -> Option<String> {
        None
    }
}

impl RuntimeMaterializer for TextFileWriterBuilder {
    fn build(
        &self,
        name: &str,
        _state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let source = resolved
            .get(0, 0)
            .ok_or_else(|| "Text File Writer lines input is not connected".to_owned())?;
        self.writer_factory
            .create(
                name,
                OutputOrigin::new(
                    source.source_node_title.clone(),
                    source.source_output_title.clone(),
                ),
            )
            .map(ProcessNodeConstruction::into_process)
    }
}

#[cfg(test)]
fn platform_parity_capabilities() -> crate::nodes::test_support::PlatformParityCapabilities {
    let factory = Arc::new(crate::nodes::test_support::TestWriterFactory);
    crate::nodes::test_support::PlatformParityCapabilities::new(
        Box::new(TextFileWriterBuilder::default()),
        Box::new(TextFileWriterBuilder::with_writer_factory(factory)),
    )
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityCapabilityRegistration::new(
        "org.logicconduit.graph-node.sinks.text-file-writer/v1",
        platform_parity_capabilities,
    )
}
