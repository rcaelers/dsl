use std::path::{Path, PathBuf};

/// Platform-neutral configuration for a DSL capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DslFileSourceConfig {
    path: PathBuf,
    channel_names: Vec<String>,
}

impl DslFileSourceConfig {
    /// Creates portable configuration for a DSL capture-file source.
    ///
    /// # Parameters
    /// - `path`: Path or host reference selected for the capture file.
    /// - `channel_names`: Optional channel names known before the file is opened.
    pub fn new(path: impl Into<PathBuf>, channel_names: impl IntoIterator<Item = String>) -> Self {
        Self {
            path: path.into(),
            channel_names: channel_names.into_iter().collect(),
        }
    }

    /// Returns the configured path or host reference.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the number of channel names supplied in this configuration.
    pub const fn channel_count(&self) -> usize {
        self.channel_names.len()
    }

    /// Returns channel names known before opening the source.
    pub fn channel_names(&self) -> &[String] {
        &self.channel_names
    }
}
