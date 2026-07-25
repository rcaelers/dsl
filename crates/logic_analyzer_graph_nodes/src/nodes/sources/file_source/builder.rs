//! Runtime builder for `DSL File Source`.
//! Native-only: no filesystem in the browser.

use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    CaptureCacheIdentity, CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs,
    parse_state,
};
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSource;
use node_graph::api::Socket;
use signal_processing::{
    DEFAULT_DERIVED_DATA_MAX_ENTRIES, DerivedDataRetention, ProcessNode, Sample, SampleBlock,
};

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
                true,
                true,
            ),
        )
    }
    fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
        DerivedDataRetention::MaxEntries(DEFAULT_DERIVED_DATA_MAX_ENTRIES)
    }
    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![]
    }
    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<Sample>(), PortKind::of::<SampleBlock>()]
    }
    fn input_port(&self, _socket: &Socket, _: usize, _: &Value, _: PortKind) -> Option<String> {
        None
    }
    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        let channel = socket.def_index;
        // The runtime negotiates Sample vs SampleBlock per connection on a
        // single `ch{channel}` port now — both kinds resolve to the same
        // port name here.
        if kind == PortKind::of::<Sample>() || kind == PortKind::of::<SampleBlock>() {
            Some(format!("ch{channel}"))
        } else {
            None
        }
    }
    fn viewer_channel_origin(&self, socket: &Socket, _state: &Value) -> Option<usize> {
        Some(socket.def_index)
    }
    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        let state: super::definition::DslFileSourceState = parse_state(state)?;
        let path = std::path::PathBuf::from(state.file.value);
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        let indexed = DslFileSource::indexed_capture_presentation(&path);
        Ok(Some(CapturePresentation::Indexed {
            identity: indexed.identity,
            factory: indexed.factory,
        }))
    }
    fn capture_cache_identity(
        &self,
        state: &Value,
        _resolved: &ResolvedInputs,
    ) -> CaptureCacheIdentity {
        let Ok(state) = parse_state::<super::definition::DslFileSourceState>(state) else {
            return CaptureCacheIdentity::Dynamic;
        };
        if state.file.value.trim().is_empty() {
            return CaptureCacheIdentity::Dynamic;
        }
        DslFileSource::capture_cache_identity(&state.file.value)
            .map(CaptureCacheIdentity::Stable)
            .unwrap_or(CaptureCacheIdentity::Dynamic)
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
        let source = DslFileSource::new(&state.file.value)
            .map_err(|e| format!("cannot open '{}': {e}", state.file.value))?
            .with_name(name);
        Ok(Box::new(source))
    }
}
