use std::path::{Path, PathBuf};

/// Platform-neutral configuration for a DSL capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DslFileSourceConfig {
    path: PathBuf,
    channel_names: Vec<String>,
}

impl DslFileSourceConfig {
    pub fn new(path: impl Into<PathBuf>, channel_names: impl IntoIterator<Item = String>) -> Self {
        Self {
            path: path.into(),
            channel_names: channel_names.into_iter().collect(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn channel_count(&self) -> usize {
        self.channel_names.len()
    }

    pub fn channel_names(&self) -> &[String] {
        &self.channel_names
    }
}
