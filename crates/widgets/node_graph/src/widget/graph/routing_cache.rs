//! Transient exact-input reuse and checked, node-pair-atomic drag history.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect};

use super::layout::GraphWidgetLayout;
use super::routing::{RouteConfig, RouteFailure, WirePath, WorkBudget, avoids_changed_obstacles};
use crate::model::{Connection, NodeId, SocketId};

#[derive(Default)]
pub(crate) struct RoutingCache {
    snapshot: Option<CachedRoutes>,
}

struct CachedRoutes {
    key: RoutingKey,
    dragging: bool,
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
    #[cfg(test)]
    pub(crate) fn route(
        &mut self,
        layout: &mut GraphWidgetLayout,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
    ) -> bool {
        self.route_interactive(layout, connections, config, zoom, false)
    }

    /// Returns whether the complete snapshot was reused. During geometry gestures,
    /// unchanged node pairs can retain revalidated paths. Ending a gesture forces
    /// a full bounded quality rebuild even if the last geometry is identical.
    pub(crate) fn route_interactive(
        &mut self,
        layout: &mut GraphWidgetLayout,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
        dragging: bool,
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
        if let Some(snapshot) = &mut self.snapshot
            && snapshot.key == key
            && (!snapshot.dragging || dragging)
        {
            snapshot.dragging = dragging;
            layout.wire_paths.clone_from(&snapshot.paths);
            layout.wire_failures.clone_from(&snapshot.failures);
            return true;
        }
        // One entry only. Arc-backed immutable paths keep earlier live layouts
        // independent without copying their curve and interaction geometry.
        let retained = self.snapshot.take().map_or_else(HashMap::new, |previous| {
            if dragging {
                previous.revalidated_pairs(&key)
            } else {
                HashMap::new()
            }
        });
        if retained.is_empty() {
            layout.rebuild_routes_with_config(connections, config, zoom);
        } else {
            layout.rebuild_routes_retaining(connections, config, zoom, &retained);
        }
        self.snapshot = Some(CachedRoutes {
            key,
            dragging,
            paths: layout.wire_paths.clone(),
            failures: layout.wire_failures.clone(),
        });
        false
    }
}

impl CachedRoutes {
    fn revalidated_pairs(&self, next: &RoutingKey) -> HashMap<(SocketId, SocketId), WirePath> {
        let previous = &self.key;
        // Topology, exclusions, zoom and configuration changes reset history.
        if previous.config != next.config
            || previous.zoom != next.zoom
            || previous.excluded != next.excluded
            || previous.bodies.len() != next.bodies.len()
            || previous.connections.len() != next.connections.len()
            || previous
                .bodies
                .iter()
                .zip(&next.bodies)
                .any(|(a, b)| a.0 != b.0)
            || previous
                .connections
                .iter()
                .zip(&next.connections)
                .any(|(a, b)| a.from != b.from || a.to != b.to)
        {
            return HashMap::new();
        }
        let mut budget = WorkBudget::new(next.config.max_history_work);
        if budget
            .spend(next.bodies.len() + next.connections.len())
            .is_err()
        {
            return HashMap::new();
        }
        let mut changed_nodes = HashSet::new();
        let mut changes = Vec::new();
        for (&(id, old), &(_, new)) in previous.bodies.iter().zip(&next.bodies) {
            if old != new {
                changed_nodes.insert(id);
                // Excluded candidates cannot obstruct unrelated connections.
                if Some(id) != next.excluded {
                    changes.push((old, new));
                }
            }
        }
        let mut rejected_pairs = HashSet::new();
        let mut retained = HashMap::new();
        // Input order is part of the history key, so bounded proof is deterministic.
        for (old, new) in previous.connections.iter().zip(&next.connections) {
            let pair = (new.from.node, new.to.node);
            if rejected_pairs.contains(&pair) {
                continue;
            }
            let key = (new.from, new.to);
            let safe = old == new
                && !changed_nodes.contains(&pair.0)
                && !changed_nodes.contains(&pair.1)
                && !self.failures.contains_key(&key)
                && self.paths.get(&key).is_some_and(|path| {
                    avoids_changed_obstacles(path, &changes, &next.config, &mut budget) == Ok(true)
                });
            if safe {
                retained.insert(key, self.paths[&key].clone());
            } else {
                rejected_pairs.insert(pair);
            }
        }
        // Never mix a previously ordered bundle with independently rerouted members.
        retained.retain(|(from, to), _| !rejected_pairs.contains(&(from.node, to.node)));
        retained
    }
}
