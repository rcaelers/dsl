use std::collections::HashMap;

use egui::{Pos2, Rect};

use super::routing::WirePath;
use super::widget::NodeGraphWidget;
use crate::model::{FrameId, NodeId, NodeKind, SocketDirection, SocketId};
use crate::support::{SOCKET_RADIUS, to_screen_rect};
use crate::widget::node::NodeWidget;

const SOCKET_HIT_PADDING: f32 = 5.0;
/// Share of a reroute point each of its two sockets claims inside the body.
///
/// A reroute is narrower than one socket hit area, so undivided hit areas
/// leave a few pixels of body between them and the point becomes almost
/// impossible to drag. Each socket keeps the outer quarter of the point and
/// all of its reach outside it, where nothing competes; the middle half is
/// the point's drag handle.
const REROUTE_SOCKET_BODY_SHARE: f32 = 0.25;
const FRAME_PADDING: f32 = 20.0;
const FRAME_TITLE_PADDING: f32 = 44.0;

pub(crate) struct GraphWidgetLayout {
    pub nodes: HashMap<NodeId, NodeWidget>,
    pub wire_paths: HashMap<(SocketId, SocketId), WirePath>,
    pub node_rects: HashMap<NodeId, Rect>,
    pub frame_rects: HashMap<FrameId, Rect>,
    pub frame_screen_rects: HashMap<FrameId, Rect>,
    pub node_screen_rects: HashMap<NodeId, Rect>,
    pub header_screen_rects: HashMap<NodeId, Rect>,
    pub collapse_toggle_screen_rects: HashMap<NodeId, Rect>,
    pub socket_screen_pos: HashMap<SocketId, Pos2>,
    pub socket_hit_rects: HashMap<SocketId, Rect>,
}

