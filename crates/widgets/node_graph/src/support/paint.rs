use std::collections::HashMap;

use egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use super::view::ViewState;
use crate::model::GraphState;

pub(crate) const SOCKET_RADIUS: f32 = 5.5;

pub(crate) fn to_screen_rect(r: Rect, view: &ViewState, origin: Pos2) -> Rect {
    Rect::from_min_max(
        view.canvas_to_screen(origin, r.min),
        view.canvas_to_screen(origin, r.max),
    )
}

/// Samples of the internal muted-node pass-through decoration.
fn bezier_wire_points(from: Pos2, to: Pos2, steps: usize) -> impl Iterator<Item = Pos2> {
    let dx = (to.x - from.x).abs().max(50.0) * 0.5;
    let cp1 = from + Vec2::new(dx, 0.0);
    let cp2 = to - Vec2::new(dx, 0.0);
    (0..=steps).map(move |k| {
        let t = k as f32 / steps as f32;
        let u = 1.0 - t;
        Pos2::new(
            u * u * u * from.x
                + 3.0 * u * u * t * cp1.x
                + 3.0 * u * t * t * cp2.x
                + t * t * t * to.x,
            u * u * u * from.y
                + 3.0 * u * u * t * cp1.y
                + 3.0 * u * t * t * cp2.y
                + t * t * t * to.y,
        )
    })
}

pub(crate) fn draw_grid(painter: &Painter, rect: Rect, view: &ViewState) {
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(28, 28, 28));

    let spacing = 20.0_f32;
    let screen_spacing = spacing * view.zoom;
    if screen_spacing < 3.0 {
        return;
    }
    let minor = Color32::from_rgb(38, 38, 38);
    let major = Color32::from_rgb(52, 52, 52);

    let kx0 = ((-view.pan.x) / screen_spacing).floor() as i32 - 1;
    let kx1 = ((rect.width() - view.pan.x) / screen_spacing).ceil() as i32 + 1;
    for k in kx0..=kx1 {
        let sx = rect.min.x + k as f32 * screen_spacing + view.pan.x;
        if sx < rect.min.x || sx > rect.max.x {
            continue;
        }
        let c = if k % 5 == 0 { major } else { minor };
        painter.line_segment(
            [Pos2::new(sx, rect.min.y), Pos2::new(sx, rect.max.y)],
            Stroke::new(1.0_f32, c),
        );
    }

    let ky0 = ((-view.pan.y) / screen_spacing).floor() as i32 - 1;
    let ky1 = ((rect.height() - view.pan.y) / screen_spacing).ceil() as i32 + 1;
    for k in ky0..=ky1 {
        let sy = rect.min.y + k as f32 * screen_spacing + view.pan.y;
        if sy < rect.min.y || sy > rect.max.y {
            continue;
        }
        let c = if k % 5 == 0 { major } else { minor };
        painter.line_segment(
            [Pos2::new(rect.min.x, sy), Pos2::new(rect.max.x, sy)],
            Stroke::new(1.0_f32, c),
        );
    }
}

pub(crate) fn draw_frames(
    painter: &Painter,
    graph: &GraphState,
    frame_rects: &HashMap<crate::model::FrameId, Rect>,
    view: &ViewState,
    origin: Pos2,
) {
    for frame in &graph.frames {
        let Some(&bounds) = frame_rects.get(&frame.id) else {
            continue;
        };
        let screen = to_screen_rect(bounds, view, origin);
        let r = CornerRadius::same(6);
        let c = frame.color;
        painter.rect_filled(
            screen,
            r,
            Color32::from_rgba_premultiplied(c.red(), c.green(), c.blue(), 28),
        );
        painter.rect_stroke(
            screen,
            r,
            Stroke::new(
                1.5_f32,
                Color32::from_rgba_premultiplied(c.red(), c.green(), c.blue(), 170),
            ),
            egui::StrokeKind::Middle,
        );
        let font_sz = (14.0 * view.zoom).clamp(8.0, 18.0);
        let label_pos = Pos2::new(screen.center().x, screen.min.y + 7.0 * view.zoom);
        let label_font = FontId::proportional(font_sz);
        painter.text(
            label_pos + Vec2::splat(1.0),
            egui::Align2::CENTER_TOP,
            &frame.label,
            label_font.clone(),
            Color32::from_rgba_premultiplied(0, 0, 0, 180),
        );
        painter.text(
            label_pos,
            egui::Align2::CENTER_TOP,
            &frame.label,
            label_font,
            Color32::from_rgba_premultiplied(245, 245, 245, 235),
        );
    }

    for frame in graph.frames.iter().filter(|frame| frame.selected) {
        let Some(&bounds) = frame_rects.get(&frame.id) else {
            continue;
        };
        let screen = to_screen_rect(bounds, view, origin);
        painter.rect_stroke(
            screen,
            CornerRadius::same(6),
            Stroke::new(2.0_f32, Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }
}

/// Legacy endpoint curve, dashed — for the internal pass-through
/// link a muted node draws between one of its own input and output sockets
/// (Blender's mute convention: external wires stay solid, an internal
/// dashed link shows what the node passes straight through).
pub(crate) fn draw_wire_dashed(
    painter: &Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    width: f32,
) {
    let points: Vec<Pos2> = bezier_wire_points(from, to, 48).collect();
    painter.extend(egui::Shape::dashed_line(
        &points,
        Stroke::new(width + 2.0, Color32::from_rgba_premultiplied(0, 0, 0, 170)),
        6.0,
        4.0,
    ));
    painter.extend(egui::Shape::dashed_line(
        &points,
        Stroke::new(width, color),
        6.0,
        4.0,
    ));
}

pub(crate) fn draw_box_select(painter: &Painter, start: Pos2, end: Pos2) {
    let rect = Rect::from_two_pos(start, end);
    painter.rect_filled(
        rect,
        CornerRadius::ZERO,
        Color32::from_rgba_premultiplied(80, 120, 220, 25),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, Color32::from_rgb(100, 150, 255)),
        egui::StrokeKind::Middle,
    );
}

pub(crate) fn draw_knife_line(painter: &Painter, points: &[Pos2]) {
    for w in points.windows(2) {
        painter.line_segment(
            [w[0], w[1]],
            Stroke::new(5.0_f32, Color32::from_rgba_premultiplied(255, 120, 30, 50)),
        );
        painter.line_segment(
            [w[0], w[1]],
            Stroke::new(1.5_f32, Color32::from_rgb(255, 170, 60)),
        );
    }
}
