//! Runtime builder for `File Writer`.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeCapabilityOverride, GraphNodeSemantics, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use node_graph_document::SocketReference;
use signal_derived::{TextSample, Word};
use signal_runtime::{ProcessNode, ProcessNodeConstruction};
use signal_sinks::OutputOrigin;
use signal_sinks::binary_file_writer::{
    BinaryFileWriterConfig, BinaryFileWriterFactory, WriteWidth, unavailable_writer_factory,
};

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

pub(crate) fn capability_override(
    writer_factory: Arc<dyn BinaryFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    GraphNodeCapabilityOverride::capabilities("org.logicconduit.graph-node.sinks.file-writer/v1")
        .with_materializer(Box::new(FileWriterBuilder::with_writer_factory(
            writer_factory,
        )))
}

impl GraphNodeSemantics for FileWriterBuilder {
    fn is_sink(&self) -> bool {
        true
    }
    fn accepted_kinds(&self, socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        match socket.definition_index() {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<TextSample>()],
            _ => vec![],
        }
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        match socket.definition_index() {
            0 => Some("data".into()),
            1 => Some("filename".into()),
            _ => None,
        }
    }
    fn output_port(&self, _: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn input_required(&self, socket: SocketReference<'_>, state: &Value) -> bool {
        match socket.definition_index() {
            // The Filename input can stay unconnected when the node's own
            // static filename (save-dialog prop) is set.
            1 => parse_state::<super::definition::FileWriterState>(state)
                .map(|state| state.filename.value.trim().is_empty())
                .unwrap_or(true),
            _ => true,
        }
    }
}

impl RuntimeMaterializer for FileWriterBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::FileWriterState =
            parse_state(state).map_err(|error| error.to_string())?;
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
        let source = resolved
            .get(0, 0)
            .ok_or_else(|| "File Writer data input is not connected".to_owned())?;
        self.writer_factory
            .create(
                name,
                BinaryFileWriterConfig::new(width, state.index_csv.value, static_filename),
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
        Box::new(FileWriterBuilder::default()),
        Box::new(FileWriterBuilder::with_writer_factory(factory)),
    )
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityCapabilityRegistration::new(
        "org.logicconduit.graph-node.sinks.file-writer/v1",
        platform_parity_capabilities,
    )
}
