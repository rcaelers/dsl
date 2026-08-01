use std::sync::Arc;

use signal_processing::WorkExecutor;

use super::configuration::DslFileSourceConfig;
use super::platform;
use crate::{CaptureSourceLifecycle, CaptureSourceMetadata, ProcessNodeConstruction};

/// Platform-neutral construction contract for a DSL capture source.
pub trait DslFileSourceFactory: Send + Sync {
    fn lifecycle(&self) -> CaptureSourceLifecycle;
    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata>;
    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

/// Returns the DSL capture-source factory selected for the current platform.
pub fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    platform::source_factory()
}
