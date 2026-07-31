use std::sync::Arc;

use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::platform;
use crate::{CaptureSourceLifecycle, CaptureSourceMetadata, ProcessNodeConstruction};

/// Platform-neutral construction contract for a U3Pro16 capture source.
pub trait DsLogicU3Pro16SourceFactory: Send + Sync {
    fn lifecycle(&self) -> CaptureSourceLifecycle;
    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata>;
    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

/// Returns the U3Pro16 source factory selected for the current platform.
pub fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    platform::source_factory()
}