impl NodeGraphWidget {
    pub(crate) fn build_layout(&self, origin: Pos2) -> GraphWidgetLayout {
        let nodes: HashMap<NodeId, NodeWidget> = self
            .graph
            .nodes
            .iter()
            .map(|(&id, node)| {
                let status = self.node_statuses.get(&id).map(String::as_str);
                (id, NodeWidget::new(&self.graph, id, node, status))
            })
            .collect();

        let node_rects: HashMap<NodeId, Rect> = nodes
            .iter()
            .map(|(&id, widget)| (id, widget.node_rect()))
            .collect();
        let mut frame_order: Vec<_> = self.graph.frames.iter().collect();
        frame_order.sort_by_key(|frame| frame.node_ids.len());
        let mut frame_rects = HashMap::new();
        for frame in frame_order {
            let mut bounds = frame
                .node_ids
                .iter()
                .filter_map(|id| node_rects.get(id).copied())
                .reduce(|bounds, rect| bounds.union(rect));
            for child in &self.graph.frames {
                if child.id == frame.id
                    || child.node_ids.len() >= frame.node_ids.len()
                    || !child.node_ids.iter().all(|id| frame.node_ids.contains(id))
                {
                    continue;
                }
                if let Some(&child_rect) = frame_rects.get(&child.id) {
                    bounds = Some(bounds.map_or(child_rect, |bounds| bounds.union(child_rect)));
                }
            }
            if let Some(bounds) = bounds {
                frame_rects.insert(
                    frame.id,
                    Rect::from_min_max(
                        Pos2::new(
                            bounds.min.x - FRAME_PADDING,
                            bounds.min.y - FRAME_TITLE_PADDING,
                        ),
                        Pos2::new(bounds.max.x + FRAME_PADDING, bounds.max.y + FRAME_PADDING),
                    ),
                );
            }
        }
        let frame_screen_rects: HashMap<FrameId, Rect> = frame_rects
            .iter()
            .map(|(&id, &rect)| (id, to_screen_rect(rect, &self.view, origin)))
            .collect();
        let node_screen_rects: HashMap<NodeId, Rect> = nodes
            .iter()
            .map(|(&id, widget)| (id, to_screen_rect(widget.node_rect(), &self.view, origin)))
            .collect();
        let header_screen_rects: HashMap<NodeId, Rect> = nodes
            .iter()
            .map(|(&id, widget)| (id, to_screen_rect(widget.header_rect(), &self.view, origin)))
            .collect();
        let collapse_toggle_screen_rects: HashMap<NodeId, Rect> = nodes
            .iter()
            .filter_map(|(&id, widget)| {
                widget
                    .collapse_toggle_rect()
                    .map(|rect| (id, to_screen_rect(rect, &self.view, origin)))
            })
            .collect();

        let mut socket_screen_pos = HashMap::new();
        let mut socket_hit_rects = HashMap::new();
        let socket_hit_radius = SOCKET_RADIUS * self.view.zoom + SOCKET_HIT_PADDING;
        for (&id, widget) in &nodes {
            let Some(node) = self.graph.nodes.get(&id) else {
                continue;
            };
            let reroute_body = (node.kind == NodeKind::Reroute)
                .then(|| node_screen_rects.get(&id).copied())
                .flatten();
            for i in 0..node.inputs.len() {
                if let Some(pos) = widget.input_socket_pos(i) {
                    let socket_id = SocketId {
                        node: id,
                        index: i,
                        direction: SocketDirection::Input,
                    };
                    let screen_pos = self.view.canvas_to_screen(origin, pos);
                    socket_screen_pos.insert(socket_id, screen_pos);
                    let mut hit_rect = Rect::from_center_size(
                        screen_pos,
                        egui::Vec2::splat(socket_hit_radius * 2.0),
                    );
                    if let Some(body) = reroute_body {
                        hit_rect.max.x = hit_rect
                            .max
                            .x
                            .min(body.left() + body.width() * REROUTE_SOCKET_BODY_SHARE);
                    }
                    socket_hit_rects.insert(socket_id, hit_rect);
                }
            }
            for i in 0..node.outputs.len() {
                if let Some(pos) = widget.output_socket_pos(i) {
                    let socket_id = SocketId {
                        node: id,
                        index: i,
                        direction: SocketDirection::Output,
                    };
                    let screen_pos = self.view.canvas_to_screen(origin, pos);
                    socket_screen_pos.insert(socket_id, screen_pos);
                    let mut hit_rect = Rect::from_center_size(
                        screen_pos,
                        egui::Vec2::splat(socket_hit_radius * 2.0),
                    );
                    if let Some(body) = reroute_body {
                        hit_rect.min.x = hit_rect
                            .min
                            .x
                            .max(body.right() - body.width() * REROUTE_SOCKET_BODY_SHARE);
                    }
                    socket_hit_rects.insert(socket_id, hit_rect);
                }
            }
        }

        let wire_paths = self
            .graph
            .connections
            .iter()
            .filter_map(|connection| {
                let from = nodes
                    .get(&connection.from.node)?
                    .output_socket_pos(connection.from.index)?;
                let to = nodes
                    .get(&connection.to.node)?
                    .input_socket_pos(connection.to.index)?;
                Some((
                    (connection.from, connection.to),
                    WirePath::legacy(from, to, self.view.zoom),
                ))
            })
            .collect();

        GraphWidgetLayout {
            nodes,
            wire_paths,
            node_rects,
            frame_rects,
            frame_screen_rects,
            node_screen_rects,
            header_screen_rects,
            collapse_toggle_screen_rects,
            socket_screen_pos,
            socket_hit_rects,
        }
    }
}
#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::runtime::NodeTypeRegistry;

    #[test]
    fn a_reroute_point_keeps_its_middle_as_a_drag_handle() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(100.0, 100.0))
            .expect("built-in reroute node");
        let layout = widget.build_layout(Pos2::ZERO);
        let body = layout.node_screen_rects[&node];
        let input = layout.socket_hit_rects[&SocketId {
            node,
            index: 0,
            direction: SocketDirection::Input,
        }];
        let output = layout.socket_hit_rects[&SocketId {
            node,
            index: 0,
            direction: SocketDirection::Output,
        }];

        // The middle half of the point belongs to the point itself.
        let quarter = body.width() * REROUTE_SOCKET_BODY_SHARE;
        for offset in [-quarter + 0.5, 0.0, quarter - 0.5] {
            let inside = Pos2::new(body.center().x + offset, body.center().y);
            assert!(!input.contains(inside), "input claims {inside:?}");
            assert!(!output.contains(inside), "output claims {inside:?}");
        }

        // Both tips still start a wire, on the point and beyond it.
        assert!(input.contains(Pos2::new(body.left(), body.center().y)));
        assert!(input.contains(Pos2::new(body.left() - 5.0, body.center().y)));
        assert!(output.contains(Pos2::new(body.right(), body.center().y)));
        assert!(output.contains(Pos2::new(body.right() + 5.0, body.center().y)));
    }

    #[test]
    fn sockets_off_a_reroute_keep_their_full_hit_area() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(100.0, 100.0))
            .expect("built-in reroute node");
        // Same geometry, no longer a reroute: the division does not apply.
        widget.graph.nodes.get_mut(&node).unwrap().kind = NodeKind::Regular;
        let layout = widget.build_layout(Pos2::ZERO);
        let input = layout.socket_hit_rects[&SocketId {
            node,
            index: 0,
            direction: SocketDirection::Input,
        }];

        let expected = SOCKET_RADIUS * widget.view.zoom + SOCKET_HIT_PADDING;
        assert_eq!(input.width(), expected * 2.0);
    }
}
