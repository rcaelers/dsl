use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use platform_artifacts::RepositoryError;

/// Framed payload whose graph-worker encoding or decoding failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphWorkerFrame {
    /// Command sent to the graph worker.
    Request,
    /// Batch of updates returned by the graph worker.
    MessageBatch,
    /// Artifact replication event embedded in a message batch.
    ArtifactEvent,
}

impl fmt::Display for GraphWorkerFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Request => "request",
            Self::MessageBatch => "message batch",
            Self::ArtifactEvent => "artifact event",
        })
    }
}

/// Failure while encoding or decoding the graph-worker wire protocol.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphWorkerCodecError {
    /// A typed value could not be serialized for one protocol field.
    #[error("could not encode graph-worker {context}: {message}")]
    Encoding {
        /// Protocol field or payload being serialized.
        context: String,
        /// Serializer diagnostic retained for presentation.
        message: String,
    },
    /// A serialized protocol field could not be decoded into its typed value.
    #[error("graph-worker {context} is invalid: {message}")]
    Decoding {
        /// Protocol field or payload being decoded.
        context: String,
        /// Deserializer diagnostic retained for presentation.
        message: String,
    },
    /// A graph document could not provide the structure required by worker transport.
    #[error("graph-worker request has invalid graph structure: {0}")]
    InvalidGraphStructure(String),
    /// The frame does not begin with the expected protocol version header.
    #[error("graph-worker {frame} has an invalid header")]
    InvalidHeader {
        /// Kind of frame whose header was rejected.
        frame: GraphWorkerFrame,
    },
    /// A frame or embedded event uses an unknown discriminant.
    #[error("graph-worker {frame} has unknown kind {kind}")]
    UnknownKind {
        /// Kind of frame containing the discriminant.
        frame: GraphWorkerFrame,
        /// Unrecognized wire discriminant.
        kind: u8,
    },
    /// A collection is too large for its fixed-width protocol count.
    #[error("graph-worker {field} count exceeds the wire format")]
    CountOverflow {
        /// Collection whose element count overflowed.
        field: String,
    },
    /// A length from the wire cannot be represented by this host.
    #[error("graph-worker field length exceeds this host")]
    LengthOverflow,
    /// Adding a field length overflowed the frame address space.
    #[error("graph-worker message length overflow")]
    AddressOverflow,
    /// A field extends past the available frame bytes.
    #[error("graph-worker message is truncated")]
    Truncated,
    /// A decoded frame contains bytes after its final field.
    #[error("graph-worker message has trailing bytes")]
    TrailingBytes,
    /// A textual protocol field is not valid UTF-8.
    #[error("graph-worker {field} is not UTF-8")]
    InvalidUtf8 {
        /// Textual field that failed validation.
        field: String,
    },
    /// A boolean flag contains a value other than zero or one.
    #[error("graph-worker {field} flag has invalid value {value}")]
    InvalidFlag {
        /// Flag whose value was rejected.
        field: String,
        /// Invalid encoded value.
        value: u8,
    },
}

/// Failure reported when a graph-worker transport becomes unusable.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphWorkerTransportFailure {
    /// The host worker API rejected or lost communication.
    #[error("{0}")]
    Host(String),
    /// A request or response violated the graph-worker wire format.
    #[error(transparent)]
    Codec(GraphWorkerCodecError),
    /// The client was configured with no request capacity.
    #[error("graph-worker queue must accept at least one request")]
    InvalidCapacity,
    /// The bounded client has no capacity for another run.
    #[error("graph-worker queue is full ({limit} outstanding request limit)")]
    QueueFull {
        /// Configured outstanding-run limit.
        limit: u64,
    },
    /// No further correlation sequence can be allocated.
    #[error("graph-worker request sequence exhausted")]
    SequenceExhausted,
    /// A worker update does not correspond to a pending run.
    #[error("graph worker returned sequence {sequence} with no pending request")]
    UnexpectedSequence {
        /// Sequence carried by the unexpected update.
        sequence: u64,
    },
    /// Applying a replicated worker artifact failed.
    #[error("graph-worker artifact replication failed: {0}")]
    Artifact(#[source] RepositoryError),
}

impl From<GraphWorkerCodecError> for GraphWorkerTransportFailure {
    fn from(error: GraphWorkerCodecError) -> Self {
        Self::Codec(error)
    }
}

/// Failure from the bounded graph-worker client contract.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GraphWorkerClientError {
    /// The client was configured with no request capacity.
    #[error("graph-worker queue must accept at least one request")]
    InvalidCapacity,
    /// The worker transport has already failed.
    #[error("graph worker is disconnected: {0}")]
    Disconnected(#[source] GraphWorkerTransportFailure),
    /// The bounded client has no capacity for another run.
    #[error("graph-worker queue is full ({limit} outstanding request limit)")]
    QueueFull {
        /// Configured outstanding-run limit.
        limit: usize,
    },
    /// No further correlation sequence can be allocated.
    #[error("graph-worker request sequence exhausted")]
    SequenceExhausted,
    /// A worker update does not correspond to a pending run.
    #[error("graph worker returned sequence {sequence} with no pending request")]
    UnexpectedSequence {
        /// Sequence carried by the unexpected update.
        sequence: u64,
    },
    /// Applying a replicated worker artifact failed.
    #[error("graph-worker artifact replication failed: {0}")]
    ArtifactReplication(#[source] RepositoryError),
}

impl From<GraphWorkerClientError> for GraphWorkerTransportFailure {
    fn from(error: GraphWorkerClientError) -> Self {
        match error {
            GraphWorkerClientError::InvalidCapacity => Self::InvalidCapacity,
            GraphWorkerClientError::Disconnected(error) => error,
            GraphWorkerClientError::QueueFull { limit } => Self::QueueFull {
                limit: u64::try_from(limit).unwrap_or(u64::MAX),
            },
            GraphWorkerClientError::SequenceExhausted => Self::SequenceExhausted,
            GraphWorkerClientError::UnexpectedSequence { sequence } => {
                Self::UnexpectedSequence { sequence }
            }
            GraphWorkerClientError::ArtifactReplication(error) => Self::Artifact(error),
        }
    }
}
