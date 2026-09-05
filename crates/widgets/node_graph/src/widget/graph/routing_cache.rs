//! Single-snapshot reuse of identical routing inputs, never persisted in the document.

use std::collections::HashMap;

use egui::{Pos2, Rect};

use super::layout::GraphWidgetLayout;
use super::routing::{RouteConfig, RouteFailure, WirePath};
use crate::model::{Connection, NodeId, SocketId};

#[derive(Default)]
pub(crate) struct RoutingCache {
    snapshot: Option<CachedRoutes>,
}

struct CachedRoutes {
    key: RoutingKey,
    paths: HashMap<(SocketId, SocketId), WirePath>,
    failures: HashMap<(SocketId, SocketId), RouteFailure>,
}

#[derive(PartialEq)]
struct RoutingKey {
    bodies: Vec<(NodeId, Rect)>,
    connections: Vec<ConnectionGeometry>,
    excluded: Option<NodeId>,
    config: RouteConfig,
    zoom: f32,
}

#[derive(PartialEq)]
struct ConnectionGeometry {
    from: SocketId,
    to: SocketId,
    source: Option<Pos2>,
    target: Option<Pos2>,
}

impl RoutingCache {
    /// Returns whether the complete snapshot was reused. Any input mismatch
    /// invokes the checked router; failures retain their diagnostic classification.
    pub(crate) fn route(
        &mut self,
        layout: &mut GraphWidgetLayout,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
    ) -> bool {
        let mut bodies: Vec<_> = layout
            .node_rects
            .iter()
            .map(|(&id, &rect)| (id, rect))
            .collect();
        bodies.sort_unstable_by_key(|(id, _)| id.0);
        let key = RoutingKey {
            bodies,
            connections: connections
                .iter()
                .map(|connection| ConnectionGeometry {
                    from: connection.from,
                    to: connection.to,
                    source: layout
                        .nodes
                        .get(&connection.from.node)
                        .and_then(|node| node.output_socket_pos(connection.from.index)),
                    target: layout
                        .nodes
                        .get(&connection.to.node)
                        .and_then(|node| node.input_socket_pos(connection.to.index)),
                })
                .collect(),
            excluded: layout.routing_excluded,
            config: *config,
            zoom,
        };
        if let Some(snapshot) = &self.snapshot
            && snapshot.key == key
        {
            layout.wire_paths.clone_from(&snapshot.paths);
            layout.wire_failures.clone_from(&snapshot.failures);
            return true;
        }
        // One entry only. Arc-backed immutable paths keep earlier live layouts
        // independent without copying their curve and interaction geometry.
        self.snapshot = None;
        layout.rebuild_routes_with_config(connections, config, zoom);
        self.snapshot = Some(CachedRoutes {
            key,
            paths: layout.wire_paths.clone(),
            failures: layout.wire_failures.clone(),
        });
        false
    }
}
