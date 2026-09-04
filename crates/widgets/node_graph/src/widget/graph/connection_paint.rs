use egui::{Color32, Painter, Pos2};

use super::layout::GraphWidgetLayout;
use super::routing::draw_path;
use crate::model::{Connection, GraphState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireEmphasis {
    Normal,
    /// Connected to a selected node, or a valid insert target for the
    /// dragged node: brighter and thicker.
    Highlight,
    /// Insert target the dragged node cannot splice into: dimmed.
    Muted,
    /// Not painted at all: the link a wire drag is carrying elsewhere, which
    /// stays in the document until the drag lands.
    Hidden,
}

fn brighten_wire_color(base: Color32) -> Color32 {
    let mix = |channel: u8| ((channel as f32 * 0.48) + (255.0 * 0.52)).round() as u8;
    Color32::from_rgba_unmultiplied(mix(base.r()), mix(base.g()), mix(base.b()), 255)
}

fn mute_wire_color(base: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (base.r() as f32 * 0.35) as u8,
        (base.g() as f32 * 0.35) as u8,
        (base.b() as f32 * 0.35) as u8,
        255,
    )
}

pub(crate) fn draw_connections(
    painter: &Painter,
    graph: &GraphState,
    registry: &crate::runtime::NodeTypeRegistry,
    layout: &GraphWidgetLayout,
    to_screen: impl Fn(Pos2) -> Pos2 + Copy,
    wire_width: f32,
    emphasis: impl Fn(usize, &Connection) -> WireEmphasis,
) {
    let context = ConnectionPaintContext {
        graph,
        registry,
        layout,
        to_screen,
        wire_width,
    };
    let mut highlighted = Vec::new();
    for (idx, conn) in graph.connections.iter().enumerate() {
        let emphasis = emphasis(idx, conn);
        if emphasis == WireEmphasis::Hidden {
            continue;
        }
        if emphasis == WireEmphasis::Highlight {
            highlighted.push((idx, conn));
            continue;
        }
        context.draw(painter, conn, emphasis);
    }
    for (idx, conn) in highlighted {
        context.draw(painter, conn, emphasis(idx, conn));
    }
}

struct ConnectionPaintContext<'a, F> {
    graph: &'a GraphState,
    registry: &'a crate::runtime::NodeTypeRegistry,
    layout: &'a GraphWidgetLayout,
    to_screen: F,
    wire_width: f32,
}

impl<F: Fn(Pos2) -> Pos2> ConnectionPaintContext<'_, F> {
    fn draw(&self, painter: &Painter, conn: &Connection, emphasis: WireEmphasis) {
        let Some(path) = self.layout.wire_paths.get(&(conn.from, conn.to)) else {
            return;
        };
        // `socket.color` is the socket's *idle* look; a resolved polymorphic
        // socket (e.g. a reroute's `Any` output taking on whatever flows
        // through it) needs the connected type's registry-wide color instead —
        // the same lookup socket dots already use — or the wire renders in the
        // socket's flat default color forever, mismatched with the dot beside it.
        let base = self
            .graph
            .nodes
            .get(&conn.from.node)
            .and_then(|n| n.outputs.get(conn.from.index))
            .map(|s| self.registry.socket_display(s).0)
            .unwrap_or(Color32::from_rgb(160, 160, 160));
        let (color, width) = match emphasis {
            WireEmphasis::Normal => (base, self.wire_width),
            WireEmphasis::Highlight => (brighten_wire_color(base), self.wire_width * 2.0),
            WireEmphasis::Muted => (mute_wire_color(base), self.wire_width),
            WireEmphasis::Hidden => return,
        };
        draw_path(painter, path, &self.to_screen, color, width);
    }
}

#[cfg(test)]
mod connection_paint_tests {
    use egui::{Pos2, Rect, Shape, Vec2};

    use super::super::routing::{PathSegment, WirePath};
    use super::super::widget::NodeGraphWidget;
    use super::*;
    use crate::model::{NodeId, SocketDirection, SocketId};
    use crate::runtime::NodeTypeRegistry;

    fn fixture(zoom: f32) -> (NodeGraphWidget, GraphWidgetLayout, NodeId) {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        widget.view.zoom = zoom;
        let a = widget.add_node_at("Reroute", Pos2::ZERO).unwrap();
        let b = widget
            .add_node_at("Reroute", Pos2::new(300.0, 0.0))
            .unwrap();
        let candidate = widget
            .add_node_at("Reroute", Pos2::new(140.0, 90.0))
            .unwrap();
        let from = SocketId {
            node: a,
            index: 0,
            direction: SocketDirection::Output,
        };
        let to = SocketId {
            node: b,
            index: 0,
            direction: SocketDirection::Input,
        };
        widget.graph.add_connection(from, to);
        let mut layout = widget.build_layout(Pos2::ZERO);
        let start = layout.nodes[&a].output_socket_pos(0).unwrap();
        let end = layout.nodes[&b].input_socket_pos(0).unwrap();
        layout.wire_paths.insert(
            (from, to),
            WirePath::new(
                vec![
                    PathSegment::Cubic([
                        start,
                        Pos2::new(80.0, 12.0),
                        Pos2::new(80.0, 102.0),
                        Pos2::new(100.0, 102.0),
                    ]),
                    PathSegment::Line([Pos2::new(100.0, 102.0), Pos2::new(200.0, 102.0)]),
                    PathSegment::Cubic([
                        Pos2::new(200.0, 102.0),
                        Pos2::new(230.0, 102.0),
                        Pos2::new(230.0, 12.0),
                        end,
                    ]),
                ],
                0.5 / zoom,
            ),
        );
        (widget, layout, candidate)
    }

