use egui::Color32;

use node_graph::api::{SocketDef, SocketShape};

/// Decoded word events (`Word` at runtime).
pub(crate) struct Words;

impl SocketDef for Words {
    type Value = u64;

    fn type_name() -> &'static str {
        "Words"
    }

    fn color() -> Color32 {
        Color32::from_rgb(215, 140, 60)
    }

    fn shape() -> SocketShape {
        SocketShape::Diamond
    }
}
