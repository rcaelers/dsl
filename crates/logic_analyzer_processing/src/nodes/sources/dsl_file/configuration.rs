use std::path::{Path, PathBuf};

/// Platform-neutral configuration for a DSL capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DslFileSourceConfig {
    path: PathBuf,
    channel_count: usize,
}

impl DslFileSourceConfig {
    pub fn new(path: impl Into<PathBuf>, channel_count: usize) -> Self {
        Self {
            path: path.into(),
            channel_count,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn channel_count(&self) -> usize {
        self.channel_count
    }
}