    #[test]
    fn painting_and_gestures_follow_the_same_multi_segment_detour() {
        for zoom in [0.25, 1.0, 3.0] {
            let (mut widget, layout, candidate) = fixture(zoom);
            let point = Pos2::new(150.0, 102.0);
            assert_eq!(widget.wire_near_point(point, &layout), Some(0));
            assert_eq!(
                widget.wire_near_point(point + Vec2::new(0.0, 3.0 / zoom), &layout),
                Some(0)
            );
            assert_eq!(
                widget.wire_near_point(Pos2::new(150.0, 12.0), &layout),
                None
            );
            assert_eq!(
                widget.compute_insert_candidate_wire(candidate, Some(point), &layout),
                Some((0, true))
            );

            let context = egui::Context::default();
            context.begin_pass(egui::RawInput::default());
            let painter = context.layer_painter(egui::LayerId::background());
            let origin = Pos2::new(20.0, 30.0);
            draw_connections(
                &painter,
                &widget.graph,
                &widget.registry,
                &layout,
                |p| origin + p.to_vec2() * zoom,
                2.0,
                |_, _| WireEmphasis::Normal,
            );
            let mut output = context.end_pass();
            output.textures_delta.clear();
            let painted: Vec<_> = output.shapes.iter().map(|shape| &shape.shape).collect();
            assert_eq!(painted.len(), 6, "three segments, shadow then color");
            let connection = &widget.graph.connections[0];
            for (shape, segment) in painted[3..]
                .iter()
                .zip(layout.wire_paths[&(connection.from, connection.to)].segments())
            {
                match (shape, segment) {
                    (Shape::CubicBezier(curve), PathSegment::Cubic(points)) => {
                        assert_eq!(curve.points, points.map(|p| origin + p.to_vec2() * zoom))
                    }
                    (Shape::LineSegment { points: actual, .. }, PathSegment::Line(points)) => {
                        assert_eq!(*actual, points.map(|p| origin + p.to_vec2() * zoom))
                    }
                    _ => panic!("painted shape differs from interaction path"),
                }
            }
            widget.apply_knife_cut(&[Pos2::new(150.0, 10.0), Pos2::new(150.0, 14.0)], &layout);
            assert_eq!(
                widget.graph.connections.len(),
                1,
                "old endpoint curve is not cut"
            );
            widget.apply_knife_cut(&[point - Vec2::Y, point + Vec2::Y], &layout);
            assert!(widget.graph.connections.is_empty());
        }
    }

    #[test]
    fn reroute_and_node_splice_target_the_detour() {
        let (mut widget, layout, _) = fixture(1.0);
        let point = Pos2::new(150.0, 102.0);
        let index = widget.wire_near_point(point, &layout).unwrap();
        widget.insert_reroute_on_wire(index, point);
        assert_eq!(widget.graph.connections.len(), 2);
        assert_eq!(widget.graph.nodes.len(), 4);

        let (mut widget, layout, candidate) = fixture(1.0);
        widget.try_wire_insert(candidate, Some(point), &layout);
        assert_eq!(widget.graph.connections.len(), 2);
        assert!(
            widget
                .graph
                .connections
                .iter()
                .all(|c| c.from.node == candidate || c.to.node == candidate)
        );
    }

    #[test]
    fn layout_preserves_screen_space_legacy_control_points_without_document_edits() {
        for zoom in [0.25, 1.0, 3.0] {
            let (mut widget, _, _) = fixture(zoom);
            widget.view.pan = Vec2::new(42.0, -13.0);
            let before = serde_json::to_value(&widget.graph).unwrap();
            let undo_len = widget.undo_stack.len();
            let origin = Pos2::new(25.0, 16.0);
            let layout = widget.build_layout(origin);
            let conn = &widget.graph.connections[0];
            let from = layout.socket_screen_pos[&conn.from];
            let to = layout.socket_screen_pos[&conn.to];
            let dx = (to.x - from.x).abs().max(50.0) * 0.5;
            let expected = [from, from + Vec2::new(dx, 0.0), to - Vec2::new(dx, 0.0), to];
            let PathSegment::Cubic(points) = layout.wire_paths[&(conn.from, conn.to)].segments()[0]
            else {
                panic!("legacy cubic")
            };
            for (actual, expected) in points
                .map(|p| widget.view.canvas_to_screen(origin, p))
                .into_iter()
                .zip(expected)
            {
                assert!(actual.distance(expected) < 0.001);
            }
            assert_eq!(before, serde_json::to_value(&widget.graph).unwrap());
            assert_eq!(undo_len, widget.undo_stack.len());
            assert!(
                layout.wire_paths[&(conn.from, conn.to)]
                    .bounds()
                    .intersects(Rect::EVERYTHING)
            );
        }
    }
}
