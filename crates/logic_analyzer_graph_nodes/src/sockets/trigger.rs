use egui::Color32;

use node_graph::{SocketDef, SocketShape};

/// Instantaneous events with no payload beyond time (`TimestampEvent` at runtime).
pub(crate) struct Trigger;

impl SocketDef for Trigger {
    type Value = ();

    fn type_name() -> &'static str {
        "Trigger"
    }

    fn color() -> Color32 {
        Color32::from_rgb(230, 190, 80)
    }

    fn shape() -> SocketShape {
        SocketShape::Diamond
    }
}
