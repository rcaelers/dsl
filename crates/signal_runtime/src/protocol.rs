//! Connection protocol negotiation vocabulary.
//!
//! A connection between an output port and an input port can be carried by
//! more than one wire protocol. [`ProtocolKind`] names the protocols a port
//! can speak; [`super::pipeline::Pipeline::build`] negotiates the best
//! mutually-supported protocol per connection (producer preference order
//! wins ties) before allocating anything for it. Adding a new protocol is
//! adding a variant here plus producer/consumer support for it — the
//! negotiation logic itself never changes.

use std::any::{Any, TypeId};
use std::sync::Arc;

/// A transport protocol a port can speak, independent of its payload type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    /// A bounded channel carrying `ChannelMessage<T>`.
    Stream,
    /// A type-erased capability shared once while the graph is materialized.
    Capability(TypeId),
}

impl ProtocolKind {
    /// Identifies a typed capability transport without teaching the runtime
    /// what the capability means.
    pub fn capability<T: ?Sized + 'static>() -> Self {
        Self::Capability(TypeId::of::<Arc<T>>())
    }
}

/// One typed, type-erased capability offered by a producer.
#[derive(Clone)]
pub struct ProtocolCapability {
    protocol: ProtocolKind,
    value: Arc<dyn Any + Send + Sync>,
}

impl ProtocolCapability {
    /// Erases a shared capability while retaining its protocol identity.
    pub fn new<T: ?Sized + Send + Sync + 'static>(value: Arc<T>) -> Self {
        Self {
            protocol: ProtocolKind::capability::<T>(),
            value: Arc::new(value),
        }
    }

    /// Returns the protocol implemented by this value.
    pub fn protocol(&self) -> ProtocolKind {
        self.protocol
    }

    /// Recovers the typed shared capability when the requested type matches.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.value.downcast_ref::<Arc<T>>().cloned()
    }
}
