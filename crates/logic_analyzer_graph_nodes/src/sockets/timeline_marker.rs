use egui::Color32;

use node_graph::api::{SocketDef, SocketShape};

/// A persisted point on the shared graph timeline.
pub(crate) struct TimelineMarker;

impl SocketDef for TimelineMarker {
    type Value = ();

    fn type_name() -> &'static str {
        "Timeline Marker"
    }

    fn color() -> Color32 {
        Color32::from_rgb(245, 150, 55)
    }

    fn shape() -> SocketShape {
        SocketShape::Square
    }
}
