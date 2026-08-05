//! Port-based API for ergonomic node connections.
//!
//! Provides a Pipeline builder that manages channels automatically,
//! plus InputPort and OutputPort type-erased wrappers for channel endpoints.

use std::any::TypeId;

use super::protocol::{ProtocolCapability, ProtocolKind};
use super::receiver::Receiver;
use super::sender::Sender;
pub use super::type_registry::register_type;
use super::watchdog::{Watchdog, WatchdogHandle};

/// Direction of a port relative to its processing node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// Port receives values from an upstream node.
    Input,
    /// Port sends values to downstream nodes.
    Output,
}

/// How much streamed input a node needs before one `work()` call is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamReadiness {
    /// One queued item (or end-of-stream) is sufficient.
    #[default]
    Item,
    /// The finite producer must finish before the consumer starts. This is
    /// for block-oriented nodes that perform unbounded lookahead on an
    /// auxiliary stream during one `work()` call.
    Complete,
}

/// Whether a streamed payload represents retained state or an occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamSemantics {
    /// Values are independent occurrences and are not replayed to late subscribers.
    #[default]
    Event,
    /// The latest value remains current and primes subscribers attached during a run.
    State,
}

/// One concrete payload representation supported by a logical port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortPayload {
    /// Concrete Rust payload type identity.
    pub type_id: TypeId,
    /// Replay behavior for subscribers attached to a running output.
    pub stream_semantics: StreamSemantics,
    /// Optional upper bound applied to the pipeline's default queue capacity.
    pub default_buffer_capacity: Option<usize>,
}

impl PortPayload {
    /// Describes and registers one event payload type.
    pub fn new<T: Clone + Send + Sync + 'static>() -> Self {
        register_type::<T>();
        Self {
            type_id: TypeId::of::<T>(),
            stream_semantics: StreamSemantics::Event,
            default_buffer_capacity: None,
        }
    }

    /// Marks this representation as retained state.
    pub fn state(mut self) -> Self {
        self.stream_semantics = StreamSemantics::State;
        self
    }

    /// Bounds the default queue capacity for this representation.
    pub fn with_default_buffer_capacity(mut self, capacity: usize) -> Self {
        self.default_buffer_capacity = Some(capacity.max(1));
        self
    }
}

/// Schema describing a runtime port's identity, capabilities, and scheduling needs.
#[derive(Debug, Clone)]
pub struct PortSchema {
    /// Stable runtime port name within its node.
    pub name: String,
    /// Concrete Rust payload type identity.
    pub type_id: TypeId,
    /// Definition index on the owning node.
    pub index: usize,
    /// Whether the port receives or produces values.
    pub direction: PortDirection,
    /// Protocols this port can speak, most preferred first. Default:
    /// `[Stream]`, the guaranteed fallback every port supports — override
    /// via [`Self::with_protocols`] for a port that can also expose a
    /// type-erased capability through [`super::node::ProcessNode::protocol_capability`].
    pub protocols: Vec<ProtocolKind>,
    /// Concrete representations accepted or offered by this logical port, in
    /// preference order. A fixed-type port contains one entry.
    pub payloads: Vec<PortPayload>,
    /// Cooperative scheduling requirement for this input. Threaded runners
    /// can block independently and therefore do not need to consult it.
    pub stream_readiness: StreamReadiness,
}

