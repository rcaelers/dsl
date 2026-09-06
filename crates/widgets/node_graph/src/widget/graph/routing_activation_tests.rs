use egui::{Pos2, Shape, Vec2};

use super::action::GraphAction;
use super::connection_paint::{WireEmphasis, draw_connections};
use super::interaction_state::InteractionState;
use super::routing::{PathSegment, RouteConfig, RouteFailure};
use super::widget::NodeGraphWidget;
use crate::model::{Connection, NodeId, NodeKind, SocketDirection, SocketId};
use crate::runtime::NodeTypeRegistry;

fn scene() -> (NodeGraphWidget, Connection, NodeId) {
    let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
    let source = widget
        .add_node_at("Reroute", Pos2::new(40.0, 120.0))
        .unwrap();
    let target = widget
        .add_node_at("Reroute", Pos2::new(500.0, 120.0))
        .unwrap();
    let obstacle = widget
        .add_node_at("Reroute", Pos2::new(250.0, 110.0))
        .unwrap();
    let connection = Connection {
        from: SocketId {
            node: source,
            index: 0,
            direction: SocketDirection::Output,
        },
        to: SocketId {
            node: target,
            index: 0,
            direction: SocketDirection::Input,
        },
    };
    widget.graph.add_connection(connection.from, connection.to);
    (widget, connection, obstacle)
}

fn path_geometry(widget: &NodeGraphWidget, connection: &Connection) -> Vec<Vec<Pos2>> {
    let layout = widget.build_layout(Pos2::ZERO);
    assert!(layout.wire_failures.is_empty());
    layout.wire_paths[&(connection.from, connection.to)]
        .segments()
        .iter()
        .map(|segment| match segment {
            PathSegment::Line(points) => points.to_vec(),
            PathSegment::Cubic(points) => points.to_vec(),
        })
        .collect()
}

#[test]
fn activated_paths_survive_pan_zoom_load_and_history_without_persisting_routes() {
    let (mut widget, connection, obstacle) = scene();
    let before = serde_json::to_value(&widget.graph).unwrap();
    let revision = widget.graph.semantic_revision();
    let undo_len = widget.undo_stack.len();
    let original = path_geometry(&widget, &connection);
    assert!(original.iter().any(|points| points.len() == 4));
    for zoom in [0.2, 1.0, 3.0] {
        widget.view.zoom = zoom;
        widget.view.pan = Vec2::new(100.0, -30.0);
        assert_eq!(path_geometry(&widget, &connection), original);
    }
    assert_eq!(before, serde_json::to_value(&widget.graph).unwrap());
    assert_eq!(widget.graph.semantic_revision(), revision);
    assert_eq!(widget.undo_stack.len(), undo_len);

    widget.push_undo_snapshot();
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 300.0;
    let moved = path_geometry(&widget, &connection);
    assert_ne!(moved, original);
    widget.undo();
    assert_eq!(path_geometry(&widget, &connection), original);
    widget.execute_action(GraphAction::Redo, &egui::Context::default(), None);
    assert_eq!(path_geometry(&widget, &connection), moved);
    widget.set_graph(serde_json::from_value(before).unwrap());
    assert_eq!(path_geometry(&widget, &connection), original);
}

