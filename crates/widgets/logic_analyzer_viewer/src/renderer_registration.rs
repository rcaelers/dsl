use std::sync::Arc;

use super::lanes::ViewerLaneRenderer;

/// Plugin registration for a viewer renderer addressed by stable metadata.
pub struct ViewerLaneRendererRegistration {
    key: &'static str,
    factory: fn() -> Arc<dyn ViewerLaneRenderer>,
}

impl ViewerLaneRendererRegistration {
    pub const fn new(key: &'static str, factory: fn() -> Arc<dyn ViewerLaneRenderer>) -> Self {
        Self { key, factory }
    }

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
