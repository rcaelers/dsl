use egui::{Color32, Pos2};

use crate::model::{GraphColor, GraphPosition};

pub(crate) fn graph_color(color: Color32) -> GraphColor {
    GraphColor::from_rgba_unmultiplied(color.r(), color.g(), color.b(), color.a())
}

pub(crate) fn egui_color(color: GraphColor) -> Color32 {
    let [red, green, blue, alpha] = color.to_array();
    Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

pub(crate) fn graph_position(position: Pos2) -> GraphPosition {
    GraphPosition::new(position.x, position.y)
}

pub(crate) fn egui_position(position: GraphPosition) -> Pos2 {
    Pos2::new(position.x, position.y)
}
