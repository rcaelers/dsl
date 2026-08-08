//! Target-neutral file-dialog requests.

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

/// Failure produced while a host selects or imports a file.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FilePickerError {
    /// The host did not supply either a path or file contents for a dropped file.
    #[error("the host did not provide contents for '{name}'")]
    ContentsUnavailable {
        /// Display name supplied by the host.
        name: String,
    },
    /// The selected file could not be read through the host mechanism.
    #[error("could not read '{name}': {message}")]
    Read {
        /// Display name supplied by the host.
        name: String,
        /// Host-adapter diagnostic.
        message: String,
    },
    /// The selected file exceeds the import capacity advertised by the host.
    #[error("'{name}' is too large for this host ({max_bytes} byte limit)")]
    TooLarge {
        /// Display name supplied by the host.
        name: String,
        /// Maximum supported imported-file size.
        max_bytes: u64,
    },
    /// The host could read the file but could not retain its imported representation.
    #[error("could not import '{name}': {message}")]
    Import {
        /// Display name supplied by the host.
        name: String,
        /// Import-adapter diagnostic.
        message: String,
    },
}

#[cfg(test)]
mod file_dialog_tests {
    use super::FilePickerError;

    #[test]
    fn picker_failures_keep_host_mechanism_categories() {
        assert!(matches!(
            FilePickerError::ContentsUnavailable {
                name: "capture.sr".to_owned()
            },
            FilePickerError::ContentsUnavailable { .. }
        ));
        assert!(matches!(
            FilePickerError::TooLarge {
                name: "capture.sr".to_owned(),
                max_bytes: 1024,
            },
            FilePickerError::TooLarge {
                max_bytes: 1024,
                ..
            }
        ));
    }
}

/// One named group of extensions offered by a host file dialog.
pub struct FileDialogFilter {
    /// User-facing filter name.
    pub name: String,
    /// Accepted extensions without leading dots.
    pub extensions: Vec<String>,
}

/// Request for selecting an existing host file.
pub struct FileOpenDialog {
    /// Dialog title.
    pub title: String,
    /// File-type filters offered to the user.
    pub filters: Vec<FileDialogFilter>,
    /// Initial directory when the host supports one.
    pub initial_directory: Option<PathBuf>,
}

/// Request for choosing a host file destination.
pub struct FileSaveDialog {
    /// Dialog title.
    pub title: String,
    /// Suggested destination file name.
    pub default_file_name: String,
    /// File-type filters offered to the user.
    pub filters: Vec<FileDialogFilter>,
    /// Initial directory when the host supports one.
    pub initial_directory: Option<PathBuf>,
}

/// Request for an asynchronous host file picker.
pub struct FilePickerRequest<'a> {
    /// Caller-chosen identifier used to retrieve asynchronous completion.
    pub request_id: u64,
    /// User-facing dialog title.
    pub title: &'a str,
    /// File types selectable in the dialog.
    pub filters: &'a [FileDialogFilter],
    /// Whether the picker selects a destination instead of an existing file.
    pub save: bool,
}

/// Progress for an asynchronous file selection or import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilePickerProgress {
    /// Bytes selected, read, or imported so far.
    pub completed_bytes: u64,
    /// Total bytes when the host can determine the size.
    pub total_bytes: Option<u64>,
}

/// Host file data delivered by drag-and-drop.
pub struct DroppedFileData {
    /// Host-provided display name.
    pub name: String,
    /// Native path when the host exposes one.
    pub path: Option<PathBuf>,
    /// File content when the host provides bytes instead of a path.
    pub bytes: Option<Arc<[u8]>>,
}

/// Opaque reference returned by a host file picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileReference(String);

impl FileReference {
    /// Returns the opaque reference as a string for persistence in a consumer model.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for FileReference {
    fn from(reference: String) -> Self {
        Self(reference)
    }
}

/// Host capability for asynchronous file selection and import.
pub trait FilePickerService: Send {
    /// Reports whether the host supports the requested picker mode.
    fn available(&self, save: bool) -> bool;

    /// Starts a selection and optionally returns an immediate reference.
    fn pick(&mut self, request: FilePickerRequest<'_>) -> Option<FileReference>;

    /// Returns a completed asynchronous selection for one request.
    fn take_picked(&mut self, request_id: u64) -> Option<Result<FileReference, FilePickerError>>;

    /// Returns current asynchronous selection or import progress.
    fn progress(&self, request_id: u64) -> Option<FilePickerProgress>;

    /// Cancels a pending selection or import when supported.
    fn cancel(&mut self, request_id: u64) -> bool;

    /// Imports data supplied by the host's drag-and-drop mechanism.
    fn import_dropped(&mut self, file: DroppedFileData) -> Result<FileReference, FilePickerError>;

    /// Installs the wake-up callback used by asynchronous operations.
    fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>);
}
