//! Adapts a complete widget layout into identity-free routing geometry.

use std::collections::HashMap;

use egui::{Pos2, Rect};

use super::interaction_state::InteractionState;
use super::layout::GraphWidgetLayout;
use super::routing::{
    BundleCandidate, BundleMember, PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput,
    WirePath, WorkBudget, compatible_groups, improve_route, route_quality_bundle,
    route_with_budget, separate_route, shares_run,
};
use super::widget::NodeGraphWidget;
use super::wire::node_has_any_connection;
use crate::model::{Connection, NodeId, SocketDirection, SocketId};

impl NodeGraphWidget {
    pub(crate) fn routing_splice_candidate(&self) -> Option<NodeId> {
        let id = match self.interaction_state {
            InteractionState::DraggingNode { node_id, .. } => node_id,
            InteractionState::PlacingNodes { .. } => {
                let mut selected = self.graph.nodes.values().filter(|node| node.selected);
                let id = selected.next()?.id;
                if selected.next().is_some() {
                    return None;
                }
                id
            }
            _ => return None,
        };
        (!node_has_any_connection(&self.graph.connections, id)).then_some(id)
    }
}

impl GraphWidgetLayout {
    /// Direct curves share the painting/interaction snapshot but never run the
    /// obstacle solver or classify ordinary classic curves as routing failures.
    pub(crate) fn rebuild_classic_routes(&mut self, connections: &[Connection], zoom: f32) {
        self.wire_paths.clear();
        self.wire_failures.clear();
        if !zoom.is_finite() || zoom <= 0.0 {
            return;
        }
        for connection in connections {
            let from = self
                .nodes
                .get(&connection.from.node)
                .and_then(|n| n.output_socket_pos(connection.from.index));
            let to = self
                .nodes
                .get(&connection.to.node)
                .and_then(|n| n.input_socket_pos(connection.to.index));
            if let (Some(from), Some(to)) = (from, to)
                && from.is_finite()
                && to.is_finite()
                && (to - from).is_finite()
            {
                let path = WirePath::legacy(from, to, zoom);
                if path.bounds().min.is_finite() && path.bounds().max.is_finite() {
                    self.wire_paths
                        .insert((connection.from, connection.to), path);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn rebuild_routes(&mut self, connections: &[Connection], zoom: f32) {
        self.rebuild_routes_with_config(connections, &RouteConfig::default(), zoom);
    }

    pub(crate) fn rebuild_routes_with_config(
        &mut self,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
    ) {
        self.rebuild_routes_retaining(connections, config, zoom, &HashMap::new());
    }

    /// Retained entries are checked, immutable, and complete for each node pair.
    pub(crate) fn rebuild_routes_retaining(
        &mut self,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
        retained: &HashMap<(SocketId, SocketId), WirePath>,
    ) {
        self.wire_paths.clear();
        self.wire_failures.clear();
        let mut budget = WorkBudget::new(config.max_work);
        let mut smoothing_budget = WorkBudget::new(config.max_smoothing_work);
        let mut groups = self.connection_groups(connections, config.max_work);
        groups.reverse();
        while let Some(group) = groups.pop() {
            if group.iter().all(|c| retained.contains_key(&(c.from, c.to))) {
                for connection in group {
                    let key = (connection.from, connection.to);
                    self.wire_paths.insert(key, retained[&key].clone());
                }
                continue;
            }
            if group.len() > 1 {
                let result =
                    self.routing_geometry(&group, &mut budget)
                        .and_then(|(nodes, members)| {
                            route_quality_bundle(
                                &nodes,
                                &members,
                                config,
                                zoom,
                                &mut budget,
                                &mut smoothing_budget,
                            )
                        });
                if let Ok(paths) = result {
                    for (connection, path) in group.into_iter().zip(paths) {
                        self.wire_paths
                            .insert((connection.from, connection.to), path);
                    }
                } else {
                    // Retry contiguous halves in stable order, eventually using the
                    // individual visibility search (and its explicit failure presentation).
                    let middle = group.len() / 2;
                    groups.push(group[middle..].to_vec());
                    groups.push(group[..middle].to_vec());
                }
                continue;
            }
            let connection = group[0];
            let key = (connection.from, connection.to);
            match self.route_connection_with_budget(connection, config, &mut budget) {
                Ok(path) => {
                    // Optional quality work never spends the checked-search budget.
                    let path = self.smooth_connection(
                        connection,
                        path,
                        config,
                        zoom,
                        &mut smoothing_budget,
                    );
                    self.wire_paths.insert(key, path);
                }
                Err(failure) => {
                    self.wire_failures.insert(key, failure);
                    let from = self
                        .nodes
                        .get(&connection.from.node)
                        .and_then(|n| n.output_socket_pos(connection.from.index));
                    let to = self
                        .nodes
                        .get(&connection.to.node)
                        .and_then(|n| n.input_socket_pos(connection.to.index));
                    if let (Some(from), Some(to)) = (from, to)
                        && from.is_finite()
                        && to.is_finite()
                        && (to - from).is_finite()
                    {
                        let path = WirePath::legacy(from, to, zoom);
                        if path.bounds().min.is_finite() && path.bounds().max.is_finite() {
                            self.wire_paths.insert(key, path);
                        }
                    }
                }
            }
        }
        self.separate_connections(connections, config, zoom);
    }

    /// A separate bounded pass also rechecks retained drag paths against newly
    /// routed signals. Stable source identity, never input/hash order, owns a run.
    fn separate_connections(
        &mut self,
        connections: &[Connection],
        config: &RouteConfig,
        zoom: f32,
    ) {
        let mut ordered: Vec<_> = connections.iter().collect();
        ordered.sort_by_key(|c| (c.from.node.0, c.from.index, c.to.node.0, c.to.index));
        let groups: HashMap<_, _> = self
            .connection_groups(connections, config.max_work)
            .into_iter()
            .enumerate()
            .flat_map(|(i, group)| group.into_iter().map(move |c| ((c.from, c.to), i)))
            .collect();
        let mut budget = WorkBudget::new(config.max_work);
        for (index, connection) in ordered.iter().enumerate() {
            let key = (connection.from, connection.to);
            if self.wire_failures.contains_key(&key) {
                continue;
            }
            let Some(path) = self.wire_paths.get(&key) else {
                continue;
            };
            // A hard separation retry must not consume the proof allowance for
            // unrelated, already checked paths later in the stable order.
            let result = budget.with_share(ordered.len() - index, |budget| {
                let mut conflict = false;
                for previous in &ordered[..index] {
                    if previous.from == connection.from {
                        continue;
                    }
                    let previous_key = (previous.from, previous.to);
                    if !self.wire_failures.contains_key(&previous_key)
                        && let Some(other) = self.wire_paths.get(&previous_key)
                        && shares_run(path, other, budget)?
                    {
                        conflict = true;
                        break;
                    }
                }
                if !conflict {
                    return Ok(None);
                }
                let others: Vec<_> = ordered
                    .iter()
                    .enumerate()
                    .filter_map(|(i, other)| {
                        let other_key = (other.from, other.to);
                        let protected = groups[&key] == groups[&other_key];
                        (other.from != connection.from
                            && (i < index || protected)
                            && !self.wire_failures.contains_key(&other_key))
                        .then(|| self.wire_paths.get(&other_key).map(|p| (p, protected)))
                        .flatten()
                    })
                    .collect();
                let (nodes, members) = self.routing_geometry(&[connection], budget)?;
                separate_route(
                    RouteInput {
                        nodes: &nodes,
                        source: members[0].source,
                        target: members[0].target,
                    },
                    &others,
                    config,
                    zoom,
                    budget,
                )
                .map(Some)
            });
            match result {
                Ok(Some(path)) => {
                    self.wire_paths.insert(key, path);
                }
                Ok(None) => {}
                Err(failure) => {
                    // A failed separation is never presented as a checked merged
                    // signal. Keep the existing visible diagnostic/fallback policy.
                    self.wire_failures.insert(key, failure);
                    let from =
                        self.nodes[&connection.from.node].output_socket_pos(connection.from.index);
                    let to = self.nodes[&connection.to.node].input_socket_pos(connection.to.index);
                    if let (Some(from), Some(to)) = (from, to) {
                        self.wire_paths
                            .insert(key, WirePath::legacy(from, to, zoom));
                    }
                }
            }
        }
    }

    fn smooth_connection(
        &self,
        connection: &Connection,
        path: WirePath,
        config: &RouteConfig,
        zoom: f32,
        budget: &mut WorkBudget,
    ) -> WirePath {
        let Ok((nodes, mut members)) = self.routing_geometry(&[connection], budget) else {
            return path;
        };
        let Some(member) = members.pop() else {
            return path;
        };
        improve_route(
            RouteInput {
                nodes: &nodes,
                source: member.source,
                target: member.target,
            },
            path,
            config,
            zoom,
            budget,
        )
    }

    fn connection_groups<'a>(
        &self,
        connections: &'a [Connection],
        max_comparisons: usize,
    ) -> Vec<Vec<&'a Connection>> {
        let mut ordered: Vec<_> = connections.iter().collect();
        ordered.sort_by_key(|c| (c.from.node.0, c.to.node.0, c.from.index, c.to.index));
        let mut result = Vec::new();
        for pair in ordered.chunk_by(|a, b| a.from.node == b.from.node && a.to.node == b.to.node) {
            let candidates: Vec<_> = pair
                .iter()
                .map(|connection| {
                    let positions = (connection.from.direction == SocketDirection::Output
                        && connection.to.direction == SocketDirection::Input)
                        .then(|| {
                            Some((
                                self.nodes
                                    .get(&connection.from.node)?
                                    .output_socket_pos(connection.from.index)?,
                                self.nodes
                                    .get(&connection.to.node)?
                                    .input_socket_pos(connection.to.index)?,
                            ))
                        })
                        .flatten();
                    let (source, target) = positions
                        .unwrap_or((Pos2::new(f32::NAN, f32::NAN), Pos2::new(f32::NAN, f32::NAN)));
                    BundleCandidate {
                        source,
                        target,
                        source_socket: connection.from.index,
                        target_socket: connection.to.index,
                    }
                })
                .collect();
            // Divide one pass-wide grouping allowance between pairs. Unused capacity
            // is not transferred, keeping partitioning independent of pair traversal.
            let pair_comparisons = (max_comparisons / connections.len().max(1)) * pair.len();
            result.extend(
                compatible_groups(&candidates, pair_comparisons)
                    .into_iter()
                    .map(|group| group.into_iter().map(|index| pair[index]).collect()),
            );
        }
        result
    }

    /// No viewport culling: even offscreen bodies obstruct a connection. Frames do not.
    #[cfg(test)]
    pub(crate) fn route_connection(
        &self,
        connection: &Connection,
        config: &RouteConfig,
    ) -> Result<WirePath, RouteFailure> {
        self.route_connection_with_budget(connection, config, &mut WorkBudget::new(config.max_work))
    }

    fn route_connection_with_budget(
        &self,
        connection: &Connection,
        config: &RouteConfig,
        budget: &mut WorkBudget,
    ) -> Result<WirePath, RouteFailure> {
        let (nodes, mut members) = self.routing_geometry(&[connection], budget)?;
        let member = members.pop().ok_or(RouteFailure::InvalidGeometry)?;
        route_with_budget(
            RouteInput {
                nodes: &nodes,
                source: member.source,
                target: member.target,
            },
            config,
            budget,
        )
    }

    fn routing_geometry(
        &self,
        connections: &[&Connection],
        budget: &mut WorkBudget,
    ) -> Result<(Vec<Rect>, Vec<BundleMember>), RouteFailure> {
        let connection = connections.first().ok_or(RouteFailure::InvalidGeometry)?;
        if connections.iter().any(|c| {
            c.from.direction != SocketDirection::Output
                || c.to.direction != SocketDirection::Input
                || c.from.node != connection.from.node
                || c.to.node != connection.to.node
        }) {
            return Err(RouteFailure::InvalidGeometry);
        }
        budget.spend(self.node_rects.len())?;
        let mut ids: Vec<_> = self
            .node_rects
            .keys()
            .copied()
            .filter(|id| {
                Some(*id) != self.routing_excluded
                    || *id == connection.from.node
                    || *id == connection.to.node
            })
            .collect();
        ids.sort_unstable_by_key(|id| id.0);
        let nodes: Vec<_> = ids.iter().map(|id| self.node_rects[id]).collect();
        let source_obstacle = ids
            .binary_search_by_key(&connection.from.node.0, |id| id.0)
            .map_err(|_| RouteFailure::InvalidGeometry)?;
        let target_obstacle = ids
            .binary_search_by_key(&connection.to.node.0, |id| id.0)
            .map_err(|_| RouteFailure::InvalidGeometry)?;
        budget.spend(connections.len())?;
        let members = connections
            .iter()
            .map(|connection| {
                let source = PortGeometry {
                    obstacle: source_obstacle,
                    position: self
                        .nodes
                        .get(&connection.from.node)
                        .and_then(|node| node.output_socket_pos(connection.from.index))
                        .ok_or(RouteFailure::InvalidGeometry)?,
                    side: PortSide::Right,
                };
                let target = PortGeometry {
                    obstacle: target_obstacle,
                    position: self
                        .nodes
                        .get(&connection.to.node)
                        .and_then(|node| node.input_socket_pos(connection.to.index))
                        .ok_or(RouteFailure::InvalidGeometry)?,
                    side: PortSide::Left,
                };
                Ok(BundleMember {
                    source,
                    target,
                    source_socket: connection.from.index,
                    target_socket: connection.to.index,
                })
            })
            .collect::<Result<Vec<_>, RouteFailure>>()?;
        Ok((nodes, members))
    }
}

#[cfg(test)]
mod routing_input_tests {
    use egui::{Pos2, Rect};

    use super::super::widget::NodeGraphWidget;
    use super::*;
    use crate::model::{FrameId, NodeKind, SocketDirection, SocketId};
    use crate::runtime::NodeTypeRegistry;

    #[test]
    fn moving_one_fanout_source_does_not_fail_remote_connections() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let specs = [
            (40.0, 650.0, 1, 12),
            (370.0, 205.0, 4, 3),
            (675.0, 185.0, 2, 3),
            (610.0, 475.0, 2, 3),
            (1045.0, 225.0, 1, 1),
            (920.0, 400.0, 2, 1),
            (1415.0, 240.0, 2, 1),
            (1275.0, 850.0, 3, 1),
            (1450.0, 420.0, 12, 1),
            (1950.0, 450.0, 2, 0),
            (1860.0, 675.0, 3, 2),
            (2230.0, 650.0, 2, 0),
            (1890.0, 80.0, 2, 0),
        ];
        let ids: Vec<_> = specs
            .into_iter()
            .map(|(x, y, inputs, outputs)| {
                let id = widget.add_node_at("Reroute", Pos2::new(x, y)).unwrap();
                let node = widget.graph.nodes.get_mut(&id).unwrap();
                node.kind = NodeKind::Regular;
                node.inputs = vec![node.inputs[0].clone(); inputs];
                node.outputs = vec![node.outputs[0].clone(); outputs];
                id
            })
            .collect();
        for (source, output, target, input) in [
            (0, 7, 1, 0),
            (0, 6, 1, 1),
            (0, 8, 1, 3),
            (1, 0, 2, 0),
            (1, 0, 3, 0),
            (2, 0, 5, 0),
            (3, 0, 5, 1),
            (2, 0, 4, 0),
            (4, 0, 6, 0),
            (6, 0, 9, 1),
            (0, 8, 7, 0),
            (5, 0, 7, 1),
            (7, 0, 8, 11),
            (0, 0, 8, 1),
            (0, 1, 8, 2),
            (0, 2, 8, 3),
            (0, 3, 8, 4),
            (0, 4, 8, 5),
            (0, 5, 8, 6),
            (0, 6, 8, 7),
            (0, 7, 8, 8),
            (8, 0, 9, 0),
            (8, 0, 10, 0),
            (6, 0, 10, 2),
            (10, 0, 11, 0),
            (10, 1, 11, 1),
            (0, 9, 8, 0),
            (0, 10, 10, 1),
            (1, 0, 12, 0),
        ] {
            widget.graph.connections.push(Connection {
                from: SocketId {
                    node: ids[source],
                    index: output,
                    direction: SocketDirection::Output,
                },
                to: SocketId {
                    node: ids[target],
                    index: input,
                    direction: SocketDirection::Input,
                },
            });
        }
        for zoom in [0.5, 1.0, 1.7] {
            widget.view.zoom = zoom;
            let config = RouteConfig::default();
            let mut cache = super::super::routing_cache::RoutingCache::default();
            let remote = widget
                .graph
                .connections
                .iter()
                .find(|c| c.from.node == ids[4])
                .unwrap();
            let remote_key = (remote.from, remote.to);
            let mut previous_remote: Option<WirePath> = None;
            for x in [40.0, 100.0, 160.0, 220.0, 40.0] {
                widget.graph.nodes.get_mut(&ids[0]).unwrap().pos.x = x;
                let mut layout = widget.build_layout(Pos2::ZERO);
                for dragging in [true, false] {
                    cache.route_interactive(
                        &mut layout,
                        &widget.graph.connections,
                        &config,
                        zoom,
                        dragging,
                    );
                    if dragging && let Some(previous) = &previous_remote {
                        assert_eq!(
                            layout.wire_paths[&remote_key].segments().as_ptr(),
                            previous.segments().as_ptr()
                        );
                    }
                    for c in &widget.graph.connections {
                        if c.from.node != ids[0] && c.to.node != ids[0] {
                            assert!(
                                !layout.wire_failures.contains_key(&(c.from, c.to)),
                                "source x={x}: {:?}",
                                layout.wire_failures
                            );
                        }
                    }
                }
                previous_remote = Some(layout.wire_paths[&remote_key].clone());
                if x == 40.0 {
                    assert!(layout.wire_failures.is_empty());
                }
                let expected = layout.wire_failures.clone();
                let reversed: Vec<_> = widget.graph.connections.iter().rev().cloned().collect();
                layout.rebuild_routes_with_config(&reversed, &config, zoom);
                assert_eq!(layout.wire_failures, expected);
            }
        }
    }

    #[test]
    fn independent_sources_do_not_share_the_target_approach_track() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let a = widget
            .add_node_at("Reroute", Pos2::new(0.0, 300.0))
            .unwrap();
        let b = widget
            .add_node_at("Reroute", Pos2::new(0.0, 600.0))
            .unwrap();
        let target = widget
            .add_node_at("Reroute", Pos2::new(600.0, 0.0))
            .unwrap();
        for id in [a, b, target] {
            let node = widget.graph.nodes.get_mut(&id).unwrap();
            node.kind = NodeKind::Regular;
            node.inputs = vec![node.inputs[0].clone(); 4];
            node.outputs = vec![node.outputs[0].clone(); 4];
        }
        let connection = |node, output, input| Connection {
            from: SocketId {
                node,
                index: output,
                direction: SocketDirection::Output,
            },
            to: SocketId {
                node: target,
                index: input,
                direction: SocketDirection::Input,
            },
        };
        // Node identity matters even when both sources use output index zero.
        for connections in [
            vec![connection(a, 0, 0), connection(b, 0, 3)],
            // Distinct outputs on one node, split into incompatible destination orders.
            vec![connection(a, 0, 3), connection(a, 3, 0)],
        ] {
            widget.graph.connections = connections.clone();
            let original = serde_json::to_value(&widget.graph).unwrap();
            for zoom in [0.5, 1.0, 1.7] {
                widget.view.zoom = zoom;
                let mut layout = widget.build_layout(Pos2::ZERO);
                let config = RouteConfig::default();
                let independent: Vec<_> = connections
                    .iter()
                    .map(|c| {
                        let path = layout.route_connection(c, &config).unwrap();
                        layout.smooth_connection(
                            c,
                            path,
                            &config,
                            zoom,
                            &mut WorkBudget::new(config.max_smoothing_work),
                        )
                    })
                    .collect();
                assert!(
                    shares_run(
                        &independent[0],
                        &independent[1],
                        &mut WorkBudget::new(config.max_work)
                    )
                    .unwrap(),
                    "fixture must reproduce the independently selected shared track"
                );
                assert!(
                    layout.wire_failures.is_empty(),
                    "{:?}",
                    layout.wire_failures
                );
                let paths: Vec<_> = connections
                    .iter()
                    .map(|c| layout.wire_paths[&(c.from, c.to)].clone())
                    .collect();
                assert!(
                    !shares_run(&paths[0], &paths[1], &mut WorkBudget::new(config.max_work))
                        .unwrap()
                );
                if zoom == 1.0 && connections[1].from.node == b {
                    separation_visual_fixture(&layout, &independent, &paths);
                }
                let before: Vec<_> = paths
                    .iter()
                    .map(|p| format!("{:?}", p.segments()))
                    .collect();
                let reversed: Vec<_> = connections.iter().rev().cloned().collect();
                layout.rebuild_routes(&reversed, zoom);
                assert!(layout.wire_failures.is_empty());
                for (c, expected) in connections.iter().zip(before) {
                    assert_eq!(
                        format!("{:?}", layout.wire_paths[&(c.from, c.to)].segments()),
                        expected
                    );
                }
                // Exact reuse and partial drag history cannot bypass separation.
                let mut cache = super::super::routing_cache::RoutingCache::default();
                assert!(!cache.route(&mut layout, &connections, &config, zoom));
                assert!(cache.route(&mut layout, &connections, &config, zoom));
                let old_position = widget.graph.nodes[&a].pos;
                widget.graph.nodes.get_mut(&a).unwrap().pos.y += 40.0;
                layout = widget.build_layout(Pos2::ZERO);
                assert!(!cache.route_interactive(&mut layout, &connections, &config, zoom, true));
                for dragging in [true, false] {
                    cache.route_interactive(&mut layout, &connections, &config, zoom, dragging);
                    assert!(layout.wire_failures.is_empty());
                    assert!(
                        !shares_run(
                            &layout.wire_paths[&(connections[0].from, connections[0].to)],
                            &layout.wire_paths[&(connections[1].from, connections[1].to)],
                            &mut WorkBudget::new(config.max_work),
                        )
                        .unwrap()
                    );
                }
                widget.graph.nodes.get_mut(&a).unwrap().pos = old_position;
            }
            assert_eq!(original, serde_json::to_value(&widget.graph).unwrap());
        }
    }

