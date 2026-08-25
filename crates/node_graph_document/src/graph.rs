use std::collections::{BTreeMap, HashMap, HashSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::connection::Connection;
use super::frame::{Frame, FrameId};
use super::ids::{NodeId, SocketDirection, SocketId};
use super::node::{Node, NodeKind};
use super::presentation::GraphColor;
use super::socket::{Socket, VariadicInfo};

#[derive(Serialize)]
struct SemanticGraphSnapshot<'a> {
    nodes: Vec<SemanticNodeSnapshot<'a>>,
    connections: &'a [Connection],
    extensions: &'a BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct SemanticNodeSnapshot<'a> {
    id: NodeId,
    kind: &'a NodeKind,
    type_name: &'a str,
    inputs: &'a [Socket],
    outputs: &'a [Socket],
    muted: bool,
    state: &'a serde_json::Value,
}

/// Persisted graph document, including nodes, wires, frames, and extensions.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GraphState {
    /// Nodes keyed by their stable document identity.
    pub nodes: HashMap<NodeId, Node>,
    /// Directed output-to-input connections.
    pub connections: Vec<Connection>,
    /// User-arranged visual frame groups.
    pub frames: Vec<Frame>,
    #[serde(flatten)]
    /// Generic allocation counters and owner-managed document extensions.
    pub metadata: GraphMetadata,
}

/// Generic persisted metadata associated with the entire graph document.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GraphMetadata {
    next_id: u32,
    next_frame_id: u32,
    #[serde(skip)]
    semantic_revision: u64,
    /// Namespaced, owner-managed document state. Values belong to the whole
    /// saved graph, may refer to graph identities, and are not part of copied
    /// node fragments. Generic graph code only preserves them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extensions: BTreeMap<String, serde_json::Value>,
}

impl GraphState {
    /// Returns the transient revision of processing-relevant document state.
    ///
    /// The revision is not persisted. Document mutation APIs advance it for nodes and wires;
    /// extension owners explicitly select semantic extension APIs when their values affect
    /// compilation or execution.
    pub fn semantic_revision(&self) -> u64 {
        self.metadata.semantic_revision
    }

    /// Records a processing-relevant mutation performed through direct record access.
    pub fn mark_semantic_change(&mut self) {
        self.metadata.semantic_revision = self.metadata.semantic_revision.saturating_add(1);
    }

    /// Advances this document beyond a replaced document's transient semantic revision.
    ///
    /// This keeps revisions monotonic when undo, redo, or whole-document replacement installs a
    /// cloned or freshly deserialized graph.
    ///
    /// # Parameters
    /// - `previous`: Revision of the document being replaced.
    pub fn mark_semantic_change_after(&mut self, previous: u64) {
        self.metadata.semantic_revision = self
            .metadata
            .semantic_revision
            .max(previous)
            .saturating_add(1);
    }

