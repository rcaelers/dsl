use egui::{Pos2, Rect, Vec2};

use super::interaction_state::InteractionState;
use super::layout::GraphWidgetLayout;
use super::routing::{PathSegment, RouteConfig, RouteFailure};
use super::routing_cache::RoutingCache;
use super::widget::{ConnectionRouting, GraphUiPrefs, NodeGraphWidget};
use crate::model::{Connection, FrameId, NodeId, NodeKind, SocketDirection, SocketId};
use crate::runtime::NodeTypeRegistry;
use crate::widget::node::NodeWidget;

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

#[test]
fn classic_preference_restores_direct_curves_and_switching_resets_routing() {
    let legacy: GraphUiPrefs = serde_json::from_value(serde_json::json!({
        "panel_width": 280.0, "panel_tab": null, "minimap_visible": true,
    }))
    .unwrap();
    assert_eq!(
        legacy.connection_routing,
        ConnectionRouting::ObstacleAvoiding
    );
    for zoom in [0.25, 1.0, 3.0] {
        for target_x in [-100.0, 500.0] {
            let (mut widget, connection, obstacle) = scene();
            widget.view.zoom = zoom;
            widget
                .graph
                .nodes
                .get_mut(&connection.to.node)
                .unwrap()
                .pos
                .x = target_x;
            widget.graph.nodes.get_mut(&obstacle).unwrap().pos =
                widget.graph.nodes[&connection.from.node].pos;
            let original = serde_json::to_value(&widget.graph).unwrap();
            let key = (connection.from, connection.to);
            assert!(
                widget
                    .build_layout(Pos2::ZERO)
                    .wire_failures
                    .contains_key(&key)
            );
            for mode in [
                ConnectionRouting::Classic,
                ConnectionRouting::ObstacleAvoiding,
                ConnectionRouting::Classic,
            ] {
                let mut prefs = widget.ui_prefs();
                prefs.connection_routing = mode;
                widget.set_ui_prefs(
                    serde_json::from_value(serde_json::to_value(prefs).unwrap()).unwrap(),
                );
                let layout = widget.build_layout(Pos2::ZERO);
                assert_eq!(widget.ui_prefs().connection_routing, mode);
                assert_eq!(serde_json::to_value(&widget.graph).unwrap(), original);
                if mode == ConnectionRouting::ObstacleAvoiding {
                    assert!(layout.wire_failures.contains_key(&key));
                    continue;
                }
                assert!(layout.wire_failures.is_empty());
                let [PathSegment::Cubic(points)] = layout.wire_paths[&key].segments() else {
                    panic!("classic cubic")
                };
                let from = layout.socket_screen_pos[&connection.from];
                let to = layout.socket_screen_pos[&connection.to];
                // Independent origin/main draw_wire formula in screen coordinates.
                let dx = (to.x - from.x).abs().max(50.0) * 0.5;
                let expected = [from, from + Vec2::new(dx, 0.0), to - Vec2::new(dx, 0.0), to];
                for (actual, expected) in points
                    .map(|p| widget.view.canvas_to_screen(Pos2::ZERO, p))
                    .into_iter()
                    .zip(expected)
                {
                    assert!(actual.distance(expected) < 0.001);
                }
                let a = points[0].lerp(points[1], 0.5);
                let b = points[1].lerp(points[2], 0.5);
                let c = points[2].lerp(points[3], 0.5);
                let midpoint = a.lerp(b, 0.5).lerp(b.lerp(c, 0.5), 0.5);
                assert_eq!(widget.wire_near_point(midpoint, &layout), Some(0));
            }
        }
    }
}

fn assert_cold_matches(
    layout: &mut GraphWidgetLayout,
    connections: &[Connection],
    config: &RouteConfig,
) {
    let expected = layout.wire_paths.clone();
    let failures = layout.wire_failures.clone();
    layout.rebuild_routes_with_config(connections, config, 1.0);
    assert_eq!(failures, layout.wire_failures);
    assert_eq!(expected.len(), layout.wire_paths.len());
    for (key, path) in expected {
        assert_eq!(
            format!("{:?}", path.segments()),
            format!("{:?}", layout.wire_paths[&key].segments())
        );
    }
}