    #[test]
    fn same_output_fanout_can_keep_its_shared_segments() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let source = widget
            .add_node_at("Reroute", Pos2::new(0.0, 400.0))
            .unwrap();
        let a = widget
            .add_node_at("Reroute", Pos2::new(600.0, 0.0))
            .unwrap();
        let b = widget
            .add_node_at("Reroute", Pos2::new(600.0, 200.0))
            .unwrap();
        for target in [a, b] {
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
        let layout = widget.build_layout(Pos2::ZERO);
        assert!(layout.wire_failures.is_empty());
        let paths: Vec<_> = widget
            .graph
            .connections
            .iter()
            .map(|c| &layout.wire_paths[&(c.from, c.to)])
            .collect();
        assert!(shares_run(paths[0], paths[1], &mut WorkBudget::new(100_000)).unwrap());
    }

    fn separation_visual_fixture(
        layout: &GraphWidgetLayout,
        before: &[WirePath],
        after: &[WirePath],
    ) {
        let mut svg = String::from(
            "<svg xmlns='http://www.w3.org/2000/svg' width='1800' height='900' viewBox='0 0 1800 900'><rect width='1800' height='900' fill='#191919'/>",
        );
        for (column, (title, paths)) in [
            ("Independent routes", before),
            ("Source-separated routes", after),
        ]
        .into_iter()
        .enumerate()
        {
            svg.push_str(&format!("<g transform='translate({},55)'><text x='20' y='-20' fill='white' font-size='22'>{title}</text>", column * 900 + 20));
            for body in layout.node_rects.values() {
                svg.push_str(&format!("<rect x='{}' y='{}' width='{}' height='{}' rx='8' fill='#303030' stroke='#666'/>", body.min.x, body.min.y, body.width(), body.height()));
            }
            for (index, path) in paths.iter().enumerate() {
                for segment in path.segments() {
                    let data = match segment {
                        super::super::routing::PathSegment::Line(p) => {
                            format!("M {} {} L {} {}", p[0].x, p[0].y, p[1].x, p[1].y)
                        }
                        super::super::routing::PathSegment::Cubic(p) => format!(
                            "M {} {} C {} {},{} {},{} {}",
                            p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y, p[3].x, p[3].y
                        ),
                    };
                    svg.push_str(&format!(
                        "<path d='{data}' fill='none' stroke='#00d9bb' stroke-width='{}'/>",
                        if index == 0 { 6 } else { 3 }
                    ));
                }
            }
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        println!("{svg}");
    }

    #[test]
    fn editor_keeps_multi_turn_candidates_bundled_across_zoom_and_connection_order() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let source = widget.add_node_at("Reroute", Pos2::ZERO).unwrap();
        let target = widget
            .add_node_at("Reroute", Pos2::new(700.0, 0.0))
            .unwrap();
        for id in [source, target] {
            let node = widget.graph.nodes.get_mut(&id).unwrap();
            node.kind = NodeKind::Regular;
            node.inputs = vec![node.inputs[0].clone(); 3];
            node.outputs = vec![node.outputs[0].clone(); 3];
        }
        let mut connections: Vec<_> = (0..3)
            .map(|index| Connection {
                from: SocketId {
                    node: source,
                    index,
                    direction: SocketDirection::Output,
                },
                to: SocketId {
                    node: target,
                    index,
                    direction: SocketDirection::Input,
                },
            })
            .collect();
        let config = RouteConfig::default();
        let mut expected = Vec::new();
        for zoom in [0.5, 1.0, 1.7] {
            widget.view.zoom = zoom;
            let mut layout = widget.build_layout(Pos2::ZERO);
            let x = layout.node_rects[&source].max.x;
            let y = layout.nodes[&source].output_socket_pos(0).unwrap().y - 20.0;
            let rect =
                |dx, dy, w, h| Rect::from_min_size(Pos2::new(x + dx, y + dy), egui::vec2(w, h));
            // Snapshot-only bodies exercise the generic geometry adapter, with no
            // concrete feature definitions or persistent topology changes.
            for (i, body) in [
                rect(-1000.0, -1000.0, 3000.0, 920.0),
                rect(-1000.0, 180.0, 3000.0, 1000.0),
                rect(150.0, -100.0, 50.0, 170.0),
                rect(300.0, 80.0, 50.0, 120.0),
            ]
            .into_iter()
            .enumerate()
            {
                layout.node_rects.insert(NodeId(9000 + i as u32), body);
            }
            if expected.is_empty() {
                let refs: Vec<_> = connections.iter().collect();
                let (nodes, members) = layout
                    .routing_geometry(&refs, &mut WorkBudget::new(config.max_work))
                    .unwrap();
                expected = route_quality_bundle(
                    &nodes,
                    &members,
                    &config,
                    zoom,
                    &mut WorkBudget::new(config.max_work),
                    &mut WorkBudget::new(config.max_smoothing_work),
                )
                .unwrap();
            }
            connections.reverse();
            layout.rebuild_routes(&connections, zoom);
            assert!(layout.wire_failures.is_empty());
            for connection in &connections {
                assert_eq!(
                    format!(
                        "{:?}",
                        layout.wire_paths[&(connection.from, connection.to)].segments()
                    ),
                    format!("{:?}", expected[connection.from.index].segments())
                );
            }
        }
    }

