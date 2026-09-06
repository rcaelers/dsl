use egui::epaint::CubicBezierShape;
use egui::{Color32, Painter, Pos2, Stroke};

use super::geometry::{PathSegment, WirePath};

pub(crate) fn draw_path(
    painter: &Painter,
    path: &WirePath,
    to_screen: impl Fn(Pos2) -> Pos2,
    color: Color32,
    width: f32,
) {
    draw_path_shadow(painter, path, &to_screen, width);
    draw_path_stroke(painter, path, to_screen, Stroke::new(width, color));
}

/// Paint the outline separately so same-source branches can share a seamless fill.
pub(crate) fn draw_path_shadow(
    painter: &Painter,
    path: &WirePath,
    to_screen: impl Fn(Pos2) -> Pos2,
    width: f32,
) {
    draw_path_stroke(
        painter,
        path,
        to_screen,
        Stroke::new(width + 2.0, Color32::from_rgba_premultiplied(0, 0, 0, 170)),
    );
}

/// Paint one pass over the exact geometry used by wire interactions.
pub(crate) fn draw_path_stroke(
    painter: &Painter,
    path: &WirePath,
    to_screen: impl Fn(Pos2) -> Pos2,
    stroke: Stroke,
) {
    for segment in path.segments() {
        match segment {
            PathSegment::Line(points) => {
                painter.line_segment(points.map(&to_screen), stroke);
            }
            PathSegment::Cubic(points) => {
                painter.add(CubicBezierShape::from_points_stroke(
                    points.map(&to_screen),
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            }
        }
    }
}
