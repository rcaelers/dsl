#[cfg(test)]
mod architecture_tests;

pub mod api;
mod model;
mod runtime;
mod support;
mod widget;

pub use api::{
    AnySocket, BoolSocket, BoolValue, DroppedFile, EnumValue, FileDialogFilter, FileDialogProgress,
    FileDialogRequest, FileDialogService, FileSocket, FileValue, FloatSocket, FloatValue,
    InlineControl, InlineControlContext, InputDef, IntSocket, IntValue, NodeDef,
    NodeInstanceSchema, NodePanelDef, NodePanelPresentation, OutputDef, PanelAction, PanelContext,
    PanelDataProvider, PanelMetadata, PanelSection, PanelTabDef, PropDef,
    PropertyPanelPresentation, SocketDef, SocketIndicatorPresentation, SocketTypeIdentity,
    SocketWithControlDef, StrSocket, StringValue,
};
pub use model::{
    BadgeSeverity, Connection, Frame, FrameId, GraphMetadata, GraphState, Node, NodeBadge, NodeId,
    NodeKind, NodeMetadata, Socket, SocketDirection, SocketId, SocketShape, VariadicInfo,
};
pub use runtime::{NodeTemplate, NodeTypeRegistry, SocketTypeStyle};
pub use widget::{GraphUiPrefs, NodeContextAction, NodeGraphWidget};
