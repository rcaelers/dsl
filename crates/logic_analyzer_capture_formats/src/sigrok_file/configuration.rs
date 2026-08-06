use std::path::{Path, PathBuf};

/// Platform-neutral configuration for a Sigrok capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokFileSourceConfig {
    path: PathBuf,
    channel_names: Vec<String>,
    demo_data: bool,
}

impl SigrokFileSourceConfig {
    /// Creates portable configuration for a Sigrok session-file source.
    ///
    /// # Parameters
    /// - `path`: Path or host reference selected for the session file.
    /// - `channel_names`: Optional channel names known before the file is opened.
    /// - `demo_data`: Whether the explicit portable synthetic-demo mode is selected.
    pub fn new(
        path: impl Into<PathBuf>,
        channel_names: impl IntoIterator<Item = String>,
        demo_data: bool,
    ) -> Self {
        Self {
            path: path.into(),
            channel_names: channel_names.into_iter().collect(),
            demo_data,
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

    /// Returns whether this configuration selects explicit synthetic demo data.
    pub const fn demo_data(&self) -> bool {
        self.demo_data
    }
}
