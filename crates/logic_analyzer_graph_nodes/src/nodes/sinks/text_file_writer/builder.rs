//! Runtime builder for `Text File Writer` (e.g. `TGCK Recorder`'s CSV rows).

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{NodeBuildContext, PortKind, ResolvedInputs};
use logic_analyzer_processing::ProcessNodeConstruction;
use logic_analyzer_processing::nodes::sinks::text_file_writer::{
    TextFileWriterFactory, writer_factory,
};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, TextSample};

pub(crate) struct TextFileWriterBuilder {
    writer_factory: Arc<dyn TextFileWriterFactory>,
}

impl Default for TextFileWriterBuilder {
    fn default() -> Self {
        Self {
            writer_factory: writer_factory(),
        }
    }
}

impl TextFileWriterBuilder {
    #[cfg(test)]
    pub(crate) fn with_writer_factory(writer_factory: Arc<dyn TextFileWriterFactory>) -> Self {
        Self { writer_factory }
    }
}

impl RuntimeBuilder for TextFileWriterBuilder {
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
    fn build(
        &self,
        name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        self.writer_factory
            .create(name)
            .map(ProcessNodeConstruction::into_process)
    }
}

#[cfg(test)]
fn platform_parity_builder() -> Box<dyn RuntimeBuilder> {
    Box::new(TextFileWriterBuilder::with_writer_factory(Arc::new(
        crate::nodes::test_support::TestWriterFactory,
    )))
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityBuilderRegistration::new(
        "org.logicconduit.graph-node.sinks.text-file-writer/v1",
        platform_parity_builder,
    )
}