#[test]
fn failure_fallback_keeps_type_color_is_editable_and_recovers_after_moving_the_obstacle() {
    let (mut widget, connection, obstacle) = scene();
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.x = 68.0;
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 120.0;
    let layout = widget.build_layout(Pos2::ZERO);
    assert_eq!(
        layout.wire_failures[&(connection.from, connection.to)],
        RouteFailure::BlockedEscape
    );
    assert!(matches!(
        layout.wire_paths[&(connection.from, connection.to)].segments()[0],
        PathSegment::Cubic(_)
    ));
    let point = Pos2::new(350.0, 132.0);
    assert_eq!(widget.wire_near_point(point, &layout), Some(0));
    let context = egui::Context::default();
    context.begin_pass(egui::RawInput::default());
    draw_connections(
        &context.layer_painter(egui::LayerId::background()),
        &widget.graph,
        &widget.registry,
        &layout,
        |p| p,
        2.0,
        |_, _| WireEmphasis::Normal,
    );
    let mut output = context.end_pass();
    output.textures_delta.clear();
    let Shape::CubicBezier(curve) = &output.shapes.last().unwrap().shape else {
        panic!("fallback cubic")
    };
    assert_eq!(
        curve.stroke.color,
        egui::epaint::ColorMode::Solid(
            widget
                .registry
                .socket_display(
                    &widget.graph.nodes[&connection.from.node].outputs[connection.from.index]
                )
                .0
        )
    );
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 300.0;
    assert!(widget.build_layout(Pos2::ZERO).wire_failures.is_empty());
    widget.apply_knife_cut(&[point - Vec2::Y, point + Vec2::Y], &layout);
    assert!(widget.graph.connections.is_empty());
}

#[test]
fn splice_preview_excludes_only_the_candidate_and_routes_rebuild_after_insertion() {
    let (mut widget, connection, candidate) = scene();
    widget.graph.nodes.get_mut(&candidate).unwrap().pos.y = 120.0;
    widget.interaction_state = InteractionState::DraggingNode {
        node_id: candidate,
        offset: Vec2::ZERO,
        constraint: None,
    };
    let layout = widget.build_layout(Pos2::ZERO);
    assert_eq!(layout.routing_excluded, Some(candidate));
    let point = layout.nodes[&candidate].node_rect().center();
    assert_eq!(
        widget.compute_insert_candidate_wire(candidate, Some(point), &layout),
        Some((0, true))
    );
    widget.try_wire_insert_on_drop(candidate, Some(point));
    widget.interaction_state = InteractionState::Idle;
    let layout = widget.build_layout(Pos2::ZERO);
    assert_eq!(layout.routing_excluded, None);
    assert!(
        !layout
            .wire_paths
            .contains_key(&(connection.from, connection.to))
    );
    assert_eq!(layout.wire_paths.len(), 2);
    assert!(layout.wire_failures.is_empty());
}

#[test]
fn connected_drop_preserves_topology_undo_and_the_existing_route_snapshot() {
    for endpoint in 0..2 {
        for placing in [false, true] {
            let (mut widget, connection, _) = scene();
            let node = if endpoint == 0 {
                connection.from.node
            } else {
                connection.to.node
            };
            widget.graph.nodes.get_mut(&node).unwrap().selected = true;
            widget.interaction_state = if placing {
                InteractionState::PlacingNodes {
                    anchor_canvas: Pos2::ZERO,
                    just_entered: false,
                }
            } else {
                InteractionState::DraggingNode {
                    node_id: node,
                    offset: Vec2::ZERO,
                    constraint: None,
                }
            };
            let mut layout = widget.build_layout(Pos2::ZERO);
            let before = serde_json::to_value(&widget.graph).unwrap();
            let undo_count = widget.undo_stack.len();
            let point = layout.nodes[&node].node_rect().center();
            widget.try_wire_insert_on_drop(node, Some(point));
            assert_eq!(serde_json::to_value(&widget.graph).unwrap(), before);
            assert_eq!(widget.undo_stack.len(), undo_count);
            // An unnecessary excluded-node layout changes the exact-input key.
            // A hit here proves that even transient routing state was untouched.
            assert!(widget.routing_cache.borrow_mut().route_interactive(
                &mut layout,
                &widget.graph.connections,
                &RouteConfig::default(),
                widget.view.zoom,
                true,
            ));
        }
    }
}

