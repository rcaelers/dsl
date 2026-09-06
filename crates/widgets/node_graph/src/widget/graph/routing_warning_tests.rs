use egui::{Event, Pos2, Rect, Shape, Vec2};

use super::layout::GraphWidgetLayout;
use super::render::GraphRenderContext;
use super::routing::{PathSegment, RouteFailure};
use super::routing_presentation::routing_warning_highlights;
use super::widget::NodeGraphWidget;
use crate::model::{Connection, SocketDirection, SocketId};
use crate::runtime::NodeTypeRegistry;

struct Fixture {
    widget: NodeGraphWidget,
    layout: GraphWidgetLayout,
    context: egui::Context,
    connections: Vec<Connection>,
    frame: usize,
}

impl Fixture {
    fn new() -> Self {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let a = widget
            .add_node_at("Reroute", Pos2::new(50.0, 100.0))
            .unwrap();
        let b = widget
            .add_node_at("Reroute", Pos2::new(50.0, 350.0))
            .unwrap();
        for (index, source) in [a, a, a, b].into_iter().enumerate() {
            let target = widget
                .add_node_at("Reroute", Pos2::new(500.0, 100.0 + index as f32 * 130.0))
                .unwrap();
            widget.graph.add_connection(
                SocketId {
                    node: source,
                    index: 0,
                    direction: SocketDirection::Output,
                },
                SocketId {
                    node: target,
                    index: 0,
                    direction: SocketDirection::Input,
                },
            );
        }
        let mut layout = widget.build_layout(Pos2::ZERO);
        assert!(layout.wire_failures.is_empty());
        let connections = widget.graph.connections.clone();
        for index in [0, 1, 3] {
            let c = &connections[index];
            layout
                .wire_failures
                .insert((c.from, c.to), RouteFailure::NoCorridor);
        }
        let context = egui::Context::default();
        context.global_style_mut(|s| {
            s.interaction.tooltip_delay = 0.0;
            s.interaction.show_tooltips_only_when_still = false;
        });
        Self {
            widget,
            layout,
            context,
            connections,
            frame: 0,
        }
    }

    fn badge(&self, connection: usize) -> Pos2 {
        self.layout.node_screen_rects[&self.connections[connection].from.node].right_top()
            + Vec2::new(-8.0, -9.0)
    }

    fn draw(
        &mut self,
        pointer: Pos2,
        socket: Option<SocketId>,
        allow_graph: bool,
        clip: Rect,
    ) -> (Vec<(egui::epaint::ColorMode, f32)>, bool, usize) {
        self.frame += 1;
        self.context.begin_pass(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            time: Some(self.frame as f64 / 30.0),
            events: vec![Event::PointerMoved(pointer)],
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            self.context.clone(),
            egui::Id::new("warning-hover-test"),
            egui::UiBuilder::new().max_rect(clip),
        );
        let painter = ui.painter().with_clip_rect(clip);
        let graph_pointer = allow_graph.then_some(pointer);
        let highlights = routing_warning_highlights(&mut ui, &self.layout, clip, graph_pointer);
        let count = highlights.as_ref().map_or(0, |keys| keys.len());
        self.widget.draw_graph(
            &mut ui,
            &painter,
            GraphRenderContext {
                rect: clip,
                origin: Pos2::ZERO,
                pointer: Some(pointer),
                layout: &self.layout,
                response_layout: None,
                hovered_socket: socket,
                routing_warning_highlights: highlights.as_ref(),
            },
        );
        self.widget.show_routing_presentation(
            &mut ui,
            &painter,
            &self.layout,
            (clip, Pos2::ZERO),
            graph_pointer,
        );
        let mut output = self.context.end_pass();
        output.textures_delta.clear();
        let strokes = self
            .connections
            .iter()
            .map(|c| {
                let segment = self.layout.wire_paths[&(c.from, c.to)]
                    .segments()
                    .last()
                    .unwrap();
                output
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|s| match (&s.shape, segment) {
                        (Shape::LineSegment { points, stroke }, PathSegment::Line(expected))
                            if points == expected =>
                        {
                            Some((egui::epaint::ColorMode::Solid(stroke.color), stroke.width))
                        }
                        (Shape::CubicBezier(curve), PathSegment::Cubic(expected))
                            if &curve.points == expected =>
                        {
                            Some((curve.stroke.color.clone(), curve.stroke.width))
                        }
                        _ => None,
                    })
                    .expect("painted wire end")
            })
            .collect();
        let tooltip = output.shapes.iter().any(|s| {
            matches!(&s.shape, Shape::Text(text)
            if text.galley.text().contains("Connection could not be routed."))
        });
        (strokes, tooltip, count)
    }
}

