//! Wire gesture, hit-testing, and rewiring behavior.
//!
//! This module owns socket compatibility, wire dragging and cutting, reroute
//! insertion, and node-on-wire splicing. It mutates topology through the graph
//! widget but does not dispatch menus or coordinate unrelated gestures.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use egui::Pos2;

use super::interaction_state::{InteractionState, WIRE_SNAP_DISTANCE};
use super::layout::GraphWidgetLayout;
use super::response::GraphResponses;
use super::widget::NodeGraphWidget;
use crate::model::{Connection, NodeId, NodeKind, SocketDirection, SocketId};
use crate::support::{bezier_wire_distance, bezier_wire_intersects_rect, wire_intersects_knife};
use crate::widget::node::NodeWidget;

impl NodeGraphWidget {
    fn compatible_wire_target(&self, from: SocketId, to: SocketId) -> bool {
        if from == to {
            return false;
        }
        let (output, input) = if from.direction == SocketDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        if output.direction != SocketDirection::Output || input.direction != SocketDirection::Input
        {
            return false;
        }
        let out_type = self
            .graph
            .nodes
            .get(&output.node)
            .and_then(|n| n.outputs.get(output.index))
            .map(|s| s.effective_type());
        let in_socket = self
            .graph
            .nodes
            .get(&input.node)
            .and_then(|n| n.inputs.get(input.index));
        matches!((out_type, in_socket), (Some(ot), Some(is)) if is.accepts(ot))
    }

