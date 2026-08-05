use std::path::PathBuf;

use signal_processing::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata,
};

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

    fn metadata(&self) -> signal_processing::Result<CaptureMetadata> {
        Ok(CaptureMetadata {
            total_probes: 1,
            samplerate: "1 MHz".into(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 1,
            total_blocks: 1,
            samples_per_block: 1,
            probe_names: vec!["D0".into()],
            trigger_sample: None,
        })
    }

    fn open(
        self: Box<Self>,
        _artifact_repository: std::sync::Arc<dyn signal_artifacts::ArtifactRepository>,
        _work_executor: std::sync::Arc<dyn signal_runtime::WorkExecutor>,
        _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
        panic!("a builder contract must not open its deferred viewer index")
    }
}
