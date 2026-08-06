//! Runtime builder for `TGCK Recorder` — pure edge/word correlation, no file I/O (see
//! `signal_sinks::tgck_recorder::TgckRecorder`'s doc comment). Its `Rows`/`Filename` outputs need a
//! `Text File Writer` downstream to actually persist anything; available on
//! every target.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{GraphNodeSemantics, RuntimeMaterializer};
use logic_analyzer_graph_capabilities::node_support::{NodeBuildContext, PortKind, ResolvedInputs};
use node_graph_document::SocketReference;
use signal_capture::Sample;
use signal_derived::{TextSample, Word};
use signal_runtime::ProcessNode;
use signal_sinks::tgck_recorder::TgckRecorder;

#[derive(Default)]
pub(crate) struct TgckRecorderBuilder;

impl GraphNodeSemantics for TgckRecorderBuilder {
    fn accepted_kinds(&self, socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        match socket.definition_index() {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of::<Sample>()],
            2 => vec![PortKind::of::<TextSample>()],
            _ => vec![],
        }
    }
    fn offered_kinds(&self, _socket: SocketReference<'_>, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<TextSample>()]
    }
    fn input_port(&self, socket: SocketReference<'_>, _: &Value, _: PortKind) -> Option<String> {
        match socket.definition_index() {
            0 => Some("words".into()),
            1 => Some("tgck".into()),
            2 => Some("filename".into()),
            _ => None,
        }
    }
    fn output_port(
        &self,
        socket: SocketReference<'_>,
        _state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        if kind != PortKind::of::<TextSample>() {
            return None;
        }
        match socket.definition_index() {
            0 => Some("rows".into()),
            1 => Some("filename".into()),
            _ => None,
        }
    }
}

impl RuntimeMaterializer for TgckRecorderBuilder {
    fn build(
        &self,
        name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Ok(Box::new(TgckRecorder::new().with_name(name)))
    }
}
