use std::sync::Arc;

use super::configuration::SigrokFileSourceConfig;
use super::platform;
use crate::{CaptureSourceLifecycle, CaptureSourceMetadata, ProcessNodeConstruction};

/// Platform-neutral construction contract for a Sigrok capture source.
pub trait SigrokFileSourceFactory: Send + Sync {
    fn lifecycle(&self) -> CaptureSourceLifecycle;
    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata>;
    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

/// Returns the Sigrok capture-source factory selected for the current platform.
pub fn source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    platform::source_factory()
}
