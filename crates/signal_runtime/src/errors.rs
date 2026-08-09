//! Error types for the runtime system.

use std::any::TypeId;
use std::fmt;
use std::sync::Arc;

use crossbeam_channel::{RecvError, SendError};

use platform_runtime::WorkExecutorError;

use super::protocol::ProtocolKind;

/// Error type for port operations
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// The named node is absent from the pipeline.
    #[error("Node '{0}' not found")]
    NodeNotFound(String),

    /// The named port is absent from the named node.
    #[error("Port '{0}' not found on node '{1}'")]
    NotFound(String, String),

    /// The positional port index is outside the named node's port list.
    #[error("Port index {0} out of range for node '{1}'")]
    IndexOutOfRange(usize, String),
}

/// Error type for connection operations
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionError {
    /// The output and input ports carry incompatible payload types.
    #[error(
        "Type mismatch: {from_node}.{from_port} ({from_type:?}) -> {to_node}.{to_port} ({to_type:?})"
    )]
    TypeMismatch {
        from_node: String,
        from_port: String,
        from_type: TypeId,
        to_node: String,
        to_port: String,
        to_type: TypeId,
    },

    /// The named node is absent from the graph.
    #[error("Node '{0}' not found")]
    NodeNotFound(String),

    #[error("Port '{port}' not found on node '{node}'")]
    PortNotFound {
        /// Name of the node on which lookup failed.
        node: String,
        /// Name of the missing port.
        port: String,
    },

    /// One port's concrete payload type does not match the requested connection type.
    #[error("Port '{port}' on node '{node}' has type {actual}, not requested type {requested}")]
    PortTypeMismatch {
        /// Name of the node owning the incompatible port.
        node: String,
        /// Name or positional label of the incompatible port.
        port: String,
        /// Requested Rust payload type.
        requested: String,
        /// Port's declared Rust payload type.
        actual: String,
    },

    /// The graph already contains a connection to the requested input.
    #[error("Input port '{to_port}' on node '{to_node}' is already connected")]
    DuplicateConnection {
        /// Name of the consuming node.
        to_node: String,
        /// Name of the already-connected input.
        to_port: String,
    },

    /// A required graph input has no producer.
    #[error("Input port '{port}' on node '{node}' is not connected")]
    UnconnectedInput {
        /// Name of the node owning the input.
        node: String,
        /// Name or positional label of the unconnected input.
        port: String,
    },
}

