//! # `node_graph::api`
//!
//! ## Responsibility
//!
//! This namespace owns the graph-document, node-definition, socket-definition, control, and portable
//! file-dialog contracts consumed by graph-node features and graph compilation.
//!
//! ## Boundaries
//!
//! It does not expose editor implementation operations, concrete payload types, compiler policy, or a
//! native/web dialog backend. The crate root remains the editor-widget composition surface.

//! Supported graph documents and contracts implemented by node definitions.
//!
//! This is the compiler- and plugin-facing graph-document namespace. It owns graph
//! identities, node and socket definitions, persistence reconciliation, and the
//! portable file-dialog contract. Editor interaction and widget composition remain
//! at the crate root; concrete node semantics remain outside `node_graph`.
//!
//! # Getting started
//!
//! An embedder registers each supported [`NodeDef`] with a
//! [`crate::NodeTypeRegistry`], then gives that registry to a
//! [`crate::NodeGraphWidget`]. The widget owns the editable [`GraphState`] document,
//! undo history, clipboard, interaction, and built-in Node panel. The host reads the
//! graph document for lowering and persists the result of its synchronization API.
//!
//! ```ignore
//! let mut registry = node_graph::NodeTypeRegistry::new();
//! registry.register::<MyNode>();
//! let mut widget = node_graph::NodeGraphWidget::new(registry);
//! // In the egui frame:
//! widget.show(ui);
//! ```
//!
//! # Concepts and terminology
//!
//! A **node definition** is the static [`NodeDef`] implementation for a node type.
//! Its serde **state** is serialized in the document, while its input/output and
//! property definitions are rebuilt from that state. A **socket type** is a
//! [`SocketDef`] with a stable identity, color, and shape. Socket compatibility is
//! declarative: an input may accept additional types, but the concrete node or
//! compiler owns the meaning of that connection.
//!
//! [`GraphState`] is plain serde data containing nodes, connections, frames, and
//! namespaced extensions. Extensions preserve host-owned document data without the
//! generic graph model interpreting it. A [`Socket`] has a native type and may have
//! a resolved type after an accepted alternative connection. A **variadic** input is
//! a growing input group with a trailing placeholder; connecting it creates a member.
//!
//! # Defining nodes and sockets
//!
//! Implement [`SocketDef`] once for every application socket identity. Built-in
//! `BoolSocket`, `IntSocket`, `FloatSocket`, `StrSocket`, and `FileSocket` support
//! inline controls; `AnySocket` is the explicit wildcard used by reroutes. The
//! serializable built-in value types are intended for embedding directly in node
//! state. [`SocketWithControlDef`] associates a socket with an [`InlineControl`] for
//! an unconnected input.
//!
//! A [`NodeDef`] supplies its initial state, inputs, outputs, inline properties,
//! property-panel sections, and optional node-contributed panels. [`InputDef`] and
//! [`OutputDef`] builders declare the socket type; an input's `accepts` declaration
//! makes a secondary compatible type explicit. [`NodeDef::on_update`] reconciles
//! dynamic state after edits, connection changes, or restoration, while
//! [`NodeDef::badge`] supplies a node-owned status badge.
//!
//! Node-contributed panels are deliberately data-agnostic. A host supplies transient
//! typed models through [`PanelDataProvider`]; panel presentations receive them only
//! for the current draw and emit opaque [`PanelAction`] values for the host to handle.
//! This keeps concrete protocol and application data outside the generic widget.
//!
//! # Host integration
//!
//! [`crate::NodeGraphWidget::show_with_panel_data`] is the full per-frame entry point
//! for hosts with contributed panels; [`crate::NodeGraphWidget::show`] uses an empty
//! provider. The root facade also exposes document replacement, programmatic node
//! creation, state replacement, external badges/statuses, and socket indicators.
//! The host owns graph compilation, storage, file-dialog implementation, concrete
//! node behavior, and all interpretation of panel actions.

mod builtins;
mod control;
mod document;
mod indicator;
mod node;
mod panel;
mod socket;

pub use builtins::{
    AnySocket, BoolSocket, BoolValue, EnumValue, FileSocket, FileValue, FloatSocket, FloatValue,
    IntSocket, IntValue, StrSocket, StringValue,
};
pub(crate) use control::UnavailableFileDialogService;
pub use control::{
    DroppedFile, FileDialogFilter, FileDialogProgress, FileDialogRequest, FileDialogService,
    InlineControl, InlineControlContext,
};
pub use document::GraphDocumentBuilder;
pub use indicator::SocketIndicatorPresentation;
pub use node::{
    InputDef, NodeDef, NodeInstanceSchema, OutputDef, PanelSection, PropDef, SocketTypeIdentity,
};
pub use panel::{
    NodePanelDef, NodePanelPresentation, PanelAction, PanelContext, PanelDataProvider,
    PanelMetadata, PanelTabDef, PropertyPanelPresentation,
};
pub use socket::{SocketDef, SocketWithControlDef};

pub use crate::model::{
    Connection, GraphColor, GraphMetadata, GraphPosition, GraphState, Node, NodeId, NodeKind,
    Socket, SocketDirection, SocketId, SocketReference, SocketShape, VariadicInfo,
};
pub use crate::runtime::{NodeTemplate, NodeTypeRegistry, SocketTypeStyle};
