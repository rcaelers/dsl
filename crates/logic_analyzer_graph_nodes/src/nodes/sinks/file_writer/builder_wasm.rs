//! Browser runtime builder for `File Writer`.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::sinks::binary_file_writer::{
    BinaryFileWriterConfig, WriteWidth, create_writer,
};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, TextSample, Word};

#[derive(Default)]
pub(crate) struct FileWriterBuilder;

impl RuntimeBuilder for FileWriterBuilder {
    fn is_sink(&self) -> bool {
        true
    }

    fn accepted_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<TextSample>()],
            _ => Vec::new(),
        }
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn input_port(&self, socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        match socket.def_index {
            0 => Some("data".to_owned()),
            1 => Some("filename".to_owned()),
            _ => None,
        }
    }

    fn output_port(&self, _: &Socket, _: &Value, _: PortKind) -> Option<String> {
        None
    }

    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        match socket.def_index {
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
        let static_filename = state.filename.value.trim();
        let static_filename = (resolved.kind(1).is_none() && !static_filename.is_empty())
            .then(|| static_filename.to_owned());
        create_writer(
            name,
            BinaryFileWriterConfig::new(width, state.index_csv.value, static_filename),
        )
    }
}