    /// Every node with at least one visible socket compatible with `from` —
    /// cached once into `InteractionState::DraggingWire::connectable` when a
    /// wire drag starts (Phase 4.3).
    pub(crate) fn connectable_nodes(&self, from: SocketId) -> HashSet<NodeId> {
        self.graph
            .nodes
            .values()
            .filter(|node| {
                let inputs = node
                    .inputs
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.visible)
                    .map(|(index, _)| SocketId {
                        node: node.id,
                        index,
                        direction: SocketDirection::Input,
                    });
                let outputs = node
                    .outputs
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.visible)
                    .map(|(index, _)| SocketId {
                        node: node.id,
                        index,
                        direction: SocketDirection::Output,
                    });
                inputs
                    .chain(outputs)
                    .any(|candidate| self.compatible_wire_target(from, candidate))
            })
            .map(|node| node.id)
            .collect()
    }

    /// The socket a wire drag started from, if it still exists.
    fn socket_at(&self, id: SocketId) -> Option<&crate::model::Socket> {
        let node = self.graph.nodes.get(&id.node)?;
        match id.direction {
            SocketDirection::Input => node.inputs.get(id.index),
            SocketDirection::Output => node.outputs.get(id.index),
        }
    }

    /// First visible socket on `node_id` compatible with `from` — used to
    /// auto-wire a freshly added node (link-drag search, Phase 1.1).
    pub(crate) fn first_compatible_socket(
        &self,
        from: SocketId,
        node_id: NodeId,
    ) -> Option<SocketId> {
        let node = self.graph.nodes.get(&node_id)?;
        let inputs = node
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.visible)
            .map(|(index, _)| SocketId {
                node: node_id,
                index,
                direction: SocketDirection::Input,
            });
        let outputs = node
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.visible)
            .map(|(index, _)| SocketId {
                node: node_id,
                index,
                direction: SocketDirection::Output,
            });
        inputs
            .chain(outputs)
            .find(|&candidate| self.compatible_wire_target(from, candidate))
    }

    pub(crate) fn snapped_wire_target(
        &self,
        from: SocketId,
        pointer_canvas: Pos2,
        layout: &GraphWidgetLayout,
    ) -> Option<(SocketId, Pos2)> {
        let threshold = WIRE_SNAP_DISTANCE / self.view.zoom;
        layout
            .nodes
            .iter()
            .flat_map(|(&node_id, widget)| {
                let input_count = self
                    .graph
                    .nodes
                    .get(&node_id)
                    .map_or(0, |node| node.inputs.len());
                let output_count = self
                    .graph
                    .nodes
                    .get(&node_id)
                    .map_or(0, |node| node.outputs.len());
                let inputs = (0..input_count).filter_map(move |index| {
                    widget.input_socket_pos(index).map(|pos| {
                        (
                            SocketId {
                                node: node_id,
                                index,
                                direction: SocketDirection::Input,
                            },
                            pos,
                        )
                    })
                });
                let outputs = (0..output_count).filter_map(move |index| {
                    widget.output_socket_pos(index).map(|pos| {
                        (
                            SocketId {
                                node: node_id,
                                index,
                                direction: SocketDirection::Output,
                            },
                            pos,
                        )
                    })
                });
                inputs.chain(outputs)
            })
            .filter(|(target, _)| self.compatible_wire_target(from, *target))
            .filter_map(|(target, pos)| {
                let dist = pointer_canvas.distance(pos);
                (dist <= threshold).then_some((target, pos, dist))
            })
            .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
            .map(|(target, pos, _)| (target, pos))
    }

    /// Whether the move modifier can pick a link up from `socket`.
    ///
    /// Only a connected output offers the choice: a plain drag from it adds
    /// another link, so the modifier is what makes it move one instead.
    /// Inputs hold a single link that a plain drag already moves, and an
    /// unconnected socket has nothing to pick up.
    fn socket_link_is_movable(&self, socket: SocketId) -> bool {
        socket.direction == SocketDirection::Output
            && self
                .graph
                .connections
                .iter()
                .any(|connection| connection.from == socket)
    }

    /// The link a move-modifier drag from `output` picks up, identified by
    /// the input end it stays anchored to.
    ///
    /// One output may feed several inputs; the link whose destination is
    /// nearest the pointer wins, which is unambiguous for the usual single
    /// link. Dragging the input end remains the way to grab one specific
    /// link out of many.
    pub(crate) fn movable_output_link(
        &self,
        output: SocketId,
        pointer_screen: Pos2,
        layout: &GraphWidgetLayout,
    ) -> Option<SocketId> {
        if !self.socket_link_is_movable(output) {
            return None;
        }
        self.graph
            .connections
            .iter()
            .filter(|connection| connection.from == output)
            .filter_map(|connection| {
                let target = layout.socket_screen_pos.get(&connection.to)?;
                Some((connection.to, pointer_screen.distance_sq(*target)))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(socket, _)| socket)
    }

    /// The link a move-modifier drag picks up from a reroute point.
    ///
    /// A reroute is a waypoint on one wire, and its body is barely wider
    /// than the two socket hit areas flanking it, so the modifier picks the
    /// link up from anywhere on the point rather than only from its output
    /// half. A plain drag still moves the point itself.
    pub(crate) fn movable_reroute_link(
        &self,
        node_id: NodeId,
        pointer_screen: Pos2,
        layout: &GraphWidgetLayout,
    ) -> Option<SocketId> {
        let node = self.graph.nodes.get(&node_id)?;
        if node.kind != NodeKind::Reroute {
            return None;
        }
        // A reroute carries exactly one pass-through pair.
        self.movable_output_link(
            SocketId {
                node: node_id,
                index: 0,
                direction: SocketDirection::Output,
            },
            pointer_screen,
            layout,
        )
    }

    /// The link a wire drag anchored at `from` is carrying elsewhere.
    ///
    /// A drag anchored at an input is relocating that input's existing link
    /// to another output, and the link stays in the document until the drag
    /// lands — it is only hidden while it is being carried. Removing it up
    /// front would revert the socket instead: a variadic member collapses
    /// and renumbers every socket after it, so `from` would address a
    /// different socket by the time the drag is dropped, and connecting
    /// there would evict that socket's own link.
    pub(crate) fn moved_connection(&self, from: SocketId) -> Option<SocketId> {
        (from.direction == SocketDirection::Input
            && self
                .graph
                .connections
                .iter()
                .any(|connection| connection.to == from))
        .then_some(from)
    }

    /// Starts carrying the link that hangs from `anchor` to another output,
    /// with `anchor` staying where it is.
    ///
    /// Nothing is mutated: dropping the link on an output replaces the
    /// connection into `anchor` in one step, and abandoning the drag leaves
    /// the link exactly as it was.
    pub(crate) fn start_link_move(
        &self,
        anchor: SocketId,
        anchor_screen: Pos2,
        origin: Pos2,
        pointer_canvas: Pos2,
    ) -> InteractionState {
        InteractionState::DraggingWire {
            from: anchor,
            from_canvas: self.view.screen_to_canvas(origin, anchor_screen),
            current_canvas: pointer_canvas,
            restore_on_cancel: false,
            connectable: Rc::new(self.connectable_nodes(anchor)),
        }
    }

    fn add_wire_connection(&mut self, from: SocketId, to: SocketId, push_undo: bool) {
        let (output, input) = if from.direction == SocketDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        if self.compatible_wire_target(from, to) {
            if push_undo {
                self.push_undo_snapshot();
            }
            // Dropped on an occupied variadic member, a new link takes that
            // place and pushes the group down instead of replacing what is
            // there. The anchored input of a carried link is not a drop
            // target — that link is landing back on its own socket.
            if input == from || !self.graph.insert_variadic_connection(output, input) {
                self.graph.add_connection(output, input);
            }
            self.run_update(output.node);
            self.run_update(input.node);
        }
    }

    pub(crate) fn try_wire_insert(
        &mut self,
        node_id: NodeId,
        pointer_canvas: Option<Pos2>,
        nodes: &HashMap<NodeId, NodeWidget>,
    ) {
        let Some(point) =
            pointer_canvas.or_else(|| Some(nodes.get(&node_id)?.node_rect().center()))
        else {
            return;
        };
        let Some(idx) = self.closest_insert_wire(node_id, point, nodes) else {
            return;
        };
        let conn = self.graph.connections[idx].clone();

        if let Some((ii, oi)) = self.wire_insert_sockets(node_id, &conn) {
            self.push_undo_snapshot();
            // Same ordering as `insert_reroute_on_wire`: the second half
            // displaces the original link, so no input socket is reverted
            // and renumbered underneath the saved target.
            self.graph.add_connection(
                conn.from,
                SocketId {
                    node: node_id,
                    index: ii,
                    direction: SocketDirection::Input,
                },
            );
            self.graph.add_connection(
                SocketId {
                    node: node_id,
                    index: oi,
                    direction: SocketDirection::Output,
                },
                conn.to,
            );
            self.run_update(node_id);
            self.run_update(conn.from.node);
            self.run_update(conn.to.node);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_drag_wire(
        &mut self,
        ui: &egui::Ui,
        pointer: Option<Pos2>,
        pointer_canvas: Option<Pos2>,
        responses: &GraphResponses,
        layout: &GraphWidgetLayout,
        from: SocketId,
        from_canvas: Pos2,
        mut current_canvas: Pos2,
        restore_on_cancel: bool,
        connectable: Rc<HashSet<NodeId>>,
    ) -> InteractionState {
        if let Some(pc) = pointer_canvas {
            let snapped = self.snapped_wire_target(from, pc, layout);
            current_canvas = snapped.map_or(pc, |(_, pos)| pos);
            // Not over a compatible socket: releasing here opens link-drag
            // search instead of connecting directly. Blender flags this
            // with a "+" cursor; egui's closest equivalent is Copy. Only a
            // drag that creates a link says so: a drag carrying one, or one
            // that pulled an input off its source, ends with the same
            // number of links it started with.
            let relocating = restore_on_cancel || self.moved_connection(from).is_some();
            if snapped.is_none() && !relocating {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
            }
        }
        let modifiers = ui.input(|input| input.modifiers);
        let confirm = self
            .input_bindings
            .pointer_trigger(&["node_graph.drag_wire"], "confirm_link", modifiers)
            .map_or_else(
                || ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary)),
                |(button, _)| ui.input(|input| input.pointer.button_released(button)),
            );
        if !confirm {
            return InteractionState::DraggingWire {
                from,
                from_canvas,
                current_canvas,
                restore_on_cancel,
                connectable,
            };
        }

        if let Some((target, _)) =
            pointer_canvas.and_then(|pc| self.snapped_wire_target(from, pc, layout))
        {
            self.add_wire_connection(from, target, !restore_on_cancel);
            return InteractionState::Idle;
        }

        if let Some((&target, _)) = responses
            .sockets
            .iter()
            .find(|(sid, response)| **sid != from && response.hovered())
        {
            self.add_wire_connection(from, target, !restore_on_cancel);
            return InteractionState::Idle;
        }

        // Released on empty canvas: open the link-drag search so a new node
        // can be added and wired in with one gesture (Blender's "link drag
        // search"). Esc/click-outside on the popup just drops the wire.
        if let Some(pointer_screen) = pointer
            && let Some(from_socket) = self.socket_at(from).cloned()
        {
            let canvas_pos = pointer_canvas.unwrap_or(current_canvas);
            self.menu.open_link_drag_search(
                pointer_screen,
                &self.registry,
                canvas_pos,
                from,
                &from_socket,
            );
        }
        InteractionState::Idle
    }

    pub(crate) fn apply_knife_cut(&mut self, path: &[Pos2], nodes: &HashMap<NodeId, NodeWidget>) {
        if path.len() < 2 {
            return;
        }
        let to_remove: Vec<usize> = self
            .graph
            .connections
            .iter()
            .enumerate()
            .filter_map(|(idx, conn)| {
                let fp = nodes
                    .get(&conn.from.node)
                    .and_then(|w| w.output_socket_pos(conn.from.index))?;
                let tp = nodes
                    .get(&conn.to.node)
                    .and_then(|w| w.input_socket_pos(conn.to.index))?;
                path.windows(2)
                    .any(|w| wire_intersects_knife(fp, tp, w[0], w[1]))
                    .then_some(idx)
            })
            .collect();
        if !to_remove.is_empty() {
            self.push_undo_snapshot();
        }
        let mut touched = Vec::new();
        for idx in to_remove.into_iter().rev() {
            let conn = self.graph.remove_connection_at(idx);
            touched.push(conn.from.node);
            touched.push(conn.to.node);
        }
        touched.sort_unstable_by_key(|id: &NodeId| id.0);
        touched.dedup();
        for node_id in touched {
            self.run_update(node_id);
        }
    }

    pub(crate) fn update_cut_wire(
        &mut self,
        ui: &egui::Ui,
        pointer_canvas: Option<Pos2>,
        nodes: &HashMap<NodeId, NodeWidget>,
        mut path: Vec<Pos2>,
    ) -> InteractionState {
        let button = self
            .input_bindings
            .pointer_button(&["node_graph"], "cut_wires")
            .unwrap_or(egui::PointerButton::Secondary);
        if ui.input(|i| i.pointer.button_down(button)) {
            if let Some(pc) = pointer_canvas {
                let min_step = 4.0 / self.view.zoom;
                if path.last().is_none_or(|&last| last.distance(pc) > min_step) {
                    path.push(pc);
                }
            }
            return InteractionState::CuttingWire { path };
        }
        self.apply_knife_cut(&path, nodes);
        InteractionState::Idle
    }
    /// Connection nearest `point_canvas`, within snap distance — double-click
    /// to insert a reroute (Phase 6.2). Unlike `closest_insert_wire`, this
    /// isn't gated on overlapping any particular node's rect; it's a plain
    /// point-to-wire hit test.
    pub(crate) fn wire_near_point(
        &self,
        point_canvas: Pos2,
        nodes: &HashMap<NodeId, NodeWidget>,
    ) -> Option<usize> {
        let threshold = WIRE_SNAP_DISTANCE / self.view.zoom;
        let mut best: Option<(usize, f32)> = None;
        for (idx, conn) in self.graph.connections.iter().enumerate() {
            let fp = nodes
                .get(&conn.from.node)
                .and_then(|w| w.output_socket_pos(conn.from.index));
            let tp = nodes
                .get(&conn.to.node)
                .and_then(|w| w.input_socket_pos(conn.to.index));
            let (Some(fp), Some(tp)) = (fp, tp) else {
                continue;
            };
            let dist = bezier_wire_distance(fp, tp, point_canvas);
            if dist <= threshold && dist < best.map_or(f32::INFINITY, |(_, d)| d) {
                best = Some((idx, dist));
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Splits the connection at `connection_index` by inserting a fresh
    /// `Reroute` node at `pos_canvas` and rewiring both halves through it —
    /// one undo step (Phase 6.2).
    pub(crate) fn insert_reroute_on_wire(&mut self, connection_index: usize, pos_canvas: Pos2) {
        let Some(conn) = self.graph.connections.get(connection_index).cloned() else {
            return;
        };
        self.push_undo_snapshot();
        let Some(node_id) = self.add_node_at("Reroute", pos_canvas) else {
            return;
        };
        // The original link is displaced by the second half rather than
        // removed first: removing it would revert its input socket, and a
        // variadic member collapsing there renumbers every socket after it
        // — `conn.to` would then address a different socket, whose own link
        // this would evict.
        self.graph.add_connection(
            conn.from,
            SocketId {
                node: node_id,
                index: 0,
                direction: SocketDirection::Input,
            },
        );
        self.graph.add_connection(
            SocketId {
                node: node_id,
                index: 0,
                direction: SocketDirection::Output,
            },
            conn.to,
        );
        self.run_update(conn.from.node);
        self.run_update(node_id);
        self.run_update(conn.to.node);
    }

    /// Wire overlapped by the dragged node's rect, ignoring wires already
    /// attached to `node_id`; when several overlap, the one closest to
    /// `point` (the pointer) wins. Compatibility is deliberately not a
    /// selection criterion — the same wire must be chosen whether or not the
    /// node fits, so the preview (highlight vs. muted) and the actual drop
    /// always agree on the target.
    fn closest_insert_wire(
        &self,
        node_id: NodeId,
        point: Pos2,
        nodes: &HashMap<NodeId, NodeWidget>,
    ) -> Option<usize> {
        let node_rect = nodes.get(&node_id)?.node_rect();
        let mut best: Option<(usize, f32)> = None;
        for (idx, conn) in self.graph.connections.iter().enumerate() {
            if conn.from.node == node_id || conn.to.node == node_id {
                continue;
            }
            let fp = nodes
                .get(&conn.from.node)
                .and_then(|w| w.output_socket_pos(conn.from.index));
            let tp = nodes
                .get(&conn.to.node)
                .and_then(|w| w.input_socket_pos(conn.to.index));
            let (Some(fp), Some(tp)) = (fp, tp) else {
                continue;
            };
            if !bezier_wire_intersects_rect(fp, tp, node_rect) {
                continue;
            }
            let dist = bezier_wire_distance(fp, tp, point);
            if dist < best.map_or(f32::INFINITY, |(_, d)| d) {
                best = Some((idx, dist));
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Socket indices (input, output) on `node_id` that would splice it into
    /// `conn`, or `None` if the node cannot be inserted there.
    fn wire_insert_sockets(&self, node_id: NodeId, conn: &Connection) -> Option<(usize, usize)> {
        // Only a completely fresh, unconnected node gets spliced into a
        // wire — a node already wired up elsewhere shouldn't have its
        // existing topology silently rearranged by an incidental drag-over.
        if node_has_any_connection(&self.graph.connections, node_id) {
            return None;
        }
        let src_type = self
            .graph
            .nodes
            .get(&conn.from.node)?
            .outputs
            .get(conn.from.index)?
            .effective_type()
            .to_owned();
        let dst_socket = self
            .graph
            .nodes
            .get(&conn.to.node)?
            .inputs
            .get(conn.to.index)?;
        let nn = self.graph.nodes.get(&node_id)?;
        let in_idx = nn
            .inputs
            .iter()
            .position(|s| s.visible && s.accepts(&src_type))?;
        let out_idx = nn
            .outputs
            .iter()
            .position(|s| s.visible && dst_socket.accepts(&s.type_name))?;
        Some((in_idx, out_idx))
    }

    /// Wire the dragged node is hovering, and whether it can be spliced in.
    pub(crate) fn compute_insert_candidate_wire(
        &self,
        node_id: NodeId,
        pointer_canvas: Option<Pos2>,
        nodes: &HashMap<NodeId, NodeWidget>,
    ) -> Option<(usize, bool)> {
        let point = pointer_canvas.or_else(|| Some(nodes.get(&node_id)?.node_rect().center()))?;
        let idx = self.closest_insert_wire(node_id, point, nodes)?;
        let conn = self.graph.connections.get(idx)?;
        Some((idx, self.wire_insert_sockets(node_id, conn).is_some()))
    }
}

/// Whether `node_id` is an endpoint of any existing connection.
pub(crate) fn node_has_any_connection(connections: &[Connection], node_id: NodeId) -> bool {
    connections
        .iter()
        .any(|c| c.from.node == node_id || c.to.node == node_id)
}
#[cfg(test)]
mod wire_tests {
    use egui::Pos2;

    use super::*;
    use crate::model::VariadicInfo;
    use crate::runtime::NodeTypeRegistry;

    fn input(node: NodeId, index: usize) -> SocketId {
        SocketId {
            node,
            index,
            direction: SocketDirection::Input,
        }
    }

    fn output(node: NodeId) -> SocketId {
        SocketId {
            node,
            index: 0,
            direction: SocketDirection::Output,
        }
    }

    /// A widget holding `sources` reroutes feeding a sink whose variadic
    /// group has one member per source plus the trailing placeholder.
    fn wired_group(sources: usize, max: usize) -> (NodeGraphWidget, Vec<NodeId>, NodeId) {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let feeds: Vec<NodeId> = (0..sources)
            .map(|index| {
                widget
                    .add_node_at("Reroute", Pos2::new(0.0, index as f32 * 50.0))
                    .expect("built-in reroute node")
            })
            .collect();
        let sink = widget
            .add_node_at("Reroute", Pos2::new(300.0, 0.0))
            .expect("built-in reroute node");
        {
            let node = widget.graph.nodes.get_mut(&sink).unwrap();
            let template = node.inputs[0].clone();
            node.inputs = (0..=sources)
                .map(|index| {
                    let mut socket = template.clone();
                    let placeholder = index == sources;
                    socket.name = if placeholder {
                        "D".to_owned()
                    } else {
                        format!("D {}", index + 1)
                    };
                    socket.variadic = Some(VariadicInfo {
                        base: "D".to_owned(),
                        max,
                        placeholder,
                    });
                    socket
                })
                .collect();
        }
        for (index, &feed) in feeds.iter().enumerate() {
            widget
                .graph
                .add_connection(output(feed), input(sink, index));
        }
        (widget, feeds, sink)
    }

    fn source_of(widget: &NodeGraphWidget, sink: NodeId, index: usize) -> Option<NodeId> {
        widget
            .graph
            .connections
            .iter()
            .find(|connection| connection.to == input(sink, index))
            .map(|connection| connection.from.node)
    }

    #[test]
    fn dropping_a_link_on_an_occupied_member_inserts_it_there() {
        let (mut widget, feeds, sink) = wired_group(3, 8);
        let newcomer = widget
            .add_node_at("Reroute", Pos2::new(0.0, 400.0))
            .expect("built-in reroute node");

        widget.add_wire_connection(output(newcomer), input(sink, 1), false);

        // The newcomer took the second slot and everything below it moved
        // one down, keeping its own source.
        assert_eq!(widget.graph.nodes[&sink].inputs.len(), 5);
        assert_eq!(source_of(&widget, sink, 0), Some(feeds[0]));
        assert_eq!(source_of(&widget, sink, 1), Some(newcomer));
        assert_eq!(source_of(&widget, sink, 2), Some(feeds[1]));
        assert_eq!(source_of(&widget, sink, 3), Some(feeds[2]));
        assert_eq!(widget.graph.connections.len(), 4);
    }

    #[test]
    fn a_carried_link_landing_on_its_own_socket_does_not_insert() {
        let (mut widget, feeds, sink) = wired_group(2, 8);
        let newcomer = widget
            .add_node_at("Reroute", Pos2::new(0.0, 400.0))
            .expect("built-in reroute node");
        // The drag carries the link into the first member towards another
        // output, so that member is the anchor, not a drop target.
        let anchor = input(sink, 0);
        widget.add_wire_connection(anchor, output(newcomer), false);

        assert_eq!(widget.graph.nodes[&sink].inputs.len(), 3);
        assert_eq!(widget.graph.connections.len(), 2);
        assert_eq!(source_of(&widget, sink, 0), Some(newcomer));
        assert_eq!(source_of(&widget, sink, 1), Some(feeds[1]));
    }
}
