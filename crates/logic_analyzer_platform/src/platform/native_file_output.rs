use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Ensures that a native output path's parent directories exist.
pub fn native_create_parent_directories(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Creates or truncates a native file for byte output.
pub fn native_create_file(path: &Path) -> io::Result<File> {
    File::create(path)
}

/// Opens a native file for appended byte output.
pub fn native_append_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Returns whether a native filesystem path exists.
pub fn native_path_exists(path: &Path) -> bool {
    path.exists()
}
