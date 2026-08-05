use std::path::PathBuf;

use signal_capture_session::CaptureSessionId;

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

/// Capture-file encoding offered by the UI export workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureExportFormat {
    Portable,
}

/// Presentation and filename data for an export format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureExportDescriptor {
    pub label: &'static str,
    pub extension: &'static str,
    pub dialog_title: &'static str,
    pub default_file_name: &'static str,
}

impl CaptureExportFormat {
    /// Returns user-facing dialog and filename metadata for this format.
    pub const fn descriptor(self) -> CaptureExportDescriptor {
        match self {
            Self::Portable => CaptureExportDescriptor {
                label: "PulseView capture",
                extension: "sr",
                dialog_title: "Save Capture Data",
                default_file_name: "capture.sr",
            },
        }
    }
}

/// Stateful service used by the UI to run and observe one capture export.
///
/// Call [`Self::start`], repeatedly [`Self::poll`] from the UI loop, then take a
/// terminal result with [`Self::take_completion`]. A service accepts one active
/// export at a time.
pub trait CaptureExportService {
    /// Starts exporting a finalized capture session to a destination file.
    ///
    /// # Parameters
    /// - `session_id`: Finalized capture session to export.
    /// - `format`: Requested output encoding.
    /// - `destination`: Host-selected output path. Existing-file handling belongs to the service.
    fn start(
        &mut self,
        session_id: CaptureSessionId,
        format: CaptureExportFormat,
        destination: PathBuf,
    ) -> Result<(), String>;

    /// Returns the most recently published progress while an export is active.
    fn status(&self) -> Option<&CaptureExportStatus>;

    /// Takes the terminal completion or failure exactly once.
    ///
    /// Returns `None` until the active export finishes or fails.
    fn take_completion(&mut self) -> Option<Result<CaptureExportCompletion, String>>;

    /// Requests cooperative cancellation of the active export.
    ///
    /// The service reports cancellation through [`Self::take_completion`] after polling advances it.
    fn request_cancel(&mut self);

    /// Advances the active export without blocking the UI event loop.
    fn poll(&mut self);

    /// Returns whether an export is currently running or cancelling.
    fn is_active(&self) -> bool;

    /// Clears inactive status and completion state so another export may start.
    fn reset(&mut self);
}
