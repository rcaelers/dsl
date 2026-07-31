use std::path::{Path, PathBuf};

/// Platform-neutral configuration for a Sigrok capture source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokFileSourceConfig {
    path: PathBuf,
    channel_names: Vec<String>,
    demo_data: bool,
}

impl SigrokFileSourceConfig {
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn channel_count(&self) -> usize {
        self.channel_names.len()
    }

    pub fn channel_names(&self) -> &[String] {
        &self.channel_names
    }

    pub const fn demo_data(&self) -> bool {
        self.demo_data
    }
}
