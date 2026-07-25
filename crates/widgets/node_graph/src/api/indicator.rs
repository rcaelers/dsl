use egui::{Painter, Rect, Vec2};

/// Draws one transient decoration next to an input or output socket.
///
/// Sizes and drawing coordinates are in screen points. The graph widget owns
/// placement and passes the current graph zoom so presentations may scale or
/// deliberately remain screen-sized.
pub trait SocketIndicatorPresentation: Send + Sync + 'static {
    fn size(&self, zoom: f32) -> Vec2;
    fn draw(&self, painter: &Painter, rect: Rect, zoom: f32);
}
