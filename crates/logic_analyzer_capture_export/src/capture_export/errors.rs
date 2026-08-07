use std::path::PathBuf;

use thiserror::Error;

use signal_capture_session::CaptureStoreError;

/// Failure produced by a concrete finalized-capture export.
#[derive(Debug, Error)]
pub enum CaptureExportError {
    /// The finalized capture has no durable timeline metadata to encode.
    #[error("capture export requires durable timeline metadata")]
    MissingTimelineMetadata,
    /// The finalized capture contains no samples.
    #[error("cannot export an empty raw capture")]
    EmptyCapture,
    /// Publication would overwrite an existing destination without permission.
    #[error("capture export destination already exists: {0}")]
    DestinationExists(PathBuf),
    /// The destination does not resolve to a writable parent directory.
    #[error("capture export destination has no parent directory: {0}")]
    InvalidDestination(PathBuf),
    /// Cooperative cancellation stopped the export before publication.
    #[error("capture export was cancelled")]
    Cancelled,
    /// Capture metadata, sample extents, or payload layout is inconsistent.
    #[error("capture data is inconsistent: {0}")]
    InconsistentCapture(String),
    /// Reading the finalized capture store failed.
    #[error(transparent)]
    Store(#[from] CaptureStoreError),
    /// Writing or publishing the destination failed.
    #[error("capture export destination I/O failed: {0}")]
    DestinationIo(#[from] std::io::Error),
    /// The concrete archive encoder rejected the output operation.
    #[error("capture archive encoding failed: {0}")]
    Archive(String),
}

impl From<zip::result::ZipError> for CaptureExportError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => Self::DestinationIo(error),
            error => Self::Archive(error.to_string()),
        }
    }
}
