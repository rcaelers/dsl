use std::sync::Arc;

use node_graph::api::NodeTypeRegistry;

/// Instance-owned replacement for one inventory node's editor registration.
///
/// Application composition uses an override when a concrete definition needs a host-supplied
/// metadata service. The generic registry identifies the definition only by its stable feature ID
/// and remains unaware of the concrete node or host capability.
#[derive(Clone)]
pub struct GraphNodeEditorOverride {
    stable_id: String,
    apply: Arc<dyn Fn(&mut NodeTypeRegistry) + Send + Sync>,
}

impl GraphNodeEditorOverride {
    /// Creates an editor registration override for one stable graph-node feature.
    pub fn new(
        stable_id: impl Into<String>,
        apply: impl Fn(&mut NodeTypeRegistry) + Send + Sync + 'static,
    ) -> Self {
        Self {
            stable_id: stable_id.into(),
            apply: Arc::new(apply),
        }
    }

    /// Returns the stable graph-node feature replaced by this override.
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    /// Registers the instance-bound definition with an editor registry.
    pub fn apply(&self, registry: &mut NodeTypeRegistry) {
        (self.apply)(registry);
    }
}
