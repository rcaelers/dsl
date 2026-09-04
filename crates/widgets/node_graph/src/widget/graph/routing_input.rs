//! Adapts a complete widget layout into identity-free routing geometry.

use super::layout::GraphWidgetLayout;
use super::routing::{
    PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WirePath, route,
};
use crate::model::Connection;

impl GraphWidgetLayout {
    /// No viewport culling: even offscreen bodies obstruct a connection. Frames do not.
    /// This entry point is intentionally not called by painting until editor activation.
    pub(crate) fn route_connection(
        &self,
        connection: &Connection,
        config: &RouteConfig,
    ) -> Result<WirePath, RouteFailure> {
        let mut ids: Vec<_> = self.node_rects.keys().copied().collect();
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
        route(
            RouteInput {
                nodes: &nodes,
                source,
                target,
            },
            config,
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
    fn adaptation_includes_offscreen_nodes_excludes_frames_and_leaves_legacy_paths_active() {
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
        assert!(layout.wire_paths[&(connection.from, connection.to)].intersects_rect(obstacle));
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