#[test]
fn identical_inputs_and_pan_share_immutable_geometry_but_zoom_rebuilds() {
    let (mut widget, connection, _) = scene();
    let key = (connection.from, connection.to);
    let first = widget.build_layout(Pos2::ZERO);
    widget.view.pan = Vec2::new(200.0, -100.0);
    let panned = widget.build_layout(Pos2::new(25.0, 40.0));
    assert_eq!(
        first.wire_paths[&key].segments().as_ptr(),
        panned.wire_paths[&key].segments().as_ptr()
    );
    assert_ne!(
        first.node_screen_rects[&connection.from.node],
        panned.node_screen_rects[&connection.from.node]
    );
    widget.view.zoom = 1.7;
    let zoomed = widget.build_layout(Pos2::ZERO);
    assert_ne!(
        first.wire_paths[&key].segments().as_ptr(),
        zoomed.wire_paths[&key].segments().as_ptr()
    );
    assert_eq!(
        format!("{:?}", first.wire_paths[&key].segments()),
        format!("{:?}", zoomed.wire_paths[&key].segments())
    );
    assert_eq!(first.wire_failures, zoomed.wire_failures);
    // Replacing even an identical document discards transient history.
    widget.set_graph(widget.graph.clone());
    let loaded = widget.build_layout(Pos2::ZERO);
    assert_ne!(
        loaded.wire_paths[&key].segments().as_ptr(),
        zoomed.wire_paths[&key].segments().as_ptr()
    );
    assert!(
        !serde_json::to_string(&widget.graph)
            .unwrap()
            .contains("routing_cache")
    );
}

#[test]
fn all_obstacle_extents_and_provisional_exclusions_invalidate_the_snapshot() {
    let (widget, connection, obstacle) = scene();
    let connections = [connection];
    let config = RouteConfig::default();
    let mut layout = widget.build_layout(Pos2::ZERO);
    let mut cache = RoutingCache::default();
    assert!(!cache.route(&mut layout, &connections, &config, 1.0));
    assert!(cache.route(&mut layout, &connections, &config, 1.0));
    let original = layout.node_rects[&obstacle];
    for body in [
        original.translate(Vec2::new(0.0, 300.0)),
        original.expand(2.0),
        original,
    ] {
        layout.node_rects.insert(obstacle, body);
        assert!(!cache.route(&mut layout, &connections, &config, 1.0));
        assert_cold_matches(&mut layout, &connections, &config);
    }
    // An offscreen, disconnected node is still a routing dependency.
    let remote = NodeId(9000);
    layout.node_rects.insert(
        remote,
        Rect::from_min_size(Pos2::new(4000.0, 4000.0), Vec2::splat(50.0)),
    );
    assert!(!cache.route(&mut layout, &connections, &config, 1.0));
    layout.node_rects.remove(&remote);
    assert!(!cache.route(&mut layout, &connections, &config, 1.0));
    for excluded in [Some(obstacle), None] {
        layout.routing_excluded = excluded;
        assert!(!cache.route(&mut layout, &connections, &config, 1.0));
        assert_cold_matches(&mut layout, &connections, &config);
    }
}

#[test]
fn changed_socket_geometry_is_detected_even_when_body_rectangles_match() {
    let (widget, connection, _) = scene();
    let config = RouteConfig::default();
    let mut layout = widget.build_layout(Pos2::ZERO);
    let mut cache = RoutingCache::default();
    cache.route(&mut layout, std::slice::from_ref(&connection), &config, 1.0);
    let before = layout.wire_paths[&(connection.from, connection.to)].clone();
    let mut source = widget.graph.nodes[&connection.from.node].clone();
    source.pos.y += 2.0;
    // Deliberately retain the rectangle snapshot while changing the socket
    // layout, proving that bounds alone are not a sufficient cache key.
    layout.nodes.insert(
        connection.from.node,
        NodeWidget::new(&widget.graph, connection.from.node, &source, None),
    );
    assert!(!cache.route(&mut layout, std::slice::from_ref(&connection), &config, 1.0));
    assert_ne!(
        format!("{:?}", before.segments()),
        format!(
            "{:?}",
            layout.wire_paths[&(connection.from, connection.to)].segments()
        )
    );
    assert_cold_matches(&mut layout, std::slice::from_ref(&connection), &config);
}

