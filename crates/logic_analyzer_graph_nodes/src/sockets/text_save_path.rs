use egui::Color32;

use node_graph::{FileValue, SocketDef, SocketWithControlDef};

use super::text::Text;

/// A text input whose unconnected inline control selects a save path.
pub(crate) struct TextSavePath;

impl SocketDef for TextSavePath {
    type Value = String;

    fn type_name() -> &'static str {
        Text::type_name()
    }

    fn color() -> Color32 {
        Text::color()
    }
}

impl SocketWithControlDef for TextSavePath {
    type Control = FileValue;
}
