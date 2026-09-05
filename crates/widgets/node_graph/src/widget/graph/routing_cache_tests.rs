use egui::{Pos2, Rect, Vec2};

use super::layout::GraphWidgetLayout;
use super::routing::{RouteConfig, RouteFailure};
use super::routing_cache::RoutingCache;
use super::widget::NodeGraphWidget;
use crate::model::{Connection, NodeId, SocketDirection, SocketId};
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