/// Error produced while constructing or supervising a processing pipeline.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PipelineError {
    /// A graph-local node name is already in use.
    #[error("Node '{node}' already exists")]
    NodeAlreadyExists {
        /// Duplicate graph-local node name.
        node: String,
    },
    /// A requested node is absent from the pipeline.
    #[error("Node '{node}' not found")]
    NodeNotFound {
        /// Missing graph-local node name or identifier.
        node: String,
    },
    /// A lifecycle operation requires a running node.
    #[error("Node '{node}' is not running")]
    NodeNotRunning {
        /// Missing running node name.
        node: String,
    },
    /// A deferred-start operation names no registered node.
    #[error("Node '{node}' is not registered")]
    NodeNotRegistered {
        /// Missing registered node name.
        node: String,
    },
    /// A node cannot be started more than once.
    #[error("Node '{node}' is already started")]
    NodeAlreadyStarted {
        /// Already-started node name.
        node: String,
    },
    /// Supplied input wiring does not match the node schema.
    #[error("Node '{node}' has {provided} input specifications for {expected} ports")]
    InputCountMismatch {
        /// Node whose wiring is invalid.
        node: String,
        /// Number of supplied input specifications.
        provided: usize,
        /// Number of declared input ports.
        expected: usize,
    },
    /// A port payload was not registered for dynamic channel construction.
    #[error("Type {type_id:?} of port '{port}' is not registered")]
    PortTypeNotRegistered {
        /// Port declaring the unregistered payload type.
        port: String,
        /// Unregistered Rust payload identity.
        type_id: TypeId,
    },
    /// A payload was not registered for dynamic channel construction.
    #[error(
        "Type {type_id:?} is not registered; call register_type::<T>() before building the pipeline"
    )]
    TypeNotRegistered {
        /// Unregistered Rust payload identity.
        type_id: TypeId,
    },
    /// A node input names a producer that is not running.
    #[error("Producer '{producer}' is not running")]
    ProducerNotRunning {
        /// Missing producer name.
        producer: String,
    },
    /// A node input names no output on its producer.
    #[error("Producer '{producer}' has no port '{port}'")]
    ProducerPortNotFound {
        /// Producer node name.
        producer: String,
        /// Missing output port name.
        port: String,
    },
    /// Connected ports have no compatible payload representation.
    #[error("Type mismatch: {from_node}.{from_port} -> {to_node}.{to_port}")]
    PayloadTypeMismatch {
        /// Producing node name.
        from_node: String,
        /// Producing output port name.
        from_port: String,
        /// Consuming node name.
        to_node: String,
        /// Consuming input port name.
        to_port: String,
    },
    /// A node returned a protocol choice list with the wrong length.
    #[error("Node '{node}' returned {actual} protocol choices for {expected} inputs")]
    ProtocolChoiceCount {
        /// Node whose protocol negotiation contract was violated.
        node: String,
        /// Number of returned choices.
        actual: usize,
        /// Number of declared inputs.
        expected: usize,
    },
    /// Producer and consumer have no mutually supported input protocol.
    #[error("No common protocol for node '{node}' input {input}")]
    NoCommonProtocol {
        /// Consuming node name.
        node: String,
        /// Positional input index.
        input: usize,
    },
    /// A node selected a protocol outside the offered and declared sets.
    #[error("Node '{node}' selected unsupported protocol {protocol:?} for input {input}")]
    UnsupportedProtocol {
        /// Consuming node name.
        node: String,
        /// Positional input index.
        input: usize,
        /// Invalid selected protocol.
        protocol: ProtocolKind,
    },
    /// A selected capability is absent from the producer's advertised handles.
    #[error("Producer '{producer}.{port}' has no {protocol:?} capability")]
    CapabilityUnavailable {
        /// Producer node name.
        producer: String,
        /// Producer output port name.
        port: String,
        /// Selected capability protocol.
        protocol: ProtocolKind,
    },
    /// A node does not support scheduled hot configuration.
    #[error("Node '{node}' does not expose a scheduled configuration handle")]
    ConfigurationUnavailable {
        /// Node requested for scheduled configuration.
        node: String,
    },
    /// A node rejected a scheduled hot configuration.
    #[error("Node '{node}' rejected scheduled hot configuration")]
    ConfigurationRejected {
        /// Node that rejected configuration.
        node: String,
    },
    /// A running node no longer accepts immediate configuration messages.
    #[error("Node '{node}' no longer accepts configuration")]
    ConfigurationClosed {
        /// Node whose configuration channel is closed.
        node: String,
    },
    /// Dynamic output-channel construction received no destination endpoints.
    #[error("Output channel has no destinations")]
    OutputChannelUnavailable,
    /// Dynamic output-channel construction received an incompatible payload endpoint.
    #[error("Output channel endpoint has the wrong payload type")]
    OutputChannelTypeMismatch,
    /// The host rejected watchdog supervision work.
    #[error("Could not start pipeline watchdog: {source}")]
    WatchdogStart {
        /// Host work-submission failure.
        #[source]
        source: WorkExecutorError,
    },
    /// The host rejected one node's runtime task.
    #[error("Could not start node '{node}': {source}")]
    NodeTaskStart {
        /// Node whose task could not be submitted.
        node: String,
        /// Host work-submission failure.
        #[source]
        source: WorkExecutorError,
    },
}

/// Error type for work function operations
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkError {
    /// Receiving from an input channel failed because all senders disconnected.
    #[error("Failed to receive from input channel: {0}")]
    RecvError(#[from] RecvError),

    /// Sending to an output channel failed because all receivers disconnected.
    #[error("Failed to send to output channel: {0}")]
    SendError(String),

    /// A processing node rejected or could not process its input.
    #[error("Node-specific error: {0}")]
    NodeError(String),

    /// A processing node retained a typed owner-specific failure.
    #[error("Node-specific error: {0}")]
    NodeSource(#[source] NodeWorkError),

    /// The runtime requested that the worker stop processing.
    #[error("Shutdown signal received")]
    Shutdown,
}

/// Cloneable owner-specific failure retained by the generic process-work boundary.
#[derive(Clone)]
pub struct NodeWorkError(Arc<dyn std::error::Error + Send + Sync>);

impl NodeWorkError {
    pub(crate) fn new(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Arc::new(source))
    }
}

impl fmt::Debug for NodeWorkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NodeWorkError")
            .field(&self.0.to_string())
            .finish()
    }
}

impl fmt::Display for NodeWorkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for NodeWorkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl PartialEq for NodeWorkError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NodeWorkError {}

impl WorkError {
    /// Retains a typed processing-node failure through runtime supervision.
    pub fn node_source(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::NodeSource(NodeWorkError::new(source))
    }
}

impl<T> From<SendError<T>> for WorkError {
    fn from(e: SendError<T>) -> Self {
        WorkError::SendError(format!("{}", e))
    }
}

/// Result alias for worker operations.
pub type WorkResult<T = ()> = std::result::Result<T, WorkError>;

#[cfg(test)]
mod node_work_error_tests {
    use std::error::Error;

    use super::WorkError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled node failure")]
    struct ControlledNodeFailure;

    #[test]
    fn typed_node_causes_survive_cloneable_work_errors() {
        let error = WorkError::node_source(ControlledNodeFailure);
        let cloned = error.clone();

        assert_eq!(error, cloned);
        assert_eq!(
            error
                .source()
                .and_then(Error::source)
                .map(ToString::to_string)
                .as_deref(),
            Some("controlled node failure")
        );
    }
}
