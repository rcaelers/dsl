use egui::Color32;

use super::control::InlineControl;
use crate::model::SocketShape;

/// Compile-time definition of a graph socket payload family.
pub trait SocketDef: 'static + Send + Sync {
    /// Rust value carried by sockets of this family.
    type Value: 'static + Send + Sync;

    /// Returns the stable user-facing payload type name.
    fn type_name() -> &'static str
    where
        Self: Sized;
    /// Returns the color used to render sockets of this family.
    fn color() -> Color32
    where
        Self: Sized;
    /// Returns the socket shape used to distinguish this family.
    fn shape() -> SocketShape
    where
        Self: Sized,
    {
        SocketShape::Circle
    }
}

/// Socket family that provides an inline editor for an unconnected input.
pub trait SocketWithControlDef: SocketDef {
    /// Inline control type that edits the socket's default value.
    type Control: InlineControl;
}
