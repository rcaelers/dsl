use egui::Color32;

use node_graph::api::SocketDef;

/// Logic level stream (`Sample` at runtime): defined at every instant.
pub(crate) struct Signal;

impl SocketDef for Signal {
    type Value = bool;

    fn type_name() -> &'static str {
        "Signal"
    }

    fn color() -> Color32 {
        Color32::from_rgb(0, 205, 160)
    }
}
