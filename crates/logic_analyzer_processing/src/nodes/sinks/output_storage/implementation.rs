use std::io::Write;
use std::path::Path;

pub trait OutputFile: Write + Send {}

impl<T> OutputFile for T where T: Write + Send {}

/// Upstream graph output whose data is written to an output file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputOrigin {
    /// User-visible upstream graph-node title.
    pub node: String,
    /// User-visible upstream output-socket title.
    pub socket: String,
}

impl OutputOrigin {
    /// Creates explicit producer metadata for one file-writing operation.
    pub fn new(node: impl Into<String>, socket: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            socket: socket.into(),
        }
    }
}

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

    /// Creates or truncates an output file and associates it with its graph origin.
    fn create_for(
        &self,
        path: &Path,
        _origin: &OutputOrigin,
    ) -> std::io::Result<Box<dyn OutputFile>> {
        self.create(path)
    }

    /// Opens an output file for appending.
    ///
    /// # Parameters
    /// - `path`: Destination file path.
    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    /// Opens an output file for appending and associates it with its graph origin.
    fn append_for(
        &self,
        path: &Path,
        _origin: &OutputOrigin,
    ) -> std::io::Result<Box<dyn OutputFile>> {
        self.append(path)
    }

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
