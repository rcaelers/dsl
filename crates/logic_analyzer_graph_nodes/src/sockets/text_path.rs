use egui::Color32;

use node_graph::api::{FileValue, SocketDef, SocketWithControlDef};

use super::text::Text;

/// A text input whose unconnected inline control edits a file path.
///
/// The associated [`FileValue`] selects open or save dialog behavior.
pub(crate) struct TextPath;

impl SocketDef for TextPath {
    type Value = String;

    fn type_name() -> &'static str {
        Text::type_name()
    }

    fn color() -> Color32 {
        Text::color()
    }
}

impl SocketWithControlDef for TextPath {
    type Control = FileValue;
}
