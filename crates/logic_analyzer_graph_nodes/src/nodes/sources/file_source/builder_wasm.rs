//! Browser runtime builder for `DSL File Source`.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs, parse_state,
};
use logic_analyzer_processing::nodes::sources::dsl_file::{DslFileSourceConfig, create_source};
use node_graph::api::Socket;
use signal_processing::{DerivedDataRetention, ProcessNode, Sample, SampleBlock};

#[derive(Default)]
pub(crate) struct FileSourceBuilder;

impl RuntimeBuilder for FileSourceBuilder {
    fn is_source(&self) -> bool {
        true
    }

    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_api::node_support::SourceDataLifecycle> {
        Some(
            logic_analyzer_graph_api::node_support::SourceDataLifecycle::new(
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::File,
                true,
                false,
                false,
            ),
        )
    }

    fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
        DerivedDataRetention::Unlimited
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<SampleBlock>(), PortKind::of::<Sample>()]
    }

    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }

    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        if kind == PortKind::of::<SampleBlock>() {
            Some(format!("block{}", socket.def_index))
        } else if kind == PortKind::of::<Sample>() {
            Some(format!("ch{}", socket.def_index))
        } else {
            None
        }
    }

    fn viewer_channel_origin(&self, socket: &Socket, _state: &Value) -> Option<usize> {
        Some(socket.def_index)
    }

    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        Ok(Some(
            super::super::synthetic_presentation::capture_presentation(
                state.channel_names.iter().cloned(),
            ),
        ))
    }

    fn input_required(&self, _socket: &Socket, _state: &Value) -> bool {
        false
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        create_source(
            name,
            DslFileSourceConfig::new(&state.file.value, state.channel_names.len()),
        )
    }
}
