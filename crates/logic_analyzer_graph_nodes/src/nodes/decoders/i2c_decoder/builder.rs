use serde_json::Value;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_graph_api::node_support::{
    DecoderTableColumnDescriptor, NodeBuildContext, PortKind, ResolvedInputs,
};
use logic_analyzer_processing::nodes::decoders::i2c_decoder::{I2C_PROTOCOL_ID, I2cDecoder};
use node_graph::api::Socket;
use signal_processing::{ProcessNode, ProtocolPacket, SampleBlock, Word};

#[derive(Default)]
pub(crate) struct I2cDecoderBuilder;

impl RuntimeBuilder for I2cDecoderBuilder {
    fn decoder_table_column(
        &self,
        socket: &Socket,
        _state: &Value,
    ) -> Option<DecoderTableColumnDescriptor> {
        super::presentation::i2c_table_column(socket.def_index)
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<SampleBlock>()]
    }

    fn offered_kinds(&self, socket: &Socket, _state: &Value) -> Vec<PortKind> {
        match socket.def_index {
            0 => vec![PortKind::of::<Word>()],
            1 => vec![PortKind::of_named::<ProtocolPacket>("Protocol Packet")],
            _ => Vec::new(),
        }
    }

    fn offered_connection_contracts(&self, socket: &Socket, _state: &Value) -> Vec<String> {
        if socket.def_index == 1 {
            vec![I2C_PROTOCOL_ID.to_owned()]
        } else {
            Vec::new()
        }
    }

    fn input_port(
        &self,
        socket: &Socket,
        _member_index: usize,
        _state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        if kind != PortKind::of::<SampleBlock>() {
            return None;
        }
        match socket.def_index {
            0 => Some("scl".into()),
            1 => Some("sda".into()),
            _ => None,
        }
    }

    fn output_port(&self, socket: &Socket, _state: &Value, kind: PortKind) -> Option<String> {
        match socket.def_index {
            0 if kind == PortKind::of::<Word>() => Some("words".into()),
            1 if kind == PortKind::of_named::<ProtocolPacket>("Protocol Packet") => {
                Some("packets".into())
            }
            _ => None,
        }
    }

    fn build(
        &self,
        name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        Ok(Box::new(I2cDecoder::new().with_name(name)))
    }
}
