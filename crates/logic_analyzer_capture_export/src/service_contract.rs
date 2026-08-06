//! Stateful capture-export service contract.

use std::path::PathBuf;

use signal_capture_session::CaptureSessionId;

use crate::CaptureExportFormat;

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
    ) -> Result<(), String>;

    /// Returns the most recently published progress while an export is active.
    fn status(&self) -> Option<&CaptureExportStatus>;

    /// Takes the terminal completion or failure exactly once.
    fn take_completion(&mut self) -> Option<Result<CaptureExportCompletion, String>>;

    /// Requests cooperative cancellation of the active export.
    fn request_cancel(&mut self);

    /// Advances the active export without blocking the caller.
    fn poll(&mut self);

    /// Returns whether an export is currently running or cancelling.
    fn is_active(&self) -> bool;

    /// Clears inactive status and completion state.
    fn reset(&mut self);
}
