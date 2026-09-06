use std::collections::HashMap;

use egui::{Color32, Painter, Pos2, Stroke};

use super::layout::GraphWidgetLayout;
use super::routing::{WirePath, draw_path_shadow, draw_path_stroke};
use crate::model::{Connection, GraphState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireEmphasis {
    Normal,
    /// Selected-node, port-hover, routing-warning, or valid insert target:
    /// brighter and thicker, retaining the data-type color.
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
        wire_width,
    };
    let mut groups: Vec<Vec<PaintedConnection<'_>>> = Vec::new();
    let mut source_groups = HashMap::new();
    for (idx, conn) in graph.connections.iter().enumerate() {
        let emphasis = emphasis(idx, conn);
        let Some(connection) = context.connection(conn, emphasis) else {
            continue;
        };
        let group = *source_groups.entry(conn.from).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group].push(connection);
    }
    // A source socket is one visual network. Paint all its outlines before any
    // fills, including mixed-emphasis branches, so siblings cannot cut a seam
    // through a shared run or T junction. Keep independent signals separate.
    groups.sort_by_key(|group| group.iter().any(|c| c.emphasis == WireEmphasis::Highlight));
    for mut group in groups {
        group.sort_by_key(|c| c.emphasis == WireEmphasis::Highlight);
        for connection in &group {
            draw_path_shadow(painter, connection.path, to_screen, connection.stroke.width);
        }
        for connection in &group {
            draw_path_stroke(painter, connection.path, to_screen, connection.stroke);
        }
    }
}

struct PaintedConnection<'a> {
    path: &'a WirePath,
    stroke: Stroke,
    emphasis: WireEmphasis,
}

struct ConnectionPaintContext<'a> {
    graph: &'a GraphState,
    registry: &'a crate::runtime::NodeTypeRegistry,
    layout: &'a GraphWidgetLayout,
    wire_width: f32,
}

