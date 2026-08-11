use egui::Color32;

use node_graph::api::SocketDef;

/// Text level stream (`TextSample` at runtime).
pub(crate) struct Text;

impl SocketDef for Text {
    type Value = String;

    fn type_name() -> &'static str {
        "Text"
    }

    fn color() -> Color32 {
        Color32::from_rgb(215, 150, 170)
    }
}
