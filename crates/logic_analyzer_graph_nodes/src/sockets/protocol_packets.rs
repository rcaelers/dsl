use egui::Color32;

use node_graph::{SocketDef, SocketShape};

/// Structured protocol events exchanged between independently authored decoders.
pub(crate) struct ProtocolPackets;

impl SocketDef for ProtocolPackets {
    type Value = ();

    fn type_name() -> &'static str {
        "Protocol Packet"
    }

    fn color() -> Color32 {
        Color32::from_rgb(175, 120, 205)
    }

    fn shape() -> SocketShape {
        SocketShape::Diamond
    }
}