    #[test]
    fn editor_uses_bundle_paths_and_splits_contiguous_halves_when_fans_do_not_fit() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let source = widget.add_node_at("Reroute", Pos2::ZERO).unwrap();
        let target = widget
            .add_node_at("Reroute", Pos2::new(600.0, 0.0))
            .unwrap();
        for id in [source, target] {
            let node = widget.graph.nodes.get_mut(&id).unwrap();
            node.kind = NodeKind::Regular;
            node.inputs = vec![node.inputs[0].clone(); 3];
            node.outputs = vec![node.outputs[0].clone(); 3];
        }
        let connections: Vec<_> = (0..3)
            .map(|index| Connection {
                from: SocketId {
                    node: source,
                    index,
                    direction: SocketDirection::Output,
                },
                to: SocketId {
                    node: target,
                    index,
                    direction: SocketDirection::Input,
                },
            })
            .collect();
        widget.graph.connections = connections.clone();
        let config = RouteConfig::default();
        let mut layout = widget.build_layout(Pos2::ZERO);
        for split in [false, true] {
            if split {
                // The escape gap is 60: two 24-unit fans fit, two 32-unit fans do not.
                widget.graph.nodes.get_mut(&target).unwrap().pos.x =
                    layout.node_rects[&source].max.x + 120.0;
            }
            let before = serde_json::to_value(&widget.graph).unwrap();
            let mut expected = Vec::new();
            layout = widget.build_layout(Pos2::ZERO);
            let bundle_connections = if split {
                let path = layout.route_connection(&connections[0], &config).unwrap();
                expected.push(layout.smooth_connection(
                    &connections[0],
                    path,
                    &config,
                    1.0,
                    &mut WorkBudget::new(config.max_smoothing_work),
                ));
                &connections[1..]
            } else {
                &connections[..]
            };
            let (nodes, members) = layout
                .routing_geometry(
                    &bundle_connections.iter().collect::<Vec<_>>(),
                    &mut WorkBudget::new(config.max_work),
                )
                .unwrap();
            expected.extend(
                route_quality_bundle(
                    &nodes,
                    &members,
                    &config,
                    1.0,
                    &mut WorkBudget::new(config.max_work),
                    &mut WorkBudget::new(config.max_smoothing_work),
                )
                .unwrap(),
            );
            for zoom in [0.5, 1.0, 1.7] {
                widget.view.zoom = zoom;
                layout = widget.build_layout(Pos2::ZERO);
                assert!(layout.wire_failures.is_empty());
                for (connection, expected) in connections.iter().zip(&expected) {
                    assert_eq!(
                        format!(
                            "{:?}",
                            layout.wire_paths[&(connection.from, connection.to)].segments()
                        ),
                        format!("{:?}", expected.segments())
                    );
                }
            }
            assert_eq!(before, serde_json::to_value(&widget.graph).unwrap());
        }
    }

    #[test]
    fn layout_groups_only_same_node_pairs_and_preserves_checked_paths_across_permutations() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let source = widget.add_node_at("Reroute", Pos2::ZERO).unwrap();
        let target = widget
            .add_node_at("Reroute", Pos2::new(600.0, 0.0))
            .unwrap();
        let other = widget
            .add_node_at("Reroute", Pos2::new(600.0, 400.0))
            .unwrap();
        for id in [source, target] {
            let node = widget.graph.nodes.get_mut(&id).unwrap();
            node.kind = NodeKind::Regular;
            node.inputs = vec![node.inputs[0].clone(); 3];
            node.outputs = vec![node.outputs[0].clone(); 3];
        }
        let connection = |from_index, to_node, to_index| Connection {
            from: SocketId {
                node: source,
                index: from_index,
                direction: SocketDirection::Output,
            },
            to: SocketId {
                node: to_node,
                index: to_index,
                direction: SocketDirection::Input,
            },
        };
        let mut connections = vec![
            connection(0, target, 2),
            connection(0, target, 0),
            connection(1, target, 1),
            connection(2, other, 0),
        ];
        let mut layout = widget.build_layout(Pos2::ZERO);
        let keys = |groups: Vec<Vec<&Connection>>| {
            groups
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|c| (c.from, c.to))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let expected = keys(layout.connection_groups(&connections, 100));
        assert_eq!(
            expected.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
        assert_eq!(
            expected[0],
            vec![
                (connections[1].from, connections[1].to),
                (connections[0].from, connections[0].to)
            ]
        );
        layout.rebuild_routes(&connections, 1.0);
        assert!(layout.wire_failures.is_empty());
        let paths: Vec<_> = connections
            .iter()
            .map(|c| format!("{:?}", layout.wire_paths[&(c.from, c.to)].segments()))
            .collect();
        connections.reverse();
        assert_eq!(keys(layout.connection_groups(&connections, 100)), expected);
        layout.rebuild_routes(&connections, 1.0);
        assert!(layout.wire_failures.is_empty());
        for (connection, path) in connections.iter().rev().zip(paths) {
            assert_eq!(
                format!(
                    "{:?}",
                    layout.wire_paths[&(connection.from, connection.to)].segments()
                ),
                path
            );
        }
        let mut invalid = connection(0, target, 0);
        invalid.from.direction = SocketDirection::Input;
        let invalid_key = (invalid.from, invalid.to);
        connections.push(invalid);
        layout.rebuild_routes(&connections, 1.0);
        assert_eq!(
            layout.wire_failures[&invalid_key],
            RouteFailure::InvalidGeometry
        );
    }

    #[test]
    fn adaptation_includes_offscreen_nodes_excludes_frames_and_activates_checked_paths() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let a = widget
            .add_node_at("Reroute", Pos2::new(2000.0, 0.0))
            .unwrap();
        let b = widget
            .add_node_at("Reroute", Pos2::new(2400.0, 0.0))
            .unwrap();
        let block = widget
            .add_node_at("Reroute", Pos2::new(2200.0, 0.0))
            .unwrap();
        let connection = Connection {
            from: SocketId {
                node: a,
                index: 0,
                direction: SocketDirection::Output,
            },
            to: SocketId {
                node: b,
                index: 0,
                direction: SocketDirection::Input,
            },
        };
        widget.graph.add_connection(connection.from, connection.to);
        let before = serde_json::to_value(&widget.graph).unwrap();
        let mut layout = widget.build_layout(Pos2::ZERO);
        // Frame geometry can cover everything without becoming a node obstacle.
        layout.frame_rects.insert(FrameId(999), Rect::EVERYTHING);
        let path = layout
            .route_connection(&connection, &RouteConfig::default())
            .unwrap();
        let obstacle = layout.node_rects[&block].expand2(egui::vec2(20.0, 16.0));
        assert!(!path.intersects_rect(obstacle));
        assert!(!layout.wire_paths[&(connection.from, connection.to)].intersects_rect(obstacle));
        assert_eq!(before, serde_json::to_value(&widget.graph).unwrap());
        // A frame-shaped rectangle is excluded; the same rectangle as a body blocks escapes.
        layout.node_rects.insert(
            block,
            Rect::from_min_max(Pos2::new(1900.0, -100.0), Pos2::new(2500.0, 100.0)),
        );
        assert!(matches!(
            layout.route_connection(&connection, &RouteConfig::default()),
            Err(RouteFailure::BlockedEscape)
        ));
    }
}
