//! Target-neutral file-dialog requests.

use std::path::PathBuf;

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
