use crate::capture::CaptureIndexQueryError;

/// Errors returned by capture and indexed-signal infrastructure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An underlying filesystem or device I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Input could not be parsed into the requested capture representation.
    #[error("Parse error: {0}")]
    ParseError(String),
    /// A requested probe index does not exist in the capture.
    #[error("Invalid probe number: {0}")]
    InvalidProbe(usize),
    /// A requested capture block index does not exist.
    #[error("Invalid block number: {0}")]
    InvalidBlock(u64),
    /// A requested sample or byte position lies outside the available data.
    #[error("Position out of bounds: {0}")]
    OutOfBounds(u64),
    /// The operation stopped because its caller cancelled it.
    #[error("operation cancelled")]
    Cancelled,
    /// A capture query has been scheduled but has not produced data yet.
    #[error("capture query is pending")]
    CaptureQueryPending,
    /// A capture query completed unsuccessfully.
    #[error("capture query failed: {0}")]
    CaptureQuery(#[source] CaptureIndexQueryError),
}

/// Result alias for capture and indexed-signal operations.
pub type Result<T> = std::result::Result<T, Error>;
