use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::{NodeId, SocketDirection};
use super::presentation::{GraphColor, GraphPosition};
use super::socket::{Socket, SocketReference, SocketShape};

/// Structural role of a node in the graph editor.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum NodeKind {
    #[default]
    /// Normal user-visible node backed by a registered node definition.
    Regular,
    /// Connection-routing node without concrete runtime behavior.
    Reroute,
}

/// Per-node status message rendered under the node body: def-driven
/// validation notes (a clamped setting, an invalid pattern) or externally
/// set compile/runtime errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeBadge {
    /// User-presentable status message.
    pub text: String,
    /// Severity used for badge styling and presentation.
    pub severity: BadgeSeverity,
}

/// Severity of a node validation or runtime status badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BadgeSeverity {
    /// Informational state that does not need user action.
    Info,
    /// State the user may need to review.
    Warning,
    /// Error preventing correct graph behavior.
    Error,
}

impl NodeBadge {
    /// Creates an informational node badge.
    ///
    /// # Parameters
    /// - `text`: User-presentable status message.
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: BadgeSeverity::Info,
        }
    }
    /// Creates a warning node badge.
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: BadgeSeverity::Warning,
        }
    }
    /// Creates an error node badge.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: BadgeSeverity::Error,
        }
    }
}

/// Persisted editor state for one graph node.
#[derive(Serialize, Deserialize)]
pub struct Node {
    /// Stable node identity in the graph document.
    pub id: NodeId,
    /// Structural editor role of this node.
    pub kind: NodeKind,
    /// Display name; user-renamable. The registered def is identified by
    /// `type_name`, never by the title.
    pub title: String,
    /// Registered node-type name. Empty in files saved before renaming
    /// existed; those fall back to `title` (which then still equals it).
    #[serde(default)]
    pub type_name: String,
    /// Definition-provided header color.
    pub header_color: GraphColor,
    /// Position in graph-canvas coordinates.
    pub pos: GraphPosition,
    /// Materialized input sockets.
    pub inputs: Vec<Socket>,
    /// Materialized output sockets.
    pub outputs: Vec<Socket>,
    #[serde(default)]
    /// Whether the editor renders this node in collapsed form.
    pub collapsed: bool,
    /// Bypassed for compilation (Phase 3): the compiler splices its
    /// compatible inputs directly to its outputs and drops the node, rather
    /// than building it. Non-destructive — the node, its config, and its
    /// wires all stay in the graph; toggling again restores it.
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    /// Concrete node-owned persisted configuration state.
    pub state: Value,
    #[serde(flatten)]
    /// Generic editor metadata not owned by the node definition.
    pub metadata: NodeMetadata,
    /// Def-driven status message, recomputed on every state update.
    #[serde(skip)]
    pub badge: Option<NodeBadge>,
    /// Whether the editor currently selects this node.
    pub selected: bool,
}

/// Generic non-persisted metadata associated with one node instance.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NodeMetadata {
    #[serde(skip)]
    property_count: usize,
}

impl NodeMetadata {
    /// Creates transient node metadata for a materialized editor definition.
    ///
    /// This value is excluded from saved documents and semantic snapshots.
    #[doc(hidden)]
    pub fn with_property_count(property_count: usize) -> Self {
        Self { property_count }
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            kind: self.kind.clone(),
            title: self.title.clone(),
            type_name: self.type_name.clone(),
            header_color: self.header_color,
            pos: self.pos,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            collapsed: self.collapsed,
            muted: self.muted,
            state: self.state.clone(),
            metadata: self.metadata.clone(),
            badge: self.badge.clone(),
            selected: self.selected,
        }
    }
}

impl Node {
    /// Returns the transient property-row count supplied by the editor definition.
    #[doc(hidden)]
    pub fn property_count(&self) -> usize {
        self.metadata.property_count
    }

    /// Replaces the transient property-row count after definition reconciliation.
    #[doc(hidden)]
    pub fn set_property_count(&mut self, property_count: usize) {
        self.metadata.property_count = property_count;
    }