    /// Serializes the executable meaning of this graph without editor-only layout state.
    ///
    /// Position, selection, collapse state, display title, header color, frames, and allocation
    /// counters cannot affect graph lowering. Consumers can compare this stable snapshot before
    /// performing expensive semantic discovery or synchronization.
    pub fn semantic_snapshot(&self) -> Vec<u8> {
        let mut nodes = self.nodes.values().collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.id.0);
        let nodes = nodes
            .into_iter()
            .map(|node| SemanticNodeSnapshot {
                id: node.id,
                kind: &node.kind,
                type_name: node.def_name(),
                inputs: &node.inputs,
                outputs: &node.outputs,
                muted: node.muted,
                state: &node.state,
            })
            .collect();
        serde_json::to_vec(&SemanticGraphSnapshot {
            nodes,
            connections: &self.connections,
            extensions: &self.metadata.extensions,
        })
        .expect("graph semantic state is always serializable")
    }

    /// Reads one owner namespace without exposing the extension map itself.
    ///
    /// Owners migrate and clean up their own values. A caller that cannot
    /// understand the stored value must leave it unchanged.
    ///
    /// # Parameters
    /// - `key`: Namespaced extension key owned by the caller.
    pub fn extension<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, serde_json::Error> {
        self.metadata
            .extensions
            .get(key)
            .cloned()
            .map(serde_json::from_value)
            .transpose()
    }

    /// Serializes and stores a value in one owner-managed extension namespace.
    ///
    /// # Parameters
    /// - `key`: Namespaced extension key owned by the caller.
    /// - `value`: Serializable owner-managed document state to store.
    pub fn set_extension<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: T,
    ) -> Result<(), serde_json::Error> {
        self.metadata
            .extensions
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    /// Stores an owner-managed extension and advances the semantic revision when it changes.
    ///
    /// Owners use this API only when the extension affects compilation or execution. Presentation
    /// extensions continue to use [`Self::set_extension`].
    ///
    /// # Parameters
    /// - `key`: Namespaced extension key owned by the caller.
    /// - `value`: Serializable processing-relevant document state.
    pub fn set_semantic_extension<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: T,
    ) -> Result<bool, serde_json::Error> {
        let key = key.into();
        let value = serde_json::to_value(value)?;
        if self.metadata.extensions.get(&key) == Some(&value) {
            return Ok(false);
        }
        self.metadata.extensions.insert(key, value);
        self.mark_semantic_change();
        Ok(true)
    }

    /// Removes one owner-managed extension value.
    ///
    /// # Parameters
    /// - `key`: Namespaced extension key to remove. Unknown keys are ignored.
    pub fn remove_extension(&mut self, key: &str) {
        self.metadata.extensions.remove(key);
    }

    /// Removes a processing-relevant extension and advances the semantic revision when present.
    ///
    /// # Parameters
    /// - `key`: Namespaced semantic extension key to remove.
    pub fn remove_semantic_extension(&mut self, key: &str) -> bool {
        let removed = self.metadata.extensions.remove(key).is_some();
        if removed {
            self.mark_semantic_change();
        }
        removed
    }

    /// Allocates the next stable node identity for this graph document.
    pub fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.metadata.next_id);
        self.metadata.next_id += 1;
        id
    }

    /// Inserts or replaces a node by its stable identity.
    ///
    /// # Parameters
    /// - `node`: Node to add. Existing connections are not reconciled automatically.
    pub fn add_node(&mut self, node: Node) {
        self.nodes.insert(node.id, node);
        self.mark_semantic_change();
    }

    /// Removes a node and every incoming or outgoing connection attached to it.
    ///
    /// # Parameters
    /// - `id`: Stable identity of the node to remove. Unknown identities are ignored.
    pub fn remove_node(&mut self, id: NodeId) {
        if self.nodes.remove(&id).is_none() {
            return;
        }
        self.mark_semantic_change();
        self.connections
            .retain(|connection| connection.to.node != id);
        // Outgoing connections are dropped one by one so each downstream
        // input reverts properly (resolution cleared, variadic members
        // removed with index fixup).
        while let Some(position) = self
            .connections
            .iter()
            .position(|connection| connection.from.node == id)
        {
            self.remove_connection_at(position);
        }
    }

    /// Adds a connection, replacing any existing one into `to`, and resolves
    /// the input socket's type to the output's type when they differ. When
    /// `to` is a variadic placeholder, it becomes a member and a new
    /// placeholder is spawned (until the group's max).
    ///
    /// # Parameters
    /// - `from`: Output socket supplying the connection.
    /// - `to`: Input socket receiving the connection.
    pub fn add_connection(&mut self, from: SocketId, to: SocketId) {
        self.connections.retain(|connection| connection.to != to);
        self.connections.push(Connection { from, to });
        self.resolve_input(from, to);
        self.grow_variadic_group(to);
        self.propagate_reroute_output(to.node);
        self.mark_semantic_change();
    }

    /// Removes the connection feeding `to`, if any, and reverts the input
    /// socket: back to its native type, or removed entirely if it is a
    /// variadic member. Returns whether a connection was removed.
    ///
    /// # Parameters
    /// - `to`: Input socket whose single incoming connection is removed.
    pub fn disconnect_input(&mut self, to: SocketId) -> bool {
        let before = self.connections.len();
        self.connections.retain(|connection| connection.to != to);
        let removed = self.connections.len() != before;
        if removed {
            self.on_input_disconnected(to);
            self.mark_semantic_change();
        }
        removed
    }

    /// Removes the connection at `index` and reverts its input socket.
    ///
    /// # Parameters
    /// - `index`: Position in [`Self::connections`] of the connection to remove.
    pub fn remove_connection_at(&mut self, index: usize) -> Connection {
        let connection = self.connections.remove(index);
        self.on_input_disconnected(connection.to);
        self.mark_semantic_change();
        connection
    }

    fn on_input_disconnected(&mut self, to: SocketId) {
        let is_member = self
            .nodes
            .get(&to.node)
            .and_then(|node| node.inputs.get(to.index))
            .is_some_and(Socket::is_variadic_member);
        if is_member {
            self.collapse_variadic_member(to);
        } else {
            self.clear_input_resolution(to);
        }
        self.propagate_reroute_output(to.node);
    }

    fn resolve_input(&mut self, from: SocketId, to: SocketId) {
        let out_type = self
            .nodes
            .get(&from.node)
            .and_then(|node| node.outputs.get(from.index))
            .map(|socket| socket.effective_type().to_owned());
        let Some(node) = self.nodes.get_mut(&to.node) else {
            return;
        };
        let Some(socket) = node.inputs.get_mut(to.index) else {
            return;
        };
        socket.resolved_type = match out_type {
            Some(t) if t != "Any" && t != socket.type_name => Some(t),
            _ => None,
        };
    }

    fn clear_input_resolution(&mut self, to: SocketId) {
        if let Some(socket) = self
            .nodes
            .get_mut(&to.node)
            .and_then(|node| node.inputs.get_mut(to.index))
        {
            socket.resolved_type = None;
        }
    }

    /// A reroute is transparent — its output should always mirror whatever
    /// flows into its input. Unlike a regular node's sockets (kept in sync by
    /// its own `on_update`), nothing else does this for a reroute, so its
    /// output's `resolved_type` — and therefore its socket-dot color *and*
    /// the color of any wire leaving it, both of which fall back to the
    /// static idle color while unresolved — used to just stay at the
    /// default gray forever. Called after a reroute's own input resolution
    /// changes; cascades forward through whatever the output feeds,
    /// including further chained reroutes.
    fn propagate_reroute_output(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(&node_id) else {
            return;
        };
        if node.kind != NodeKind::Reroute {
            return;
        }
        let input_type = node.inputs[0].effective_type().to_owned();
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        node.outputs[0].resolved_type = (input_type != "Any").then_some(input_type);

        let output = SocketId {
            node: node_id,
            index: 0,
            direction: SocketDirection::Output,
        };
        let downstream: Vec<SocketId> = self
            .connections
            .iter()
            .filter(|c| c.from == output)
            .map(|c| c.to)
            .collect();
        for to in downstream {
            self.resolve_input(output, to);
            self.propagate_reroute_output(to.node);
        }
    }

    /// Recomputes every reroute's output resolution from its current input —
    /// a one-time correction after loading a graph that may have been saved
    /// before reroute outputs propagated their resolved type at all (they
    /// used to just render flat gray, wire included).
    /// Recomputes reroute output types after loading an older saved graph.
    pub fn reconcile_reroute_outputs(&mut self) {
        let reroutes: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::Reroute)
            .map(|(&id, _)| id)
            .collect();
        for id in reroutes {
            self.propagate_reroute_output(id);
        }
    }

    // ── Variadic groups ───────────────────────────────────────────────────────

    /// Converts a just-connected placeholder into a member and spawns a fresh
    /// placeholder after it while the group is below its max.
    fn grow_variadic_group(&mut self, to: SocketId) {
        let (def_index, info, template) = {
            let Some(socket) = self
                .nodes
                .get(&to.node)
                .and_then(|node| node.inputs.get(to.index))
            else {
                return;
            };
            let Some(info) = socket.variadic.clone() else {
                return;
            };
            if !info.placeholder {
                return;
            }
            (socket.def_index, info, socket.clone())
        };
        let Some(node) = self.nodes.get_mut(&to.node) else {
            return;
        };
        let members = node
            .inputs
            .iter()
            .filter(|socket| socket.def_index == def_index && socket.is_variadic_member())
            .count();
        let number = members + 1;
        let socket = &mut node.inputs[to.index];
        socket.variadic = Some(VariadicInfo {
            placeholder: false,
            ..info.clone()
        });
        socket.name = format!("{} {}", info.base, number);
        if number < info.max {
            let mut placeholder = template;
            placeholder.resolved_type = None;
            placeholder.name = info.base;
            self.insert_input_socket(to.node, to.index + 1, placeholder);
        }
    }

    /// Connects `from` to an occupied variadic member by making room at that
    /// position instead of replacing the link already there: the member and
    /// every one after it shifts down a place, keeping its own link, and the
    /// group grows by one.
    ///
    /// Returns whether the insert applied. It does not when `to` is not an
    /// occupied member, or when the group is already at its max and has
    /// nowhere to shift into — the caller then connects normally, replacing.
    ///
    /// # Parameters
    /// - `from`: Output socket supplying the new connection.
    /// - `to`: Occupied variadic member the new connection takes the place of.
    pub fn insert_variadic_connection(&mut self, from: SocketId, to: SocketId) -> bool {
        let (def_index, info, template) = {
            let Some(socket) = self
                .nodes
                .get(&to.node)
                .and_then(|node| node.inputs.get(to.index))
            else {
                return false;
            };
            let Some(info) = socket.variadic.clone() else {
                return false;
            };
            // A placeholder is a free slot: connecting to it grows the group
            // on its own, and there is nothing to push down.
            if info.placeholder {
                return false;
            }
            (socket.def_index, info, socket.clone())
        };
        if !self
            .connections
            .iter()
            .any(|connection| connection.to == to)
        {
            return false;
        }
        let members = self.variadic_members(to.node, def_index);
        if members >= info.max {
            return false;
        }

        let mut inserted = template;
        inserted.resolved_type = None;
        self.insert_input_socket(to.node, to.index, inserted);
        self.connections.push(Connection { from, to });
        self.resolve_input(from, to);
        self.renumber_variadic_group(to.node, def_index, &info);
        self.propagate_reroute_output(to.node);
        self.mark_semantic_change();
        true
    }

    fn variadic_members(&self, node_id: NodeId, def_index: usize) -> usize {
        self.nodes.get(&node_id).map_or(0, |node| {
            node.inputs
                .iter()
                .filter(|socket| socket.def_index == def_index && socket.is_variadic_member())
                .count()
        })
    }

    /// Renumbers a group's members and retires the trailing placeholder once
    /// the group is full — the counterpart of the placeholder restoration in
    /// [`Self::collapse_variadic_member`].
    fn renumber_variadic_group(&mut self, node_id: NodeId, def_index: usize, info: &VariadicInfo) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let mut members = 0usize;
        let mut placeholder = None;
        for (index, socket) in node.inputs.iter_mut().enumerate() {
            if socket.def_index != def_index {
                continue;
            }
            if socket.is_variadic_member() {
                members += 1;
                socket.name = format!("{} {}", info.base, members);
            } else if socket.is_variadic_placeholder() {
                placeholder = Some(index);
            }
        }
        if members >= info.max
            && let Some(index) = placeholder
        {
            self.remove_input_socket(node_id, index);
        }
    }

    /// Removes a disconnected variadic member, renumbers the remaining
    /// members, and restores the trailing placeholder if the group had been
    /// at its max.
    fn collapse_variadic_member(&mut self, to: SocketId) {
        let (def_index, info) = {
            let Some(socket) = self
                .nodes
                .get(&to.node)
                .and_then(|node| node.inputs.get(to.index))
            else {
                return;
            };
            let Some(info) = socket.variadic.clone() else {
                return;
            };
            if info.placeholder {
                return;
            }
            (socket.def_index, info)
        };
        let Some(removed) = self.remove_input_socket(to.node, to.index) else {
            return;
        };

        let Some(node) = self.nodes.get_mut(&to.node) else {
            return;
        };
        let mut members = 0usize;
        let mut group_end = None;
        let mut has_placeholder = false;
        for (index, socket) in node.inputs.iter_mut().enumerate() {
            if socket.def_index != def_index {
                continue;
            }
            group_end = Some(index);
            if socket.is_variadic_member() {
                members += 1;
                socket.name = format!("{} {}", info.base, members);
            } else if socket.is_variadic_placeholder() {
                has_placeholder = true;
            }
        }
        if !has_placeholder && members < info.max {
            let mut placeholder = removed;
            placeholder.resolved_type = None;
            placeholder.name = info.base.clone();
            placeholder.variadic = Some(VariadicInfo {
                placeholder: true,
                ..info
            });
            let insert_at = group_end.map_or(to.index, |index| index + 1);
            self.insert_input_socket(to.node, insert_at, placeholder);
        }
    }

    /// Inserts an input socket, shifting the indices of existing connections
    /// into this node accordingly.
    fn insert_input_socket(&mut self, node_id: NodeId, index: usize, socket: Socket) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let index = index.min(node.inputs.len());
        node.inputs.insert(index, socket);
        for connection in &mut self.connections {
            if connection.to.node == node_id && connection.to.index >= index {
                connection.to.index += 1;
            }
        }
    }

    /// Removes an input socket, shifting the indices of existing connections
    /// into this node accordingly. Any connection to the removed socket is
    /// dropped.
    fn remove_input_socket(&mut self, node_id: NodeId, index: usize) -> Option<Socket> {
        let node = self.nodes.get_mut(&node_id)?;
        if index >= node.inputs.len() {
            return None;
        }
        let socket = node.inputs.remove(index);
        self.connections
            .retain(|connection| !(connection.to.node == node_id && connection.to.index == index));
        for connection in &mut self.connections {
            if connection.to.node == node_id && connection.to.index > index {
                connection.to.index -= 1;
            }
        }
        Some(socket)
    }

    /// Reverts inputs of `ids` that have no incoming connection — used after
    /// pasting, where a socket may have been copied resolved (or as a grown
    /// variadic member) while its feeding connection was not part of the
    /// payload.
    ///
    /// # Parameters
    /// - `ids`: Nodes whose copied or restored inputs must be reconciled with their connections.
    pub fn prune_unconnected_resolutions(&mut self, ids: &[NodeId]) {
        for &id in ids {
            // Collapse unconnected variadic members one at a time: each
            // removal shifts indices, so recompute between iterations.
            loop {
                let connected: HashSet<usize> = self
                    .connections
                    .iter()
                    .filter(|connection| connection.to.node == id)
                    .map(|connection| connection.to.index)
                    .collect();
                let Some(node) = self.nodes.get(&id) else {
                    break;
                };
                let victim = node.inputs.iter().enumerate().find_map(|(index, socket)| {
                    (socket.is_variadic_member() && !connected.contains(&index)).then_some(index)
                });
                let Some(index) = victim else {
                    break;
                };
                self.collapse_variadic_member(SocketId {
                    node: id,
                    index,
                    direction: SocketDirection::Input,
                });
            }

            let connected: HashSet<usize> = self
                .connections
                .iter()
                .filter(|connection| connection.to.node == id)
                .map(|connection| connection.to.index)
                .collect();
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            for (index, socket) in node.inputs.iter_mut().enumerate() {
                if socket.resolved_type.is_some() && !connected.contains(&index) {
                    socket.resolved_type = None;
                }
            }
        }
    }

    /// Returns whether an input socket currently has an incoming connection.
    ///
    /// # Parameters
    /// - `socket`: Input socket identity to test.
    pub fn is_input_connected(&self, socket: SocketId) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.to == socket)
    }

    /// Returns whether an output socket currently has an outgoing connection.
    ///
    /// # Parameters
    /// - `socket`: Output socket identity to test.
    pub fn is_output_connected(&self, socket: SocketId) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.from == socket)
    }

    /// Returns node identities in stable ascending document order.
    pub fn sorted_node_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// Adds a visual frame group and returns its new stable identity.
    ///
    /// # Parameters
    /// - `label`: User-facing frame title.
    /// - `color`: Frame accent color.
    /// - `node_ids`: Initial nodes grouped by the frame.
    pub fn add_frame(
        &mut self,
        label: String,
        color: GraphColor,
        node_ids: Vec<NodeId>,
    ) -> FrameId {
        let id = FrameId(self.metadata.next_frame_id);
        self.metadata.next_frame_id += 1;
        self.frames.push(Frame {
            id,
            label,
            color,
            node_ids,
            selected: false,
        });
        id
    }

    /// Removes missing node identities from frames and drops empty frame groups.
    pub fn cleanup_frames(&mut self) {
        let alive: HashSet<NodeId> = self.nodes.keys().copied().collect();
        for frame in &mut self.frames {
            frame.node_ids.retain(|id| alive.contains(id));
        }
        self.frames.retain(|frame| !frame.node_ids.is_empty());
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{GraphColor, GraphPosition, NodeKind, Socket, SocketDirection, SocketShape};

    fn socket(type_name: &str, allowed: &[&str]) -> Socket {
        Socket {
            schema_id: String::new(),
            name: String::new(),
            type_name: type_name.to_owned(),
            color: GraphColor::GRAY,
            shape: SocketShape::Circle,
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            resolved_type: None,
            def_index: 0,
            variadic: None,
            visible: true,
            editor_visible: true,
            hidden: false,
            has_control: false,
            extensions: Default::default(),
        }
    }

    fn node_with_sockets(id: NodeId, inputs: Vec<Socket>, outputs: Vec<Socket>) -> Node {
        let mut node = Node::new_reroute(id, GraphPosition::ZERO);
        node.kind = NodeKind::Regular;
        node.inputs = inputs;
        node.outputs = outputs;
        node
    }

    fn sid(node: NodeId, index: usize, direction: SocketDirection) -> SocketId {
        SocketId {
            node,
            index,
            direction,
        }
    }

    #[test]
    fn socket_accepts_native_allowed_and_any() {
        let s = socket("Signal", &["Float", "Int"]);
        assert!(s.accepts("Signal"));
        assert!(s.accepts("Float"));
        assert!(s.accepts("Int"));
        assert!(s.accepts("Any"));
        assert!(!s.accepts("Protocol"));
    }

    #[test]
    fn connect_resolves_and_disconnect_reverts() {
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Float", &[])]));
        graph.add_node(node_with_sockets(
            dst,
            vec![socket("Signal", &["Float"])],
            vec![],
        ));

        let from = sid(src, 0, SocketDirection::Output);
        let to = sid(dst, 0, SocketDirection::Input);
        assert!(!graph.is_output_connected(from));
        graph.add_connection(from, to);
        assert!(graph.is_output_connected(from));
        assert_eq!(
            graph.nodes[&dst].inputs[0].resolved_type.as_deref(),
            Some("Float")
        );
        assert_eq!(graph.nodes[&dst].inputs[0].effective_type(), "Float");

        assert!(graph.disconnect_input(to));
        assert!(!graph.is_output_connected(from));
        assert_eq!(graph.nodes[&dst].inputs[0].resolved_type, None);
        assert_eq!(graph.nodes[&dst].inputs[0].effective_type(), "Signal");
    }

    #[test]
    fn reroute_output_mirrors_its_input_when_connected_and_disconnected() {
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let reroute_id = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Float", &[])]));
        graph.add_node(Node::new_reroute(reroute_id, GraphPosition::ZERO));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(reroute_id, 0, SocketDirection::Input),
        );
        assert_eq!(
            graph.nodes[&reroute_id].outputs[0].resolved_type.as_deref(),
            Some("Float"),
            "the reroute's output should mirror its resolved input type"
        );
        assert_eq!(
            graph.nodes[&reroute_id].outputs[0].effective_type(),
            "Float"
        );

        graph.disconnect_input(sid(reroute_id, 0, SocketDirection::Input));
        assert_eq!(
            graph.nodes[&reroute_id].outputs[0].resolved_type, None,
            "disconnecting the input should revert the output back to Any"
        );
    }

    #[test]
    fn reroute_output_propagation_cascades_through_a_chain() {
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let reroute_a = graph.next_id();
        let reroute_b = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Words", &[])]));
        graph.add_node(Node::new_reroute(reroute_a, GraphPosition::ZERO));
        graph.add_node(Node::new_reroute(reroute_b, GraphPosition::ZERO));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(reroute_a, 0, SocketDirection::Input),
        );
        graph.add_connection(
            sid(reroute_a, 0, SocketDirection::Output),
            sid(reroute_b, 0, SocketDirection::Input),
        );

        assert_eq!(graph.nodes[&reroute_a].outputs[0].effective_type(), "Words");
        assert_eq!(
            graph.nodes[&reroute_b].inputs[0].effective_type(),
            "Words",
            "reroute B's input should have resolved from A's now-correct output"
        );
        assert_eq!(
            graph.nodes[&reroute_b].outputs[0].effective_type(),
            "Words",
            "propagation should cascade all the way through the chain"
        );
    }

    #[test]
    fn reconcile_reroute_outputs_corrects_a_stale_load() {
        // Simulates a graph saved before reroute outputs propagated at all:
        // the connection and the input's resolution are both present and
        // correct, but the output was never updated to match.
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let reroute_id = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Trigger", &[])]));
        graph.add_node(Node::new_reroute(reroute_id, GraphPosition::ZERO));
        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(reroute_id, 0, SocketDirection::Input),
        );
        // Force it back to the stale/buggy state after the (correct) connect.
        graph.nodes.get_mut(&reroute_id).unwrap().outputs[0].resolved_type = None;
        assert_eq!(graph.nodes[&reroute_id].outputs[0].effective_type(), "Any");

        graph.reconcile_reroute_outputs();

        assert_eq!(
            graph.nodes[&reroute_id].outputs[0].effective_type(),
            "Trigger"
        );
    }

    #[test]
    fn connect_same_type_does_not_resolve() {
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Signal", &[])]));
        graph.add_node(node_with_sockets(
            dst,
            vec![socket("Signal", &["Float"])],
            vec![],
        ));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );
        assert_eq!(graph.nodes[&dst].inputs[0].resolved_type, None);
    }

    #[test]
    fn removing_source_node_reverts_downstream_inputs() {
        let mut graph = GraphState::default();
        let src = graph.next_id();
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(src, vec![], vec![socket("Int", &[])]));
        graph.add_node(node_with_sockets(
            dst,
            vec![socket("Signal", &["Int"])],
            vec![],
        ));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );
        assert!(graph.nodes[&dst].inputs[0].resolved_type.is_some());

        graph.remove_node(src);
        assert!(graph.connections.is_empty());
        assert_eq!(graph.nodes[&dst].inputs[0].resolved_type, None);
    }

    #[test]
    fn prune_clears_resolution_without_connection() {
        let mut graph = GraphState::default();
        let dst = graph.next_id();
        let mut node = node_with_sockets(dst, vec![socket("Signal", &["Float"])], vec![]);
        node.inputs[0].resolved_type = Some("Float".to_owned());
        graph.add_node(node);

        graph.prune_unconnected_resolutions(&[dst]);
        assert_eq!(graph.nodes[&dst].inputs[0].resolved_type, None);
    }

    fn variadic_placeholder(type_name: &str, base: &str, max: usize) -> Socket {
        let mut s = socket(type_name, &[]);
        s.name = base.to_owned();
        s.variadic = Some(VariadicInfo {
            base: base.to_owned(),
            max,
            placeholder: true,
        });
        s
    }

    /// Source node with `count` Signal outputs.
    fn source(graph: &mut GraphState, count: usize) -> NodeId {
        let id = graph.next_id();
        let outputs = (0..count).map(|_| socket("Signal", &[])).collect();
        graph.add_node(node_with_sockets(id, vec![], outputs));
        id
    }

    #[test]
    fn connecting_placeholder_grows_group() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 4)],
            vec![],
        ));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );

        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 2);
        assert!(inputs[0].is_variadic_member());
        assert_eq!(inputs[0].name, "Ch 1");
        assert!(inputs[1].is_variadic_placeholder());
        assert_eq!(inputs[1].name, "Ch");

        graph.add_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        );
        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[1].name, "Ch 2");
        assert!(inputs[2].is_variadic_placeholder());
    }

    #[test]
    fn connecting_an_occupied_member_inserts_and_pushes_the_rest_down() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 3);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 4)],
            vec![],
        ));
        for index in 0..2 {
            graph.add_connection(
                sid(src, index, SocketDirection::Output),
                sid(dst, index, SocketDirection::Input),
            );
        }

        // A third link lands on the first, occupied member.
        assert!(graph.insert_variadic_connection(
            sid(src, 2, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        ));

        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 4);
        assert_eq!(
            inputs
                .iter()
                .map(|socket| socket.name.as_str())
                .collect::<Vec<_>>(),
            ["Ch 1", "Ch 2", "Ch 3", "Ch"]
        );
        assert!(inputs[3].is_variadic_placeholder());
        // Every link kept its own source, one place further down.
        let source_of = |index: usize| {
            graph
                .connections
                .iter()
                .find(|connection| connection.to == sid(dst, index, SocketDirection::Input))
                .map(|connection| connection.from.index)
        };
        assert_eq!(source_of(0), Some(2));
        assert_eq!(source_of(1), Some(0));
        assert_eq!(source_of(2), Some(1));
    }

    #[test]
    fn a_full_group_replaces_instead_of_inserting() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 3);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 2)],
            vec![],
        ));
        for index in 0..2 {
            graph.add_connection(
                sid(src, index, SocketDirection::Output),
                sid(dst, index, SocketDirection::Input),
            );
        }

        // Nowhere to shift into: the caller falls back to a plain connect.
        assert!(!graph.insert_variadic_connection(
            sid(src, 2, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        ));
        assert_eq!(graph.nodes[&dst].inputs.len(), 2);
        assert_eq!(graph.connections.len(), 2);
    }

    #[test]
    fn an_unoccupied_or_plain_socket_is_left_to_the_ordinary_connect() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![
                socket("Signal", &[]),
                variadic_placeholder("Signal", "Ch", 4),
            ],
            vec![],
        ));
        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );

        // An occupied but non-variadic input still replaces.
        assert!(!graph.insert_variadic_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        ));
        // A placeholder grows the group by itself.
        assert!(!graph.insert_variadic_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        ));
    }

    #[test]
    fn group_stops_growing_at_max() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 2)],
            vec![],
        ));

        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );
        graph.add_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        );

        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 2);
        assert!(inputs.iter().all(Socket::is_variadic_member));
    }

    #[test]
    fn disconnecting_member_removes_it_and_renumbers() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 3);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 4)],
            vec![],
        ));
        for i in 0..3 {
            graph.add_connection(
                sid(src, i, SocketDirection::Output),
                sid(dst, i, SocketDirection::Input),
            );
        }
        assert_eq!(graph.nodes[&dst].inputs.len(), 4);

        // Remove the middle member; the two remaining members renumber and
        // the connection into "Ch 3" shifts down to keep pointing at it.
        graph.disconnect_input(sid(dst, 1, SocketDirection::Input));
        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0].name, "Ch 1");
        assert_eq!(inputs[1].name, "Ch 2");
        assert!(inputs[2].is_variadic_placeholder());
        assert_eq!(graph.connections.len(), 2);
        assert!(
            graph
                .connections
                .iter()
                .any(|c| c.from.index == 2 && c.to.index == 1)
        );
    }

    #[test]
    fn disconnecting_member_at_max_restores_placeholder() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 2)],
            vec![],
        ));
        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );
        graph.add_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        );

        graph.disconnect_input(sid(dst, 0, SocketDirection::Input));
        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 2);
        assert!(inputs[0].is_variadic_member());
        assert_eq!(inputs[0].name, "Ch 1");
        assert!(inputs[1].is_variadic_placeholder());
    }

    #[test]
    fn removing_source_collapses_variadic_members() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 4)],
            vec![],
        ));
        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );
        graph.add_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        );

        graph.remove_node(src);
        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].is_variadic_placeholder());
        assert!(graph.connections.is_empty());
    }

    #[test]
    fn variadic_grow_shifts_connections_of_later_inputs() {
        let mut graph = GraphState::default();
        let src = source(&mut graph, 2);
        let dst = graph.next_id();
        // Variadic group followed by a static input.
        let mut static_input = socket("Signal", &[]);
        static_input.def_index = 1;
        graph.add_node(node_with_sockets(
            dst,
            vec![variadic_placeholder("Signal", "Ch", 4), static_input],
            vec![],
        ));

        // Connect the static input first (index 1), then grow the group.
        graph.add_connection(
            sid(src, 0, SocketDirection::Output),
            sid(dst, 1, SocketDirection::Input),
        );
        graph.add_connection(
            sid(src, 1, SocketDirection::Output),
            sid(dst, 0, SocketDirection::Input),
        );

        // The placeholder insert shifted the static input to index 2.
        let inputs = &graph.nodes[&dst].inputs;
        assert_eq!(inputs.len(), 3);
        assert!(inputs[1].is_variadic_placeholder());
        assert!(
            graph
                .connections
                .iter()
                .any(|c| c.from.index == 0 && c.to.index == 2)
        );
    }

    #[test]
    fn graph_round_trips_node_data() {
        let mut graph = GraphState::default();
        let id = graph.next_id();
        graph.add_node(Node::new_reroute(id, GraphPosition::ZERO));

        let json = serde_json::to_string(&graph).expect("graph state should serialize");
        let loaded: GraphState =
            serde_json::from_str(&json).expect("graph state should deserialize");

        assert_eq!(loaded.nodes[&id].kind, NodeKind::Reroute);
    }

    #[test]
    fn semantic_snapshot_ignores_editor_layout_but_tracks_processing_state() {
        let mut graph = GraphState::default();
        let id = graph.next_id();
        graph.add_node(Node::blank(id, "Test Node", GraphPosition::ZERO));
        let original = graph.semantic_snapshot();

        let node = graph.nodes.get_mut(&id).unwrap();
        node.pos = GraphPosition::new(100.0, 200.0);
        node.selected = true;
        node.collapsed = true;
        node.title = "Renamed".into();
        node.header_color = GraphColor::RED;
        graph.add_frame("Group".into(), GraphColor::BLUE, vec![id]);
        assert_eq!(graph.semantic_snapshot(), original);

        graph.nodes.get_mut(&id).unwrap().state = serde_json::json!({"threshold": 7});
        assert_ne!(graph.semantic_snapshot(), original);
    }

    #[test]
    fn semantic_revision_tracks_processing_edits_but_not_presentation_edits() {
        let mut graph = GraphState::default();
        let id = graph.next_id();
        graph.add_node(Node::blank(id, "Test Node", GraphPosition::ZERO));
        let after_node = graph.semantic_revision();

        graph.nodes.get_mut(&id).unwrap().pos = GraphPosition::new(100.0, 200.0);
        graph
            .set_extension("example.panel", serde_json::json!({"width": 300}))
            .unwrap();
        assert_eq!(graph.semantic_revision(), after_node);

        assert!(
            graph
                .set_semantic_extension("example.outputs", serde_json::json!([id.0]))
                .unwrap()
        );
        let after_output = graph.semantic_revision();
        assert!(after_output > after_node);
        assert!(
            !graph
                .set_semantic_extension("example.outputs", serde_json::json!([id.0]))
                .unwrap()
        );
        assert_eq!(graph.semantic_revision(), after_output);

        assert!(graph.remove_semantic_extension("example.outputs"));
        assert!(graph.semantic_revision() > after_output);
    }

    #[test]
    fn replacement_revision_advances_beyond_undo_snapshots() {
        let mut current = GraphState::default();
        current.mark_semantic_change();
        current.mark_semantic_change();
        let previous = current.semantic_revision();

        let mut replacement = GraphState::default();
        replacement.mark_semantic_change_after(previous);

        assert!(replacement.semantic_revision() > previous);

        let encoded = serde_json::to_vec(&replacement).unwrap();
        let restored: GraphState = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.semantic_revision(), 0);
    }

    #[test]
    fn opaque_document_and_socket_extensions_round_trip() {
        let mut graph = GraphState::default();
        assert!(
            !serde_json::to_value(&graph)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("extensions")
        );

        let document_value = serde_json::json!({
            "version": 37,
            "plugin_owned": {"node": 7, "future_field": [1, 2, 3]}
        });
        graph
            .set_extension("example.selection", &document_value)
            .unwrap();
        let id = graph.next_id();
        let mut reroute = Node::new_reroute(id, GraphPosition::ZERO);
        reroute.inputs[0].extensions.insert(
            "example.socket-presentation".to_owned(),
            serde_json::json!({"version": 9, "local_style": "compact"}),
        );
        graph.add_node(reroute);
        let json = serde_json::to_string(&graph).unwrap();
        let loaded: GraphState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded
                .extension::<serde_json::Value>("example.selection")
                .unwrap(),
            Some(document_value)
        );
        assert_eq!(
            loaded.nodes[&id].inputs[0]
                .extensions
                .get("example.socket-presentation"),
            Some(&serde_json::json!({
                "version": 9,
                "local_style": "compact"
            }))
        );

        let legacy: GraphState = serde_json::from_str(
            r#"{"nodes":{},"connections":[],"frames":[],"next_id":0,"next_frame_id":0}"#,
        )
        .unwrap();
        assert_eq!(
            legacy.extension::<NodeId>("example.selection").unwrap(),
            None
        );
    }
}
