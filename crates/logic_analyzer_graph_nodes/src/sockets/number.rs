use egui::Color32;

use node_graph::api::SocketDef;

/// Integer level stream (`NumberSample` at runtime).
pub(crate) struct Number;

impl SocketDef for Number {
    type Value = i64;

    fn type_name() -> &'static str {
        "Number"
    }

    fn color() -> Color32 {
        Color32::from_rgb(95, 145, 210)
    }
}