impl PortSchema {
    /// Creates a port schema with one concrete payload type.
    ///
    /// # Parameters
    /// - `name`: Stable runtime name within the owning node.
    /// - `index`: Definition index on the owning node.
    /// - `direction`: Whether the port receives or produces values.
    pub fn new<T: Clone + Send + Sync + 'static>(
        name: impl Into<String>,
        index: usize,
        direction: PortDirection,
    ) -> Self {
        Self {
            name: name.into(),
            type_id: TypeId::of::<T>(),
            index,
            direction,
            protocols: vec![ProtocolKind::Stream],
            payloads: vec![PortPayload::new::<T>()],
            stream_readiness: StreamReadiness::Item,
        }
    }

    /// Creates a fixed-type port whose latest value remains current.
    pub fn state<T: Clone + Send + Sync + 'static>(
        name: impl Into<String>,
        index: usize,
        direction: PortDirection,
    ) -> Self {
        Self::new::<T>(name, index, direction).with_state_semantics()
    }

    /// Declares which protocols this port can speak, most preferred first.
    pub fn with_protocols(mut self, protocols: Vec<ProtocolKind>) -> Self {
        self.protocols = protocols;
        self
    }

    /// Replaces the concrete representations this port accepts or offers.
    pub fn with_payloads(mut self, payloads: Vec<PortPayload>) -> Self {
        assert!(
            !payloads.is_empty(),
            "a port must support at least one payload"
        );
        self.payloads = payloads;
        self
    }

    /// Marks every currently declared representation as retained state.
    pub fn with_state_semantics(mut self) -> Self {
        for payload in &mut self.payloads {
            payload.stream_semantics = StreamSemantics::State;
        }
        self
    }

    /// Bounds every currently declared representation's default queue capacity.
    pub fn with_default_buffer_capacity(mut self, capacity: usize) -> Self {
        for payload in &mut self.payloads {
            payload.default_buffer_capacity = Some(capacity.max(1));
        }
        self
    }

    /// Requires a finite streamed producer to close before this input is
    /// considered ready by a cooperative runner.
    pub fn with_complete_stream(mut self) -> Self {
        self.stream_readiness = StreamReadiness::Complete;
        self
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Type-erased port wrappers
// ────────────────────────────────────────────────────────────────────────────

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Receiver as CrossbeamReceiver;

use super::sender::ChannelMessage;

/// Type-erased input port wrapping a `Receiver<T>`.
pub struct InputPort {
    channel: Box<dyn std::any::Any + Send>,
    watchdog_handle: Option<WatchdogHandle>,
    eos_received: AtomicBool,
    /// Capability selected as this connection's transport.
    protocol_capability: Option<ProtocolCapability>,
    /// Capability offered by the producer even when this connection streams.
    available_protocol_capabilities: Vec<ProtocolCapability>,
}

impl InputPort {
    /// Create from type-erased box (for internal use by Pipeline).
    /// Watchdog must be attached via with_watchdog() before use.
    pub(crate) fn from_type_erased(channel: Box<dyn std::any::Any + Send>) -> Self {
        Self {
            channel,
            watchdog_handle: None,
            eos_received: AtomicBool::new(false),
            protocol_capability: None,
            available_protocol_capabilities: Vec::new(),
        }
    }

    /// Creates an intentionally disconnected optional input. Typed `get()` calls will
    /// return `None`, matching an optional/unconnected port.
    pub fn disconnected() -> Self {
        Self::from_type_erased(Box::new(()))
    }

    /// Creates a typed input port with watchdog instrumentation.
    ///
    /// # Parameters
    /// - `receiver`: Stream receiver carrying typed channel messages.
    /// - `watchdog`: Runtime watchdog that records blocked receives.
    /// - `node_name`: Runtime name of the owning node.
    /// - `port_name`: Runtime name of this input port.
    pub fn new_with_watchdog<T: Send + 'static>(
        receiver: CrossbeamReceiver<ChannelMessage<T>>,
        watchdog: &Watchdog,
        node_name: &str,
        port_name: &str,
    ) -> Self {
        Self {
            channel: Box::new(receiver),
            watchdog_handle: Some(watchdog.register_port(node_name, "recv", port_name)),
            eos_received: AtomicBool::new(false),
            protocol_capability: None,
            available_protocol_capabilities: Vec::new(),
        }
    }

    /// Attaches watchdog context when assembling a port outside a pipeline.
    pub fn with_watchdog(
        mut self,
        watchdog: Watchdog,
        node_name: String,
        port_name: String,
    ) -> Self {
        self.watchdog_handle = Some(watchdog.register_port(&node_name, "recv", &port_name));
        self
    }

    /// Attaches the capability selected as this connection's transport.
    pub fn with_protocol_capability(mut self, capability: Option<ProtocolCapability>) -> Self {
        if let Some(capability) = &capability {
            self.available_protocol_capabilities
                .push(capability.clone());
        }
        self.protocol_capability = capability;
        self
    }

    /// Retains a producer capability without changing the selected transport.
    pub fn with_available_protocol_capabilities(
        mut self,
        capabilities: Vec<ProtocolCapability>,
    ) -> Self {
        self.available_protocol_capabilities = capabilities;
        self
    }

    /// Recovers the typed capability selected for this connection.
    pub fn protocol_capability<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.protocol_capability.as_ref()?.get::<T>()
    }

    /// Recovers an offered capability independently of the selected transport.
    pub fn available_protocol_capability<T: ?Sized + Send + Sync + 'static>(
        &self,
    ) -> Option<Arc<T>> {
        let protocol = ProtocolKind::capability::<T>();
        self.available_protocol_capabilities
            .iter()
            .find(|capability| capability.protocol() == protocol)?
            .get::<T>()
    }

    /// Whether this input has either a streamed channel or a negotiated query.
    pub fn is_connected(&self) -> bool {
        !self.channel.is::<()>() || self.protocol_capability.is_some()
    }

    /// Get a Receiver with automatic watchdog monitoring.
    ///
    /// Returns `None` if the port doesn't contain a `Receiver<T>`.
    ///
    /// # Panics
    /// Panics if watchdog has not been attached to this port.
    pub fn get<'a, T: Send + 'static>(
        &'a self,
        buffer: &'a mut std::collections::VecDeque<T>,
    ) -> Option<Receiver<'a, T>> {
        let receiver = self
            .channel
            .downcast_ref::<CrossbeamReceiver<ChannelMessage<T>>>()?;
        let watchdog = self.watchdog_handle.as_ref().expect(
            "InputPort.get() called before watchdog attached - this is a bug in the pipeline",
        );
        Some(Receiver::with_watchdog(
            receiver,
            buffer,
            watchdog.clone(),
            &self.eos_received,
        ))
    }
}

