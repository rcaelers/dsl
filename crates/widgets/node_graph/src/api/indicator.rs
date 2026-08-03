use egui::{Painter, Rect, Vec2};

/// Draws one transient decoration next to an input or output socket.
///
/// Sizes and drawing coordinates are in screen points. The graph widget owns
/// placement and passes the current graph zoom so presentations may scale or
/// deliberately remain screen-sized.
pub trait SocketIndicatorPresentation: Send + Sync + 'static {
    /// Returns the decoration's desired screen-point size at the current graph zoom.
    ///
    /// # Parameters
    /// - `zoom`: Current graph canvas zoom factor.
    fn size(&self, zoom: f32) -> Vec2;
    /// Draws the decoration in the rectangle allocated by the graph widget.
    ///
    /// # Parameters
    /// - `painter`: Painter clipped and layered by the graph widget.
    /// - `rect`: Screen-space rectangle allocated for this decoration.
    /// - `zoom`: Current graph canvas zoom factor.
    fn draw(&self, painter: &Painter, rect: Rect, zoom: f32);
}
