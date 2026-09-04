//! Adapts a complete widget layout into identity-free routing geometry.

use super::interaction_state::InteractionState;
use super::layout::GraphWidgetLayout;
use super::routing::{
    PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WirePath, WorkBudget,
    route_with_budget,
};
use super::widget::NodeGraphWidget;
use super::wire::node_has_any_connection;
use crate::model::{Connection, NodeId, SocketDirection};

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
    pub(crate) fn rebuild_routes(&mut self, connections: &[Connection], zoom: f32) {
        self.wire_paths.clear();
        self.wire_failures.clear();
        let config = RouteConfig::default();
        let mut budget = WorkBudget::new(config.max_work);
        let mut ordered: Vec<_> = connections.iter().collect();
        ordered.sort_by_key(|c| (c.from.node.0, c.from.index, c.to.node.0, c.to.index));
        for connection in ordered {
            let key = (connection.from, connection.to);
            match self.route_connection_with_budget(connection, &config, &mut budget) {
                Ok(path) => {
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
        if connection.from.direction != SocketDirection::Output
            || connection.to.direction != SocketDirection::Input
        {
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
        let source = PortGeometry {
            obstacle: ids
                .binary_search_by_key(&connection.from.node.0, |id| id.0)
                .map_err(|_| RouteFailure::InvalidGeometry)?,
            position: self
                .nodes
                .get(&connection.from.node)
                .and_then(|node| node.output_socket_pos(connection.from.index))
                .ok_or(RouteFailure::InvalidGeometry)?,
            side: PortSide::Right,
        };
        let target = PortGeometry {
            obstacle: ids
                .binary_search_by_key(&connection.to.node.0, |id| id.0)
                .map_err(|_| RouteFailure::InvalidGeometry)?,
            position: self
                .nodes
                .get(&connection.to.node)
                .and_then(|node| node.input_socket_pos(connection.to.index))
                .ok_or(RouteFailure::InvalidGeometry)?,
            side: PortSide::Left,
        };
        route_with_budget(
            RouteInput {
                nodes: &nodes,
                source,
                target,
            },
            config,
            budget,
        )
    }
}

#[cfg(test)]
mod routing_input_tests {
    use egui::{Pos2, Rect};

    use super::super::widget::NodeGraphWidget;
    use super::*;
    use crate::model::{FrameId, SocketDirection, SocketId};
    use crate::runtime::NodeTypeRegistry;

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
