use std::io::Write;
use std::path::Path;

pub trait OutputFile: Write + Send {}

impl<T> OutputFile for T where T: Write + Send {}

/// Host-provided destination capability used by file-writing processing nodes.
pub trait OutputStorage: Send + Sync {
    /// Ensures that the parent directories for an output path exist.
    ///
    /// # Parameters
    /// - `path`: Output file path whose parent directories are required.
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()>;

    /// Creates or truncates an output file for writing.
    ///
    /// # Parameters
    /// - `path`: Destination file path.
    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    /// Opens an output file for appending.
    ///
    /// # Parameters
    /// - `path`: Destination file path.
    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    /// Returns whether an output path currently exists.
    ///
    /// # Parameters
    /// - `path`: Destination file path to inspect.
    fn exists(&self, path: &Path) -> bool;
}

pub(crate) struct UnavailableOutputStorage;

impl OutputStorage for UnavailableOutputStorage {
    fn create_parent_dirs(&self, _path: &Path) -> std::io::Result<()> {
        Err(unavailable())
    }

    fn create(&self, _path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        Err(unavailable())
    }

    fn append(&self, _path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        Err(unavailable())
    }

    fn exists(&self, _path: &Path) -> bool {
        false
    }
}

fn unavailable() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no output destination capability was supplied",
    )
}
