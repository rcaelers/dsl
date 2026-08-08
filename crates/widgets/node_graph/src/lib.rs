//! Generic graph document model and egui editor widget.
//!
//! [`api`] contains the compiler- and plugin-facing document and node-definition
//! contracts. The crate-root facade supplies editor composition. Concrete node
//! behavior, graph compiler policy, protocol semantics, and host adapters remain
//! outside this reusable widget crate.

#[cfg(test)]
mod architecture_tests;

pub mod api;
mod model;
mod runtime;
mod support;
mod widget;

pub use api::{
    AnySocket, BoolSocket, BoolValue, DroppedFile, EnumValue, FileDialogError, FileDialogFilter,
    FileDialogProgress, FileDialogRequest, FileDialogService, FileSocket, FileValue, FloatSocket,
    FloatValue, InlineControl, InlineControlContext, InputDef, IntSocket, IntValue, NodeDef,
    NodeInstanceSchema, NodePanelDef, NodePanelPresentation, OutputDef, PanelAction, PanelContext,
    PanelDataProvider, PanelMetadata, PanelSection, PanelTabDef, PropDef,
    PropertyPanelPresentation, SocketDef, SocketIndicatorPresentation, SocketTypeIdentity,
    SocketWithControlDef, StrSocket, StringValue,
};
pub use model::{
    BadgeSeverity, Connection, Frame, FrameId, GraphColor, GraphMetadata, GraphPosition,
    GraphState, Node, NodeBadge, NodeId, NodeKind, NodeMetadata, Socket, SocketDirection, SocketId,
    SocketReference, SocketShape, VariadicInfo,
};
pub use runtime::{NodeTemplate, NodeTypeRegistry, SocketTypeStyle};
pub use widget::{GraphUiPrefs, NodeContextAction, NodeGraphWidget};
