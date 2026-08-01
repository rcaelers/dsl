//! Runtime builder for `File Writer`.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::ProcessNodeConstruction;
use logic_analyzer_processing::nodes::sinks::binary_file_writer::{
    BinaryFileWriterConfig, BinaryFileWriterFactory, WriteWidth, unavailable_writer_factory,
};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, TextSample, Word};

pub(crate) struct FileWriterBuilder {
    writer_factory: Arc<dyn BinaryFileWriterFactory>,
}

impl Default for FileWriterBuilder {
    fn default() -> Self {
        Self {
            writer_factory: unavailable_writer_factory(),
        }
    }
}

impl FileWriterBuilder {
    pub(crate) fn with_writer_factory(writer_factory: Arc<dyn BinaryFileWriterFactory>) -> Self {
        Self { writer_factory }
    }
}

pub(crate) fn runtime_builder_override(
    writer_factory: Arc<dyn BinaryFileWriterFactory>,
) -> logic_analyzer_graph_api::node::RuntimeBuilderOverride {
    logic_analyzer_graph_api::node::RuntimeBuilderOverride::new(
        "org.logicconduit.graph-node.sinks.file-writer/v1",
        Box::new(FileWriterBuilder::with_writer_factory(writer_factory)),
    )
}

impl RuntimeBuilder for FileWriterBuilder {
    fn is_sink(&self) -> bool {
        true
    }
    fn accepted_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<TextSample>()],
            _ => vec![],
        }
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("data".into()),
            1 => Some("filename".into()),
            _ => None,
        }
    }
    fn output_port(&self, _: &Socket, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        match socket.def_index {
            // The Filename input can stay unconnected when the node's own
            // static filename (save-dialog prop) is set.
            1 => parse_state::<super::definition::FileWriterState>(state)
                .map(|state| state.filename.value.trim().is_empty())
                .unwrap_or(true),
            _ => true,
        }
    }
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::FileWriterState = parse_state(state)?;
        let width = match state.write_width.selected() {
            "U16 LE" => WriteWidth::U16Le,
            "U32 LE" => WriteWidth::U32Le,
            _ => WriteWidth::U8,
        };
        // Static fallback only when nothing is wired into Filename — a
        // connected stream always wins.
        let static_filename = state.filename.value.trim();
        let static_filename = (resolved.kind(1).is_none() && !static_filename.is_empty())
            .then(|| static_filename.to_owned());
        self.writer_factory
            .create(
                name,
                BinaryFileWriterConfig::new(width, state.index_csv.value, static_filename),
            )
            .map(ProcessNodeConstruction::into_process)
    }
}

#[cfg(test)]
fn platform_parity_builder() -> Box<dyn RuntimeBuilder> {
    Box::new(FileWriterBuilder::with_writer_factory(Arc::new(
        crate::nodes::test_support::TestWriterFactory,
    )))
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityBuilderRegistration::new(
        "org.logicconduit.graph-node.sinks.file-writer/v1",
        platform_parity_builder,
    )
}