/// Type-erased output port wrapping one or more `Sender<T>`s.
///
/// Usually exactly one concrete `T`. A port offering several [`PortPayload`]
/// representations can hold multiple concretely typed senders at once. A short
/// vector is used because lookup occurs only during node startup, never per value.
pub struct OutputPort {
    channels: Vec<(TypeId, Box<dyn std::any::Any + Send>)>,
    watchdog_handle: Option<WatchdogHandle>,
}

impl OutputPort {
    /// Create from a type-erased box holding a `Sender<T>` (for internal
    /// use by Pipeline). `payload_type` must be `TypeId::of::<T>()` — the
    /// *payload* type `get::<T>()`/`split_senders::<T>()` will look up by,
    /// **not** `channel`'s own `Any::type_id()` (which would be
    /// `TypeId::of::<Sender<T>>()`, the wrapper, always different from
    /// `T`). Watchdog must be attached via with_watchdog() before use.
    pub(crate) fn from_type_erased(
        payload_type: TypeId,
        channel: Box<dyn std::any::Any + Send>,
    ) -> Self {
        Self {
            channels: vec![(payload_type, channel)],
            watchdog_handle: None,
        }
    }

    /// Adds a second concretely-typed sender to this port (internal use by
    /// `Pipeline::build`/`PipelineManager` when a producer negotiated more
    /// than one `SampleKind` for the same logical port). Same `payload_type`
    /// caveat as [`Self::from_type_erased`].
    pub(crate) fn extend_type_erased(
        mut self,
        payload_type: TypeId,
        channel: Box<dyn std::any::Any + Send>,
    ) -> Self {
        self.channels.push((payload_type, channel));
        self
    }

    /// Creates a typed output port with watchdog instrumentation.
    ///
    /// # Parameters
    /// - `sender`: Broadcast sender carrying typed channel messages.
    /// - `watchdog`: Runtime watchdog that records blocked sends.
    /// - `node_name`: Runtime name of the owning node.
    /// - `port_name`: Runtime name of this output port.
    pub fn new_with_watchdog<T: Send + Clone + 'static>(
        sender: Sender<T>,
        watchdog: &Watchdog,
        node_name: &str,
        port_name: &str,
    ) -> Self {
        Self {
            channels: vec![(TypeId::of::<T>(), Box::new(sender))],
            watchdog_handle: Some(watchdog.register_port(node_name, "send", port_name)),
        }
    }

    /// Set watchdog context for this port
    pub(crate) fn with_watchdog(
        mut self,
        watchdog: Watchdog,
        node_name: String,
        port_name: String,
    ) -> Self {
        self.watchdog_handle = Some(watchdog.register_port(&node_name, "send", &port_name));
        self
    }

    fn find<T: 'static>(&self) -> Option<&Sender<T>> {
        self.channels
            .iter()
            .find(|(type_id, _)| *type_id == TypeId::of::<T>())
            .and_then(|(_, boxed)| boxed.downcast_ref::<Sender<T>>())
    }

    /// Get a Sender with automatic watchdog monitoring.
    /// Returns an owned sender (cheaply cloned from internal storage).
    ///
    /// Returns `None` if the port doesn't contain a `Sender<T>`.
    ///
    /// # Panics
    /// Panics if watchdog has not been attached to this port.
    pub fn get<T: Send + Clone + 'static>(&self) -> Option<Sender<T>> {
        let sender = self.find::<T>()?;
        let watchdog = self.watchdog_handle.as_ref().expect(
            "OutputPort.get() called before watchdog attached - this is a bug in the pipeline",
        );
        Some(sender.with_watchdog(watchdog.clone()))
    }

    /// Clone the underlying Sender for this port.
    /// Used by nodes that spawn their own worker threads.
    pub fn clone_sender<T: Send + Clone + 'static>(&self) -> Option<Sender<T>> {
        self.find::<T>().cloned()
    }

    /// Split the underlying broadcast Sender into individual senders (one per destination).
    ///
    /// For nodes that need per-destination parallelism,
    /// this allows spawning one thread per destination. Each returned Sender
    /// sends to exactly one destination.
    ///
    /// Returns `None` if the port doesn't contain a `Sender<T>`, or if the sender
    /// has no destinations. A port carrying more than one `SampleKind`
    /// (see the struct doc) is queried independently per `T` — each call
    /// only sees the destinations that negotiated that particular type.
    pub fn split_senders<T: Send + Clone + 'static>(&self) -> Option<Vec<Sender<T>>> {
        let sender = self.find::<T>()?;
        let splits = sender.split_senders();
        if splits.is_empty() {
            None
        } else {
            Some(splits)
        }
    }
}

impl fmt::Debug for OutputPort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "OutputPort")
    }
}
