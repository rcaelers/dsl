use std::path::PathBuf;

use signal_processing::{CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory};

pub(crate) struct TestCaptureIndexFactory {
    path: PathBuf,
}

impl TestCaptureIndexFactory {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl CaptureIndexFactory for TestCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn open(
        self: Box<Self>,
        _artifact_repository: std::sync::Arc<dyn signal_processing::ArtifactRepository>,
        _work_executor: std::sync::Arc<dyn signal_processing::WorkExecutor>,
        _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
        panic!("a builder contract must not open its deferred viewer index")
    }
}
