use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Framed payload whose capture-worker encoding or decoding failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerFrame {
    /// Command sent to the capture worker.
    Request,
    /// Batch of updates returned by the capture worker.
    MessageBatch,
}

impl fmt::Display for CaptureWorkerFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Request => "request",
            Self::MessageBatch => "message batch",
        })
    }
}

/// Failure while encoding or decoding the capture-worker wire protocol.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerCodecError {
    /// A typed frame could not be serialized.
    #[error("could not encode capture-worker {frame}: {message}")]
    Encoding {
        /// Kind of frame being serialized.
        frame: CaptureWorkerFrame,
        /// Serializer diagnostic retained for presentation.
        message: String,
    },
    /// A serialized frame could not be decoded into its typed value.
    #[error("capture-worker {frame} is invalid: {message}")]
    Decoding {
        /// Kind of frame being decoded.
        frame: CaptureWorkerFrame,
        /// Deserializer diagnostic retained for presentation.
        message: String,
    },
    /// The frame does not begin with the expected protocol version header.
    #[error("capture-worker {frame} has an invalid header")]
    InvalidHeader {
        /// Kind of frame whose header was rejected.
        frame: CaptureWorkerFrame,
    },
    /// A frame uses an unknown discriminant.
    #[error("capture-worker {frame} has unknown kind {kind}")]
    UnknownKind {
        /// Kind of frame containing the discriminant.
        frame: CaptureWorkerFrame,
        /// Unrecognized wire discriminant.
        kind: u8,
    },
    /// A collection is too large for its fixed-width protocol count.
    #[error("capture-worker {field} count exceeds the wire format")]
    CountOverflow {
        /// Collection whose element count overflowed.
        field: String,
    },
    /// A length from the wire cannot be represented by this host.
    #[error("capture-worker field length exceeds this host")]
    LengthOverflow,
    /// Adding a field length overflowed the frame address space.
    #[error("capture-worker message length overflow")]
    AddressOverflow,
    /// A field extends past the available frame bytes.
    #[error("capture-worker message is truncated")]
    Truncated,
    /// A decoded frame contains bytes after its final field.
    #[error("capture-worker message has trailing bytes")]
    TrailingBytes,
}

/// Kind of outstanding request tracked by the bounded capture-worker client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerRequestKind {
    /// Capture preparation request.
    Preparation,
    /// Sampled-window query request.
    Query,
    /// Packed-block replay request.
    Replay,
}

impl fmt::Display for CaptureWorkerRequestKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Preparation => "preparation",
            Self::Query => "query",
            Self::Replay => "replay",
        })
    }
}

/// Kind of update returned through the capture-worker protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerMessageKind {
    /// Preparation progress update.
    Progress,
    /// Discovered source metadata.
    Metadata,
    /// Prepared capture session.
    Prepared,
    /// Sampled query window.
    Window,
    /// Packed replay data.
    Replay,
    /// Terminal worker failure.
    Failure,
    /// Cancellation confirmation.
    Cancellation,
}

impl fmt::Display for CaptureWorkerMessageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Progress => "progress",
            Self::Metadata => "metadata",
            Self::Prepared => "prepared",
            Self::Window => "window",
            Self::Replay => "replay",
            Self::Failure => "failure",
            Self::Cancellation => "cancellation",
        })
    }
}

/// Failure reported when a capture-worker transport becomes unusable.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerTransportFailure {
    /// The host worker API rejected or lost communication.
    #[error("{0}")]
    Host(String),
    /// A request or response violated the capture-worker wire format.
    #[error(transparent)]
    Codec(CaptureWorkerCodecError),
    /// The client was configured with no request capacity.
    #[error("capture-worker queue must accept at least one request")]
    InvalidCapacity,
    /// The bounded client has no capacity for another request.
    #[error("capture-worker queue is full ({limit} outstanding request limit)")]
    QueueFull {
        /// Configured outstanding-request limit.
        limit: u64,
    },
    /// No further correlation sequence can be allocated.
    #[error("capture-worker request sequence exhausted")]
    SequenceExhausted,
    /// A worker update does not correspond to a pending request.
    #[error("capture worker returned sequence {sequence} with no pending request")]
    UnexpectedSequence {
        /// Sequence carried by the unexpected update.
        sequence: u64,
    },
    /// A worker update is not valid for its pending request kind.
    #[error("capture worker returned {message} for a {request} request")]
    UnexpectedMessage {
        /// Kind of pending request.
        request: CaptureWorkerRequestKind,
        /// Kind of incompatible update.
        message: CaptureWorkerMessageKind,
    },
}

impl From<CaptureWorkerCodecError> for CaptureWorkerTransportFailure {
    fn from(error: CaptureWorkerCodecError) -> Self {
        Self::Codec(error)
    }
}

/// Failure from the bounded capture-worker client contract.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CaptureWorkerClientError {
    /// The client was configured with no request capacity.
    #[error("capture-worker queue must accept at least one request")]
    InvalidCapacity,
    /// The worker transport has already failed.
    #[error("capture worker is disconnected: {0}")]
    Disconnected(#[source] CaptureWorkerTransportFailure),
    /// The bounded client has no capacity for another request.
    #[error("capture-worker queue is full ({limit} outstanding request limit)")]
    QueueFull {
        /// Configured outstanding-request limit.
        limit: usize,
    },
    /// No further correlation sequence can be allocated.
    #[error("capture-worker request sequence exhausted")]
    SequenceExhausted,
    /// A worker update does not correspond to a pending request.
    #[error("capture worker returned sequence {sequence} with no pending request")]
    UnexpectedSequence {
        /// Sequence carried by the unexpected update.
        sequence: u64,
    },
    /// A worker update is not valid for its pending request kind.
    #[error("capture worker returned {message} for a {request} request")]
    UnexpectedMessage {
        /// Kind of pending request.
        request: CaptureWorkerRequestKind,
        /// Kind of incompatible update.
        message: CaptureWorkerMessageKind,
    },
}

impl From<CaptureWorkerClientError> for CaptureWorkerTransportFailure {
    fn from(error: CaptureWorkerClientError) -> Self {
        match error {
            CaptureWorkerClientError::InvalidCapacity => Self::InvalidCapacity,
            CaptureWorkerClientError::Disconnected(error) => error,
            CaptureWorkerClientError::QueueFull { limit } => Self::QueueFull {
                limit: u64::try_from(limit).unwrap_or(u64::MAX),
            },
            CaptureWorkerClientError::SequenceExhausted => Self::SequenceExhausted,
            CaptureWorkerClientError::UnexpectedSequence { sequence } => {
                Self::UnexpectedSequence { sequence }
            }
            CaptureWorkerClientError::UnexpectedMessage { request, message } => {
                Self::UnexpectedMessage { request, message }
            }
        }
    }
}

/// Classified terminal failure from one worker-hosted capture operation.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWorkerFailure {
    /// Capture preparation or index construction failed.
    #[error("capture preparation failed: {0}")]
    Preparation(String),
    /// A sampled-window query failed.
    #[error("capture query failed: {0}")]
    Query(String),
    /// Packed capture replay failed.
    #[error("capture replay failed: {0}")]
    Replay(String),
    /// The host worker transport stopped or rejected the request.
    #[error("capture worker transport failed: {0}")]
    Transport(#[source] CaptureWorkerTransportFailure),
}