#[test]
fn warning_hover_uses_port_emphasis_only_for_its_failures_and_restores_selection() {
    let mut f = Fixture::new();
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0));
    let away = Pos2::new(800.0, 650.0);
    let (normal, _, _) = f.draw(away, None, true, clip);
    let source = f.connections[0].from;
    let (port, _, _) = f.draw(away, Some(source), true, clip);
    assert!(port[0].1 > normal[0].1);
    for node in f.widget.graph.nodes.values_mut() {
        node.selected = true;
    }
    let document = serde_json::to_value(&f.widget.graph).unwrap();
    let (selected, _, _) = f.draw(away, None, true, clip);
    let badge = f.badge(0);
    let mut tooltip_shown = false;
    for _ in 0..4 {
        let (hover, tooltip, count) = f.draw(badge, None, true, clip);
        assert_eq!(count, 2);
        assert_eq!(&hover[..2], &port[..2]);
        assert_eq!(
            &hover[2..],
            &normal[2..],
            "healthy and other-badge failures ignore selection"
        );
        tooltip_shown |= tooltip;
    }
    assert!(tooltip_shown, "warning popup remains available");
    let badge = f.badge(3);
    let (hover, _, count) = f.draw(badge, None, true, clip);
    assert_eq!(count, 1);
    assert_eq!(&hover[..3], &normal[..3]);
    assert_eq!(hover[3], selected[3]);
    let (restored, _, count) = f.draw(away, None, true, clip);
    assert_eq!(count, 0);
    assert_eq!(restored, selected);
    let badge = f.badge(0);
    let (_, _, count) = f.draw(badge, None, true, clip);
    assert_eq!(count, 2);
    f.layout.wire_failures.clear();
    let (recovered, _, count) = f.draw(badge, None, true, clip);
    assert_eq!(
        count, 0,
        "routing recovery removes the hover override immediately"
    );
    assert_eq!(recovered, selected);
    assert_eq!(document, serde_json::to_value(&f.widget.graph).unwrap());
}

#[test]
fn covered_or_clipped_badges_do_not_suppress_selection() {
    let mut f = Fixture::new();
    for node in f.widget.graph.nodes.values_mut() {
        node.selected = true;
    }
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0));
    let (selected, _, _) = f.draw(Pos2::new(800.0, 650.0), None, true, clip);
    let badge = f.badge(0);
    let (_, _, count) = f.draw(badge, None, true, clip);
    assert_eq!(count, 2);
    let (covered, _, count) = f.draw(badge, None, false, clip);
    assert_eq!(count, 0);
    assert_eq!(covered, selected);
    let clipped = Rect::from_min_max(Pos2::new(0.0, badge.y + 10.0), clip.max);
    let (_, _, count) = f.draw(badge, None, true, clipped);
    assert_eq!(count, 0);
}

#[test]
fn destination_warning_highlights_only_the_failures_anchored_there() {
    let mut f = Fixture::new();
    let c = f.connections[0].clone();
    f.layout.node_screen_rects.remove(&c.from.node);
    f.layout.wire_paths.remove(&(c.from, c.to));
    let pos = f.layout.node_screen_rects[&c.to.node].right_top() + Vec2::new(-8.0, -9.0);
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0));
    let mut highlighted = None;
    for _ in 0..3 {
        f.context.begin_pass(egui::RawInput {
            screen_rect: Some(clip),
            events: vec![Event::PointerMoved(pos)],
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            f.context.clone(),
            egui::Id::new("destination-warning"),
            egui::UiBuilder::new().max_rect(clip),
        );
        highlighted = routing_warning_highlights(&mut ui, &f.layout, clip, Some(pos));
        let mut output = f.context.end_pass();
        output.textures_delta.clear();
    }
    let highlighted =
        highlighted.expect("destination badge remains hoverable without a drawable path");
    assert_eq!(highlighted.len(), 1);
    assert!(highlighted.contains(&(c.from, c.to)));
}
