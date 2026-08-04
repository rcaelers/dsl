use std::any::TypeId;
use std::fmt;

use signal_processing::{NumberSample, Sample, SampleBlock, TextSample, Trigger, Word};

/// Rust value type that may flow through a graph port.
///
/// Implementations provide stable, presentation-independent type metadata used
/// by graph negotiation and runtime channel sizing.
pub trait PortValue: Send + Sync + Clone + 'static {
    /// Returns the stable runtime name used to identify this payload kind.
    fn kind_name() -> &'static str;

    /// Returns the preferred bounded-channel capacity for this payload.
    ///
    /// Source outputs may need a larger buffer than transformed intermediate
    /// values to absorb capture bursts.
    ///
    /// # Parameters
    /// - `producer_is_source`: Whether the producing node originates capture data.
    fn buffer_size(producer_is_source: bool) -> usize {
        let _ = producer_is_source;
        100
    }
}

/// Runtime metadata for a graph-port payload type.
///
/// A `PortKind` retains type identity without exposing the concrete value type,
/// allowing the generic graph compiler to negotiate compatible connections.
#[derive(Clone, Copy)]
pub struct PortKind {
    type_id: TypeId,
    name: &'static str,
    buffer_size_fn: fn(bool) -> usize,
    register_type_fn: fn(),
}

impl PartialEq for PortKind {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for PortKind {}

impl std::hash::Hash for PortKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl PortKind {
    /// Creates metadata for a payload type implementing [`PortValue`].
    pub fn of<T: PortValue>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            name: T::kind_name(),
            buffer_size_fn: T::buffer_size,
            register_type_fn: signal_processing::register_type::<T>,
        }
    }

    /// Creates an open payload kind whose Rust value type is owned by another
    /// workspace layer or compile-time plugin.
    pub fn of_named<T: Clone + Send + Sync + 'static>(name: &'static str) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            name,
            buffer_size_fn: default_buffer_size,
            register_type_fn: signal_processing::register_type::<T>,
        }
    }

    /// Returns the concrete Rust type identity used for compatibility checks.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Returns the stable payload-kind name shown in graph diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the channel capacity selected for a producer role.
    ///
    /// # Parameters
    /// - `producer_is_source`: Whether the producing node originates capture data.
    pub fn buffer_size(&self, producer_is_source: bool) -> usize {
        (self.buffer_size_fn)(producer_is_source)
    }

    /// Registers the payload type with the runtime serialization registry.
    pub fn register_runtime_type(&self) {
        (self.register_type_fn)();
    }
}

fn default_buffer_size(_producer_is_source: bool) -> usize {
    100
}

impl fmt::Debug for PortKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name)
    }
}

impl PortValue for Sample {
    fn kind_name() -> &'static str {
        "SampleEdge"
    }

    fn buffer_size(producer_is_source: bool) -> usize {
        if producer_is_source {
            10_000_000
        } else {
            1_000
        }
    }
}

impl PortValue for SampleBlock {
    fn kind_name() -> &'static str {
        "Block"
    }

    fn buffer_size(_producer_is_source: bool) -> usize {
        2
    }
}

impl PortValue for Word {
    fn kind_name() -> &'static str {
        "Word"
    }

    fn buffer_size(_producer_is_source: bool) -> usize {
        8
    }
}

impl PortValue for Trigger {
    fn kind_name() -> &'static str {
        "Trigger"
    }
}

impl PortValue for NumberSample {
    fn kind_name() -> &'static str {
        "Number"
    }
}

impl PortValue for TextSample {
    fn kind_name() -> &'static str {
        "Text"
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[test]
    fn kinds_use_open_type_identity() {
        assert_eq!(PortKind::of::<Sample>(), PortKind::of::<Sample>());
        assert_ne!(PortKind::of::<Sample>(), PortKind::of::<SampleBlock>());
        assert_eq!(format!("{:?}", PortKind::of::<Sample>()), "SampleEdge");
    }

    #[test]
    fn named_kind_supports_a_payload_owned_by_a_lower_layer() {
        #[derive(Clone)]
        struct ExternalPayload;

        let kind = PortKind::of_named::<ExternalPayload>("External");
        assert_eq!(kind.type_id(), TypeId::of::<ExternalPayload>());
        assert_eq!(kind.name(), "External");
        assert_eq!(kind.buffer_size(false), 100);
    }
}
