//! Native `I2C Decoder` graph-node definition.

use egui::Color32;

use node_graph::api::{InputDef, NodeDef, OutputDef};

use crate::sockets::{COLOR_DECODERS, ProtocolPackets, Signal, Words};

pub(crate) struct I2cDecoder;
impl NodeDef for I2cDecoder {
    type State = ();

    fn name() -> &'static str {
        "I2C Decoder"
    }
    fn category() -> &'static str {
        "Decoders"
    }
    fn color() -> Color32 {
        COLOR_DECODERS
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Signal>("SCL"),
            InputDef::new::<Signal>("SDA"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![
            OutputDef::new::<Words>("Words"),
            OutputDef::new::<ProtocolPackets>("Packets").stable_id("packets"),
        ]
    }

    fn state() -> Self::State {}

    fn panels() -> Vec<node_graph::api::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }
}
