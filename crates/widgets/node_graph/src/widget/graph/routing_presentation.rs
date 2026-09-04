//! Routing warnings and optional geometry diagnostics; never mutates graph topology.

use std::collections::HashSet;

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Vec2};

use super::layout::GraphWidgetLayout;
use super::routing::{PathSegment, RouteConfig, RouteFailure};
use super::widget::NodeGraphWidget;
use crate::support::to_screen_rect;

#[derive(Default)]
pub(crate) struct RoutingDebug {
    pub(crate) open: bool,
    pub(crate) obstacles: bool,
    pub(crate) escapes: bool,
    pub(crate) results: bool,
}

fn failure_message(failure: RouteFailure) -> &'static str {
    match failure {
        RouteFailure::InvalidGeometry => "A node or socket has invalid geometry.",
        RouteFailure::BlockedEscape => {
            "Another node blocks a socket's exit. Move the nodes farther apart."
        }
        RouteFailure::NoCorridor => "No clear path was found between these sockets.",
        RouteFailure::WorkLimit => {
            "The routing work limit was reached. Simplify the layout to reduce routing work."
        }
    }
}

impl NodeGraphWidget {
    pub(crate) fn show_routing_presentation(
        &mut self,
        ui: &mut egui::Ui,
        painter: &Painter,
        layout: &GraphWidgetLayout,
        viewport: (Rect, Pos2),
        pointer: Option<Pos2>,
    ) {
        let (rect, origin) = viewport;
        let color = Color32::from_rgb(255, 170, 50);
        let mut failures: Vec<_> = layout.wire_failures.iter().collect();
        failures.sort_by_key(|((from, to), _)| (from.node.0, from.index, to.node.0, to.index));
        let mut marked = HashSet::new();
        for &(&(from, to), &failure) in &failures {
            let fallback = if layout.wire_paths.contains_key(&(from, to)) {
                "The orange fallback may pass through nodes."
            } else {
                "The wire cannot be drawn until its socket geometry is valid."
            };
            let explanation = format!(
                "Connection could not be routed. {} {fallback}",
                failure_message(failure)
            );
            // Prefer the source, then the destination when the source cannot be drawn.
            let anchor = [from.node, to.node].into_iter().find_map(|id| {
                let body = *layout.node_screen_rects.get(&id)?;
                (body.min.is_finite() && body.max.is_finite())
                    .then_some((id, body.right_top() + Vec2::new(-8.0, -9.0)))
            });
            if let Some((id, pos)) = anchor
                && marked.insert(id)
            {
                painter.circle_filled(pos, 7.0, color);
                painter.text(
                    pos,
                    Align2::CENTER_CENTER,
                    "!",
                    FontId::proportional(12.0),
                    Color32::BLACK,
                );
                ui.interact(
                    Rect::from_center_size(pos, Vec2::splat(16.0)).intersect(rect),
                    ui.id().with(("routing-warning", id.0)),
                    Sense::hover(),
                )
                .on_hover_text(&explanation);
            }
            if let Some(pointer) = pointer {
                let canvas = self.view.screen_to_canvas(origin, pointer);
                if !layout
                    .node_screen_rects
                    .values()
                    .any(|r| r.contains(pointer))
                    && layout
                        .wire_paths
                        .get(&(from, to))
                        .is_some_and(|path| path.distance(canvas) * self.view.zoom <= 6.0)
                {
                    ui.interact(
                        Rect::from_center_size(pointer, Vec2::splat(8.0)).intersect(rect),
                        ui.id().with(("routing-wire", from, to)),
                        Sense::hover(),
                    )
                    .on_hover_text(&explanation);
                }
            }
        }
        if !failures.is_empty() {
            let message = format!("{} connection(s) could not be routed", failures.len());
            let details = failures
                .iter()
                .map(|((from, to), failure)| {
                    let title = |id| {
                        self.graph
                            .nodes
                            .get(&id)
                            .map_or("Missing node", |node| node.title.as_str())
                    };
                    format!(
                        "{} → {}: {}",
                        title(from.node),
                        title(to.node),
                        failure_message(**failure)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            ui.put(
                Rect::from_min_size(
                    rect.left_bottom() + Vec2::new(10.0, -26.0),
                    Vec2::new(290.0, 22.0),
                ),
                egui::Label::new(egui::RichText::new(message).color(color)),
            )
            .on_hover_text(details);
        }
        if self.routing_debug.open {
            let mut open = true;
            egui::Window::new("Routing diagnostics")
                .id(ui.id().with("routing-debug"))
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.checkbox(&mut self.routing_debug.obstacles, "Expanded obstacles");
                    ui.checkbox(&mut self.routing_debug.escapes, "Port escapes");
                    ui.checkbox(&mut self.routing_debug.results, "Route results");
                    ui.label(format!(
                        "{} checked, {} unroutable",
                        layout
                            .wire_paths
                            .keys()
                            .filter(|key| !layout.wire_failures.contains_key(key))
                            .count(),
                        failures.len()
                    ));
                    if layout.routing_excluded.is_some() {
                        ui.label("Provisional splice preview: the dragged node is excluded.");
                    }
                });
            self.routing_debug.open = open;
            let config = RouteConfig::default();
            if self.routing_debug.obstacles {
                for (&id, body) in &layout.node_rects {
                    let expanded = body.expand2(Vec2::new(config.clearance_x, config.clearance_y));
                    if expanded.min.is_finite() && expanded.max.is_finite() {
                        let color = if Some(id) == layout.routing_excluded {
                            Color32::GRAY
                        } else {
                            Color32::LIGHT_RED
                        };
                        painter.rect_stroke(
                            to_screen_rect(expanded, &self.view, origin),
                            0.0,
                            Stroke::new(1.0, color),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
            }
            for (key, path) in &layout.wire_paths {
                if self.routing_debug.escapes && !layout.wire_failures.contains_key(key) {
                    for segment in [path.segments().first(), path.segments().last()]
                        .into_iter()
                        .flatten()
                    {
                        if let PathSegment::Line(points) = segment {
                            let screen =
                                points.map(|point| self.view.canvas_to_screen(origin, point));
                            painter.line_segment(screen, Stroke::new(2.0, Color32::LIGHT_GREEN));
                            painter.circle_filled(screen[0], 3.0, Color32::LIGHT_GREEN);
                            painter.circle_filled(screen[1], 3.0, Color32::LIGHT_GREEN);
                        }
                    }
                }
                if self.routing_debug.results {
                    let label = if layout.wire_failures.contains_key(key) {
                        "fallback"
                    } else if layout.routing_excluded.is_some() {
                        "provisional"
                    } else {
                        "checked"
                    };
                    painter.text(
                        self.view.canvas_to_screen(origin, path.bounds().center()),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::monospace(10.0),
                        color,
                    );
                }
            }
        }
    }
}
