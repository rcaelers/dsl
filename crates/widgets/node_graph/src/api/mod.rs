//! Supported graph documents and contracts implemented by node definitions.

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
    Connection, GraphMetadata, GraphState, Node, NodeId, NodeKind, Socket, SocketDirection,
    SocketId, SocketShape, VariadicInfo,
};
pub use crate::runtime::{NodeTemplate, NodeTypeRegistry, SocketTypeStyle};
