//! Stateful capture-export service contract.

use std::path::PathBuf;

use thiserror::Error;

use signal_capture_session::{CaptureSessionId, CaptureStoreError};

use crate::capture_export::{CaptureExportError, CaptureExportFormat};

/// Failure reported by the stateful capture-export application service.
#[derive(Debug, Error)]
pub enum CaptureExportServiceError {
    /// This host has no capture-export implementation.
    #[error("capture export is unavailable on this host")]
    Unavailable,
    /// Another export still owns the service worker.
    #[error("a capture export is already active")]
    AlreadyActive,
    /// The requested finalized capture could not be opened.
    #[error("could not open the capture for export: {0}")]
    Capture(#[source] CaptureStoreError),
    /// The asynchronous export worker could not be started.
    #[error("capture export executor failed: {0}")]
    Executor(#[source] std::io::Error),
    /// The asynchronous export worker stopped without publishing a result.
    #[error("capture export worker stopped without a result")]
    WorkerStopped,
    /// Cooperative cancellation stopped the active export.
    #[error("capture export was cancelled")]
    Cancelled,
    /// The exporter could not encode or publish the capture.
    #[error(transparent)]
    Export(CaptureExportError),
}

impl From<CaptureExportError> for CaptureExportServiceError {
    fn from(error: CaptureExportError) -> Self {
        match error {
            CaptureExportError::Cancelled => Self::Cancelled,
            error => Self::Export(error),
        }
    }
}

/// Progress reported while a capture export is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureExportStatus {
    pub format_label: String,
    pub destination: PathBuf,
    pub samples_written: u64,
    pub total_samples: u64,
    pub cancelling: bool,
}

/// Successful terminal result of a capture export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureExportCompletion {
    pub destination: PathBuf,
    pub warnings: Vec<String>,
}

/// Stateful service used by an application to run and observe one capture export.
pub trait CaptureExportService {
    /// Starts exporting a finalized capture session to a destination file.
    fn start(
        &mut self,
        session_id: CaptureSessionId,
        format: CaptureExportFormat,
        destination: PathBuf,
    ) -> Result<(), CaptureExportServiceError>;

    /// Returns the most recently published progress while an export is active.
    fn status(&self) -> Option<&CaptureExportStatus>;

    /// Takes the terminal completion or failure exactly once.
    fn take_completion(
        &mut self,
    ) -> Option<Result<CaptureExportCompletion, CaptureExportServiceError>>;

    /// Requests cooperative cancellation of the active export.
    fn request_cancel(&mut self);

    /// Advances the active export without blocking the caller.
    fn poll(&mut self);

    /// Returns whether an export is currently running or cancelling.
    fn is_active(&self) -> bool;

    /// Clears inactive status and completion state.
    fn reset(&mut self);
}