    /// The registered node-type name this node was created from.
    pub fn def_name(&self) -> &str {
        if self.type_name.is_empty() {
            &self.title
        } else {
            &self.type_name
        }
    }

    /// Returns the semantic identity of one materialized input or output.
    ///
    /// Variadic member positions are derived from the materialized sockets so callers do not
    /// need to reproduce document indexing rules.
    pub fn socket_reference(
        &self,
        direction: SocketDirection,
        socket_index: usize,
    ) -> Option<SocketReference<'_>> {
        let sockets = match direction {
            SocketDirection::Input => &self.inputs,
            SocketDirection::Output => &self.outputs,
        };
        let socket = sockets.get(socket_index)?;
        let member_index = if socket.is_variadic_member() {
            sockets[..socket_index]
                .iter()
                .filter(|other| other.def_index == socket.def_index && other.is_variadic_member())
                .count()
        } else {
            0
        };
        Some(socket.reference(direction, member_index))
    }

    /// The input↔output pairing a muted node bypasses through: for each
    /// output (in order), the earliest not-yet-claimed input whose type is
    /// compatible with it. Purely a function of this node's own declared
    /// sockets — independent of whatever happens to be wired upstream or
    /// downstream. Mirrors Blender: muting only usefully bypasses a node
    /// whose input and output share a type (e.g. `Buffer`'s `Any`/`Any`); a
    /// type-transforming node (Signal → Words) has no such pair, so muting
    /// it drops its output rather than faking one.
    pub fn mute_pass_through_pairs(&self) -> Vec<(usize, usize)> {
        let mut used = vec![false; self.inputs.len()];
        let mut pairs = Vec::new();
        for (out_idx, output) in self.outputs.iter().enumerate() {
            let Some(in_idx) = self
                .inputs
                .iter()
                .enumerate()
                .position(|(i, input)| !used[i] && input.accepts(output.effective_type()))
            else {
                continue;
            };
            used[in_idx] = true;
            pairs.push((out_idx, in_idx));
        }
        pairs
    }
}

impl Node {
    /// Creates an untyped built-in reroute node at a graph-space position.
    ///
    /// # Parameters
    /// - `id`: Stable document identity assigned by the graph.
    /// - `pos`: Initial graph-canvas position.
    pub fn new_reroute(id: NodeId, pos: GraphPosition) -> Self {
        let input = Socket {
            schema_id: String::new(),
            name: String::new(),
            type_name: "Any".to_string(),
            color: GraphColor::from_rgb(150, 150, 150),
            shape: SocketShape::Circle,
            allowed: Vec::new(),
            resolved_type: None,
            def_index: 0,
            variadic: None,
            visible: true,
            editor_visible: true,
            hidden: false,
            has_control: false,
            extensions: Default::default(),
        };
        let output = input.clone();
        Self {
            id,
            kind: NodeKind::Reroute,
            title: String::new(),
            type_name: String::new(),
            header_color: GraphColor::from_rgb(80, 80, 80),
            pos,
            inputs: vec![input],
            outputs: vec![output],
            collapsed: false,
            muted: false,
            state: Value::Null,
            metadata: NodeMetadata::default(),
            badge: None,
            selected: false,
        }
    }

    /// A regular node with no properties panel, its sockets and state left
    /// for the caller to fill in. For building a `Node` directly outside the
    /// widget/registry path — e.g. a compiler-generated collector,
    /// which is never rendered and so has no properties panel to size.
    pub fn blank(id: NodeId, type_name: impl Into<String>, pos: GraphPosition) -> Self {
        let type_name = type_name.into();
        Self {
            id,
            kind: NodeKind::Regular,
            title: type_name.clone(),
            type_name,
            header_color: GraphColor::from_rgb(80, 80, 80),
            pos,
            inputs: Vec::new(),
            outputs: Vec::new(),
            collapsed: false,
            muted: false,
            state: Value::Null,
            metadata: NodeMetadata::default(),
            badge: None,
            selected: false,
        }
    }
}