#[test]
fn topology_configuration_and_failure_recovery_never_reuse_a_stale_safe_result() {
    let (widget, connection, _) = scene();
    let config = RouteConfig::default();
    let mut layout = widget.build_layout(Pos2::ZERO);
    let mut cache = RoutingCache::default();
    cache.route(&mut layout, std::slice::from_ref(&connection), &config, 1.0);
    assert!(layout.wire_failures.is_empty());
    let limited = RouteConfig {
        max_work: 0,
        ..config
    };
    assert!(!cache.route(
        &mut layout,
        std::slice::from_ref(&connection),
        &limited,
        1.0
    ));
    assert_eq!(
        layout.wire_failures[&(connection.from, connection.to)],
        RouteFailure::WorkLimit
    );
    assert!(cache.route(
        &mut layout,
        std::slice::from_ref(&connection),
        &limited,
        1.0
    ));
    assert_eq!(
        layout.wire_failures[&(connection.from, connection.to)],
        RouteFailure::WorkLimit
    );
    assert!(!cache.route(&mut layout, std::slice::from_ref(&connection), &config, 1.0));
    assert!(layout.wire_failures.is_empty());
    assert!(!cache.route(&mut layout, &[], &config, 1.0));
    assert!(layout.wire_paths.is_empty() && layout.wire_failures.is_empty());
    let mut invalid = connection.clone();
    invalid.to.index = 9000;
    assert!(!cache.route(&mut layout, &[invalid], &config, 1.0));
    assert!(layout.wire_paths.is_empty());
    assert_eq!(layout.wire_failures.len(), 1);
    assert!(!cache.route(&mut layout, std::slice::from_ref(&connection), &config, 1.0));
    assert!(layout.wire_failures.is_empty());
    assert_cold_matches(&mut layout, std::slice::from_ref(&connection), &config);
}

#[test]
fn moving_an_obstacle_into_an_escape_keeps_old_layouts_independent_and_rechecks() {
    let (mut widget, connection, obstacle) = scene();
    let key = (connection.from, connection.to);
    let safe = widget.build_layout(Pos2::ZERO);
    let original = format!("{:?}", safe.wire_paths[&key].segments());
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos =
        widget.graph.nodes[&connection.from.node].pos;
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.x += 28.0;
    let blocked = widget.build_layout(Pos2::ZERO);
    assert_eq!(blocked.wire_failures[&key], RouteFailure::BlockedEscape);
    assert_eq!(format!("{:?}", safe.wire_paths[&key].segments()), original);
    assert!(safe.wire_failures.is_empty());
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 400.0;
    let recovered = widget.build_layout(Pos2::ZERO);
    assert!(recovered.wire_failures.is_empty());
    assert_ne!(
        recovered.wire_paths[&key].segments().as_ptr(),
        blocked.wire_paths[&key].segments().as_ptr()
    );
}

#[test]
fn drag_history_keeps_a_valid_detour_and_release_discovers_the_open_corridor() {
    let (mut widget, connection, obstacle) = scene();
    let key = (connection.from, connection.to);
    let initial = widget.build_layout(Pos2::ZERO);
    let document = serde_json::to_string(&widget.graph).unwrap();
    // A frame gesture has no single-node provisional splice exclusion.
    widget.interaction_state = InteractionState::DraggingFrame {
        frame_id: FrameId(9000),
        last_canvas: Pos2::ZERO,
    };
    let started = widget.build_layout(Pos2::ZERO);
    assert_eq!(
        initial.wire_paths[&key].segments().as_ptr(),
        started.wire_paths[&key].segments().as_ptr()
    );
    assert_eq!(document, serde_json::to_string(&widget.graph).unwrap());
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 500.0;
    let moving = widget.build_layout(Pos2::ZERO);
    assert!(moving.wire_failures.is_empty());
    assert_eq!(
        initial.wire_paths[&key].segments().as_ptr(),
        moving.wire_paths[&key].segments().as_ptr()
    );
    widget.interaction_state = InteractionState::Idle;
    let mut released = widget.build_layout(Pos2::ZERO);
    assert_ne!(
        format!("{:?}", moving.wire_paths[&key].segments()),
        format!("{:?}", released.wire_paths[&key].segments())
    );
    assert_cold_matches(
        &mut released,
        &widget.graph.connections,
        &RouteConfig::default(),
    );
    let stationary = widget.build_layout(Pos2::ZERO);
    let repeated = widget.build_layout(Pos2::ZERO);
    assert_eq!(
        stationary.wire_paths[&key].segments().as_ptr(),
        repeated.wire_paths[&key].segments().as_ptr()
    );
}