#[test]
fn eligible_drop_uses_final_node_geometry_for_pointer_and_center_targets() {
    for placing in [false, true] {
        for pointer_present in [false, true] {
            let (mut widget, connection, candidate) = scene();
            widget.graph.nodes.get_mut(&candidate).unwrap().selected = true;
            widget.graph.nodes.get_mut(&candidate).unwrap().pos.y = 300.0;
            widget.interaction_state = if placing {
                InteractionState::PlacingNodes {
                    anchor_canvas: Pos2::ZERO,
                    just_entered: false,
                }
            } else {
                InteractionState::DraggingNode {
                    node_id: candidate,
                    offset: Vec2::ZERO,
                    constraint: None,
                }
            };
            let stale = widget.build_layout(Pos2::ZERO);
            assert!(
                widget
                    .compute_insert_candidate_wire(candidate, None, &stale)
                    .is_none()
            );
            // Simulate the final input event moving the node onto the wire.
            widget.graph.nodes.get_mut(&candidate).unwrap().pos.y = 120.0;
            widget.try_wire_insert_on_drop(
                candidate,
                pointer_present.then_some(Pos2::new(262.0, 132.0)),
            );
            assert_eq!(widget.graph.connections.len(), 2);
            assert!(
                widget
                    .graph
                    .connections
                    .iter()
                    .any(|c| c.from == connection.from && c.to.node == candidate)
            );
            assert!(
                widget
                    .graph
                    .connections
                    .iter()
                    .any(|c| c.from.node == candidate && c.to == connection.to)
            );
            widget.interaction_state = InteractionState::Idle;
            let layout = widget.build_layout(Pos2::ZERO);
            assert_eq!(layout.wire_paths.len(), 2);
            assert!(layout.wire_failures.is_empty());
        }
    }
}

#[test]
fn failed_connections_show_a_warning_and_diagnostics_do_not_mutate_the_document() {
    let (mut widget, _, obstacle) = scene();
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.x = 68.0;
    let before = serde_json::to_value(&widget.graph).unwrap();
    widget.routing_debug.open = true;
    widget.routing_debug.obstacles = true;
    widget.routing_debug.escapes = true;
    widget.routing_debug.results = true;
    let context = egui::Context::default();
    context.begin_pass(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(800.0, 600.0),
        )),
        ..Default::default()
    });
    let mut ui = egui::Ui::new(
        context.clone(),
        egui::Id::new("routing-ui-test"),
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(800.0, 600.0),
        )),
    );
    widget.show(&mut ui);
    let mut output = context.end_pass();
    output.textures_delta.clear();
    assert!(output.shapes.iter().any(|clipped| matches!(&clipped.shape, Shape::Text(text) if text.galley.text().contains("could not be routed"))));
    assert_eq!(before, serde_json::to_value(&widget.graph).unwrap());
}

#[test]
fn placement_and_reroute_branching_keep_existing_topology_and_fresh_socket_keys() {
    let (mut widget, connection, candidate) = scene();
    widget.graph.nodes.get_mut(&candidate).unwrap().selected = true;
    widget.interaction_state = InteractionState::PlacingNodes {
        anchor_canvas: Pos2::ZERO,
        just_entered: false,
    };
    assert_eq!(
        widget.build_layout(Pos2::ZERO).routing_excluded,
        Some(candidate)
    );
    widget.graph.add_connection(
        SocketId {
            node: candidate,
            index: 0,
            direction: SocketDirection::Output,
        },
        connection.to,
    );
    assert_eq!(widget.build_layout(Pos2::ZERO).routing_excluded, None);
    widget.interaction_state = InteractionState::Idle;
    let new_target = widget
        .add_node_at("Reroute", Pos2::new(500.0, 300.0))
        .unwrap();
    widget.graph.add_connection(
        SocketId {
            node: candidate,
            index: 0,
            direction: SocketDirection::Output,
        },
        SocketId {
            node: new_target,
            index: 0,
            direction: SocketDirection::Input,
        },
    );
    let layout = widget.build_layout(Pos2::ZERO);
    assert_eq!(layout.wire_paths.len(), widget.graph.connections.len());
    assert!(
        !layout
            .wire_paths
            .contains_key(&(connection.from, connection.to))
    );
    assert!(layout.wire_failures.is_empty());
}