impl ConnectionPaintContext<'_> {
    fn connection(
        &self,
        conn: &Connection,
        emphasis: WireEmphasis,
    ) -> Option<PaintedConnection<'_>> {
        let path = self.layout.wire_paths.get(&(conn.from, conn.to))?;
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
        // Color identifies the data type, including on diagnostic fallback paths.
        // Routing failures are presented by separate markers and explanations.
        let (color, width) = match emphasis {
            WireEmphasis::Normal => (base, self.wire_width),
            WireEmphasis::Highlight => (brighten_wire_color(base), self.wire_width * 2.0),
            WireEmphasis::Muted => (mute_wire_color(base), self.wire_width),
            WireEmphasis::Hidden => return None,
        };
        Some(PaintedConnection {
            path,
            stroke: Stroke::new(width, color),
            emphasis,
        })
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

    fn junction_shapes(
        source_variant: usize,
        emphasis: [WireEmphasis; 2],
        reverse: bool,
        zoom: f32,
    ) -> (Vec<Shape>, Color32) {
        let (mut widget, mut layout, target) = fixture(zoom);
        let first = widget.graph.connections[0].clone();
        let mut from = first.from;
        match source_variant {
            0 => {}
            1 => {
                let source = widget.graph.nodes.get_mut(&from.node).unwrap();
                source.outputs.push(source.outputs[0].clone());
                from.index = 1;
            }
            2 => from.node = first.to.node,
            _ => panic!("source variant"),
        }
        let second = Connection {
            from,
            to: SocketId {
                node: target,
                index: 0,
                direction: SocketDirection::Input,
            },
        };
        for (connection, points) in [
            (
                &first,
                vec![[0.0, 50.0], [100.0, 50.0], [100.0, 100.0], [200.0, 100.0]],
            ),
            (&second, vec![[0.0, 50.0], [100.0, 50.0], [100.0, 200.0]]),
        ] {
            layout.wire_paths.insert(
                (connection.from, connection.to),
                WirePath::new(
                    points
                        .windows(2)
                        .map(|p| PathSegment::Line([Pos2::from(p[0]), Pos2::from(p[1])]))
                        .collect(),
                    0.5 / zoom,
                ),
            );
        }
        widget.graph.connections.push(second);
        if reverse {
            widget.graph.connections.reverse();
        }
        let base = widget
            .registry
            .socket_display(&widget.graph.nodes[&first.from.node].outputs[0])
            .0;
        let context = egui::Context::default();
        context.begin_pass(egui::RawInput::default());
        draw_connections(
            &context.layer_painter(egui::LayerId::background()),
            &widget.graph,
            &widget.registry,
            &layout,
            |p| Pos2::new(20.0, 30.0) + p.to_vec2() * zoom,
            2.0,
            |_, c| emphasis[usize::from(c.to != first.to)],
        );
        let mut output = context.end_pass();
        output.textures_delta.clear();
        (output.shapes.into_iter().map(|s| s.shape).collect(), base)
    }

    /// Last covering stroke away from antialiasing boundaries: a later dark
    /// outline here is the visible seam, even when the centerlines are joined.
    fn junction_ink(shapes: &[Shape], point: Pos2) -> Option<Color32> {
        shapes
            .iter()
            .filter_map(|shape| {
                let Shape::LineSegment {
                    points: [a, b],
                    stroke,
                } = shape
                else {
                    panic!("line fixture")
                };
                let d = *b - *a;
                let t = ((point - *a).dot(d) / d.length_sq()).clamp(0.0, 1.0);
                (point.distance(*a + d * t) < stroke.width * 0.5).then_some(stroke.color)
            })
            .next_back()
    }

    #[test]
    fn same_output_junction_has_no_outline_seam_in_either_paint_order() {
        for zoom in [0.25, 1.0, 3.0] {
            for reverse in [false, true] {
                for emphasis in [
                    [WireEmphasis::Normal; 2],
                    [WireEmphasis::Highlight; 2],
                    [WireEmphasis::Normal, WireEmphasis::Highlight],
                    [WireEmphasis::Highlight, WireEmphasis::Normal],
                    [WireEmphasis::Muted, WireEmphasis::Highlight],
                ] {
                    let (shapes, base) = junction_shapes(0, emphasis, reverse, zoom);
                    assert_eq!(shapes.len(), 10);
                    let expected = match emphasis[0] {
                        WireEmphasis::Normal => base,
                        WireEmphasis::Highlight => brighten_wire_color(base),
                        WireEmphasis::Muted => mute_wire_color(base),
                        WireEmphasis::Hidden => unreachable!(),
                    };
                    let offset = if emphasis[1] == WireEmphasis::Highlight {
                        2.5
                    } else {
                        1.5
                    };
                    let point = Pos2::new(20.0 + 100.0 * zoom + offset, 30.0 + 100.0 * zoom);
                    assert_eq!(
                        junction_ink(&shapes, point),
                        Some(expected),
                        "a sibling outline must not cut the branch"
                    );
                }
            }
        }
    }

    #[test]
    fn different_outputs_keep_crossing_outlines_and_hidden_branches_do_not_paint() {
        for source_variant in [1, 2] {
            let (shapes, _) =
                junction_shapes(source_variant, [WireEmphasis::Normal; 2], false, 1.0);
            assert_eq!(
                junction_ink(&shapes, Pos2::new(121.5, 130.0)),
                Some(Color32::from_rgba_premultiplied(0, 0, 0, 170))
            );
        }
        let (shapes, base) =
            junction_shapes(0, [WireEmphasis::Normal, WireEmphasis::Hidden], false, 1.0);
        assert_eq!(shapes.len(), 6);
        assert_eq!(junction_ink(&shapes, Pos2::new(121.5, 130.0)), Some(base));
        assert_eq!(junction_ink(&shapes, Pos2::new(120.0, 180.0)), None);
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
    fn failed_routes_preserve_legacy_control_points_without_document_edits() {
        for zoom in [0.25, 1.0, 3.0] {
            let (mut widget, _, candidate) = fixture(zoom);
            widget.graph.nodes.get_mut(&candidate).unwrap().pos.x = 25.0;
            widget.graph.nodes.get_mut(&candidate).unwrap().pos.y = 0.0;
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

    #[test]
    fn routing_failures_do_not_override_type_color_or_interaction_emphasis() {
        let (mut widget, mut layout, _) = fixture(1.0);
        let conn = widget.graph.connections[0].clone();
        let key = (conn.from, conn.to);
        for base in [
            Color32::from_rgb(20, 180, 210),
            Color32::from_rgb(190, 70, 120),
        ] {
            widget.graph.nodes.get_mut(&conn.from.node).unwrap().outputs[conn.from.index].color =
                crate::support::graph_color(base);
            let base = widget
                .registry
                .socket_display(&widget.graph.nodes[&conn.from.node].outputs[conn.from.index])
                .0;
            for failure in [
                None,
                Some(super::super::routing::RouteFailure::InvalidGeometry),
                Some(super::super::routing::RouteFailure::BlockedEscape),
                Some(super::super::routing::RouteFailure::NoCorridor),
                Some(super::super::routing::RouteFailure::WorkLimit),
            ] {
                layout.wire_failures.clear();
                if let Some(failure) = failure {
                    layout.wire_failures.insert(key, failure);
                }
                for (emphasis, color, width) in [
                    (WireEmphasis::Normal, base, 2.0),
                    (WireEmphasis::Highlight, brighten_wire_color(base), 4.0),
                    (WireEmphasis::Muted, mute_wire_color(base), 2.0),
                    (WireEmphasis::Hidden, base, 2.0),
                ] {
                    let context = egui::Context::default();
                    context.begin_pass(egui::RawInput::default());
                    draw_connections(
                        &context.layer_painter(egui::LayerId::background()),
                        &widget.graph,
                        &widget.registry,
                        &layout,
                        |p| p,
                        2.0,
                        |_, _| emphasis,
                    );
                    let mut output = context.end_pass();
                    output.textures_delta.clear();
                    if emphasis == WireEmphasis::Hidden {
                        assert!(output.shapes.is_empty());
                        continue;
                    }
                    // The fixture paints three shadows followed by the three wire segments.
                    assert_eq!(output.shapes.len(), 6);
                    for shape in &output.shapes[3..] {
                        match &shape.shape {
                            Shape::CubicBezier(curve) => {
                                assert_eq!(
                                    curve.stroke.color,
                                    egui::epaint::ColorMode::Solid(color)
                                );
                                assert_eq!(curve.stroke.width, width);
                            }
                            Shape::LineSegment { stroke, .. } => {
                                assert_eq!(stroke.color, color);
                                assert_eq!(stroke.width, width);
                            }
                            _ => panic!("unexpected wire shape"),
                        }
                    }
                }
            }
        }
    }
}