#[test]
fn dragging_into_an_escape_rejects_history_and_recovers_from_the_warning() {
    let (widget, connection, obstacle) = scene();
    let key = (connection.from, connection.to);
    let mut layout = widget.build_layout(Pos2::ZERO);
    let mut cache = RoutingCache::default();
    let config = RouteConfig::default();
    cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
    let safe = layout.wire_paths[&key].clone();
    layout.node_rects.insert(
        obstacle,
        Rect::from_min_size(Pos2::new(68.0, 120.0), Vec2::splat(24.0)),
    );
    cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
    assert_eq!(layout.wire_failures[&key], RouteFailure::BlockedEscape);
    assert_ne!(
        safe.segments().as_ptr(),
        layout.wire_paths[&key].segments().as_ptr()
    );
    layout.node_rects.insert(
        obstacle,
        Rect::from_min_size(Pos2::new(250.0, 500.0), Vec2::splat(24.0)),
    );
    cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
    assert!(layout.wire_failures.is_empty());
    assert_cold_matches(&mut layout, &widget.graph.connections, &config);
}

#[test]
fn exhausted_history_work_rebuilds_instead_of_claiming_unchecked_reuse() {
    for allowance in [0, 4] {
        let (widget, connection, obstacle) = scene();
        let key = (connection.from, connection.to);
        let config = RouteConfig {
            max_history_work: allowance,
            ..RouteConfig::default()
        };
        let mut layout = widget.build_layout(Pos2::ZERO);
        let mut cache = RoutingCache::default();
        cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
        let before = layout.wire_paths[&key].clone();
        let body = layout.node_rects[&obstacle];
        layout
            .node_rects
            .insert(obstacle, body.translate(Vec2::new(0.0, 500.0)));
        cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
        assert_ne!(
            before.segments().as_ptr(),
            layout.wire_paths[&key].segments().as_ptr()
        );
        assert!(layout.wire_failures.is_empty());
        assert_cold_matches(&mut layout, &widget.graph.connections, &config);
    }
}

#[test]
fn changed_one_socket_invalidates_every_member_of_the_node_pair() {
    let (mut widget, connection, obstacle) = scene();
    widget.graph.nodes.get_mut(&obstacle).unwrap().pos.y = 500.0;
    for id in [connection.from.node, connection.to.node] {
        let node = widget.graph.nodes.get_mut(&id).unwrap();
        node.kind = NodeKind::Regular;
        node.inputs = vec![node.inputs[0].clone(); 3];
        node.outputs = vec![node.outputs[0].clone(); 3];
    }
    let mut second = connection.clone();
    second.from.index = 2;
    second.to.index = 2;
    widget.graph.add_connection(second.from, second.to);
    let mut layout = widget.build_layout(Pos2::ZERO);
    let config = RouteConfig::default();
    let mut cache = RoutingCache::default();
    cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
    assert!(layout.wire_failures.is_empty());
    let first = layout.wire_paths[&(connection.from, connection.to)].clone();
    let unchanged = layout.nodes[&connection.from.node].output_socket_pos(0);
    let changed = layout.nodes[&connection.from.node].output_socket_pos(2);
    let mut source = widget.graph.nodes[&connection.from.node].clone();
    source.outputs[1].hidden = true;
    layout.nodes.insert(
        connection.from.node,
        NodeWidget::new(&widget.graph, connection.from.node, &source, None),
    );
    assert_eq!(
        unchanged,
        layout.nodes[&connection.from.node].output_socket_pos(0)
    );
    assert_ne!(
        changed,
        layout.nodes[&connection.from.node].output_socket_pos(2)
    );
    cache.route_interactive(&mut layout, &widget.graph.connections, &config, 1.0, true);
    assert_ne!(
        first.segments().as_ptr(),
        layout.wire_paths[&(connection.from, connection.to)]
            .segments()
            .as_ptr()
    );
    assert_cold_matches(&mut layout, &widget.graph.connections, &config);
}

