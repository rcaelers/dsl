use std::sync::Arc;

use super::lanes::ViewerLaneRenderer;

/// Plugin registration for a viewer renderer addressed by stable metadata.
pub struct ViewerLaneRendererRegistration {
    key: &'static str,
    factory: fn() -> Arc<dyn ViewerLaneRenderer>,
}

impl ViewerLaneRendererRegistration {
    /// Creates an inventory registration for a stable renderer key.
    ///
    /// # Parameters
    /// - `key`: Stable metadata key emitted by a payload or node feature.
    /// - `factory`: Constructor for the renderer selected by that key.
    pub const fn new(key: &'static str, factory: fn() -> Arc<dyn ViewerLaneRenderer>) -> Self {
        Self { key, factory }
    }

    /// Returns the stable renderer key.
    pub fn key(&self) -> &'static str {
        self.key
    }

    fn create(&self) -> Arc<dyn ViewerLaneRenderer> {
        (self.factory)()
    }
}

/// Resolves a stable renderer key contributed by a concrete node or payload plugin.
pub fn viewer_lane_renderer(key: &str) -> Option<Arc<dyn ViewerLaneRenderer>> {
    inventory::iter::<ViewerLaneRendererRegistration>
        .into_iter()
        .find(|registration| registration.key == key)
        .map(ViewerLaneRendererRegistration::create)
}

inventory::collect!(ViewerLaneRendererRegistration);
