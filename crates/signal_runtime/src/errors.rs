//! Error types for the runtime system.

use std::any::TypeId;

use crossbeam_channel::{RecvError, SendError};

/// Error type for port operations
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    /// The named port is absent from the named node.
    #[error("Port '{0}' not found on node '{1}'")]
    NotFound(String, String),

    /// The positional port index is outside the named node's port list.
    #[error("Port index {0} out of range for node '{1}'")]
    IndexOutOfRange(usize, String),
}

/// Error type for connection operations
#[derive(Debug, thiserror::Error)]
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

    /// The graph already contains the requested connection.
    #[error("{0}")]
    DuplicateConnection(String),
}

/// Error type for work function operations
#[derive(Debug, thiserror::Error)]
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

    /// The runtime requested that the worker stop processing.
    #[error("Shutdown signal received")]
    Shutdown,
}

impl<T> From<SendError<T>> for WorkError {
    fn from(e: SendError<T>) -> Self {
        WorkError::SendError(format!("{}", e))
    }
}

/// Result alias for worker operations.
pub type WorkResult<T = ()> = std::result::Result<T, WorkError>;