#[test]
fn collapsed_and_resized_nodes_use_live_geometry_and_invalid_endpoints_are_not_painted() {
    let (mut widget, connection, obstacle) = scene();
    widget
        .graph
        .nodes
        .get_mut(&connection.to.node)
        .unwrap()
        .pos
        .x = 900.0;
    let node = widget.graph.nodes.get_mut(&obstacle).unwrap();
    node.kind = NodeKind::Regular;
    node.title = "Wide obstacle with a changing header".to_owned();
    for collapsed in [false, true] {
        widget.graph.nodes.get_mut(&obstacle).unwrap().collapsed = collapsed;
        let layout = widget.build_layout(Pos2::ZERO);
        assert!(layout.wire_failures.is_empty());
        assert!(
            !layout.wire_paths[&(connection.from, connection.to)]
                .intersects_rect(layout.node_rects[&obstacle].expand2(Vec2::new(20.0, 16.0)))
        );
    }
    widget
        .graph
        .nodes
        .get_mut(&connection.from.node)
        .unwrap()
        .pos
        .x = f32::NAN;
    let layout = widget.build_layout(Pos2::ZERO);
    assert_eq!(
        layout.wire_failures[&(connection.from, connection.to)],
        RouteFailure::InvalidGeometry
    );
    assert!(
        !layout
            .wire_paths
            .contains_key(&(connection.from, connection.to))
    );
}

/// Portable paint fixtures also emit an SVG when run with `--nocapture` for inspection.
#[test]
fn routing_visual_fixtures_have_expected_path_classes() {
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1200' height='1380' viewBox='0 0 1200 1380'><rect width='1200' height='1380' fill='#1c1c1c'/>",
    );
    for (index, (zoom, failed)) in [(0.5, false), (1.0, false), (1.7, false), (1.0, true)]
        .into_iter()
        .enumerate()
    {
        let (mut widget, _, obstacle) = scene();
        if failed {
            widget.graph.nodes.get_mut(&obstacle).unwrap().pos.x = 68.0;
        }
        widget.view.zoom = zoom;
        let layout = widget.build_layout(Pos2::ZERO);
        assert_eq!(layout.wire_failures.is_empty(), !failed);
        let context = egui::Context::default();
        context.begin_pass(egui::RawInput::default());
        draw_connections(
            &context.layer_painter(egui::LayerId::background()),
            &widget.graph,
            &widget.registry,
            &layout,
            |p| p,
            2.0,
            |_, _| WireEmphasis::Normal,
        );
        let mut output = context.end_pass();
        output.textures_delta.clear();
        svg.push_str(&format!("<g transform='translate(30,{})'><text x='0' y='20' fill='white' font-size='16'>{}: {}x</text><g transform='translate(0,30) scale({zoom})'>", index * 340, if failed { "Blocked escape — diagnostic fallback" } else { "Checked individual route" }, zoom));
        for shape in &output.shapes {
            match &shape.shape {
                Shape::LineSegment { points, stroke } => svg.push_str(&format!("<path d='M {} {} L {} {}' fill='none' stroke='#{:02x}{:02x}{:02x}' stroke-width='{}'/>", points[0].x, points[0].y, points[1].x, points[1].y, stroke.color.r(), stroke.color.g(), stroke.color.b(), stroke.width)),
                Shape::CubicBezier(curve) => {
                    let p = curve.points;
                    let egui::epaint::ColorMode::Solid(color) = curve.stroke.color else { panic!("solid wire fixture") };
                    svg.push_str(&format!("<path d='M {} {} C {} {},{} {},{} {}' fill='none' stroke='#{:02x}{:02x}{:02x}' stroke-width='{}'/>", p[0].x,p[0].y,p[1].x,p[1].y,p[2].x,p[2].y,p[3].x,p[3].y,color.r(),color.g(),color.b(),curve.stroke.width));
                }
                _ => {}
            }
        }
        for body in layout.node_rects.values() {
            svg.push_str(&format!(
                "<rect x='{}' y='{}' width='{}' height='{}' fill='#40454d' stroke='#88909b'/>",
                body.min.x,
                body.min.y,
                body.width(),
                body.height()
            ));
        }
        svg.push_str("</g></g>");
    }
    svg.push_str("</svg>");
    println!("{svg}");
}