#[test]
fn drag_history_resets_on_endpoint_extents_topology_exclusion_config_and_zoom() {
    for case in 0..7 {
        let (widget, connection, obstacle) = scene();
        let key = (connection.from, connection.to);
        let mut layout = widget.build_layout(Pos2::ZERO);
        let mut connections = widget.graph.connections.clone();
        let mut config = RouteConfig::default();
        let mut zoom = 1.0;
        let mut cache = RoutingCache::default();
        cache.route_interactive(&mut layout, &connections, &config, zoom, true);
        let before = layout.wire_paths[&key].clone();
        match case {
            0 => {
                let body = layout.node_rects[&connection.from.node];
                layout.node_rects.insert(
                    connection.from.node,
                    Rect::from_min_max(body.min, body.max + Vec2::new(0.0, 2.0)),
                );
            }
            1 => connections.push(connection.clone()),
            2 => layout.routing_excluded = Some(obstacle),
            3 => config.corner_radius += 1.0,
            4 => zoom = 1.7,
            5 => {
                layout.node_rects.remove(&obstacle);
            }
            _ => {
                layout.node_rects.insert(
                    NodeId(9999),
                    Rect::from_min_size(Pos2::new(9000.0, 9000.0), Vec2::splat(24.0)),
                );
            }
        }
        cache.route_interactive(&mut layout, &connections, &config, zoom, true);
        assert_ne!(
            before.segments().as_ptr(),
            layout.wire_paths[&key].segments().as_ptr(),
            "case {case}"
        );
    }
}

#[test]
fn identical_drag_histories_have_identical_geometry_and_failures() {
    let sequence = || {
        let (widget, _, obstacle) = scene();
        let mut layout = widget.build_layout(Pos2::ZERO);
        let mut cache = RoutingCache::default();
        let mut results = Vec::new();
        for position in [
            Pos2::new(250.0, 110.0),
            Pos2::new(250.0, 500.0),
            Pos2::new(68.0, 120.0),
            Pos2::new(250.0, 500.0),
        ] {
            layout
                .node_rects
                .insert(obstacle, Rect::from_min_size(position, Vec2::splat(24.0)));
            cache.route_interactive(
                &mut layout,
                &widget.graph.connections,
                &RouteConfig::default(),
                1.0,
                true,
            );
            let c = &widget.graph.connections[0];
            results.push((
                format!("{:?}", layout.wire_paths[&(c.from, c.to)].segments()),
                layout.wire_failures.clone(),
            ));
        }
        results
    };
    assert_eq!(sequence(), sequence());
}

#[test]
fn moving_a_connected_node_rebuilds_incident_paths_but_shares_unrelated_geometry() {
    let (mut widget, connection, _) = scene();
    let from = widget
        .add_node_at("Reroute", Pos2::new(900.0, 500.0))
        .unwrap();
    let to = widget
        .add_node_at("Reroute", Pos2::new(1400.0, 500.0))
        .unwrap();
    let remote = Connection {
        from: SocketId {
            node: from,
            index: 0,
            direction: SocketDirection::Output,
        },
        to: SocketId {
            node: to,
            index: 0,
            direction: SocketDirection::Input,
        },
    };
    widget.graph.add_connection(remote.from, remote.to);
    let initial = widget.build_layout(Pos2::ZERO);
    assert!(initial.wire_failures.is_empty());
    widget.interaction_state = InteractionState::DraggingNode {
        node_id: from,
        offset: Vec2::ZERO,
        constraint: None,
    };
    widget.graph.nodes.get_mut(&from).unwrap().pos.y += 10.0;
    let moved = widget.build_layout(Pos2::ZERO);
    assert!(moved.wire_failures.is_empty());
    let key = (connection.from, connection.to);
    assert_eq!(
        initial.wire_paths[&key].segments().as_ptr(),
        moved.wire_paths[&key].segments().as_ptr()
    );
    assert_ne!(
        initial.wire_paths[&(remote.from, remote.to)]
            .segments()
            .as_ptr(),
        moved.wire_paths[&(remote.from, remote.to)]
            .segments()
            .as_ptr()
    );
    widget.view.pan = Vec2::new(200.0, 100.0);
    let panned = widget.build_layout(Pos2::new(30.0, 40.0));
    for (key, path) in &moved.wire_paths {
        assert_eq!(
            path.segments().as_ptr(),
            panned.wire_paths[key].segments().as_ptr()
        );
    }
}
