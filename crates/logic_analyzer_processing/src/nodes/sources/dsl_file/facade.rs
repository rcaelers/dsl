use std::sync::Arc;

use super::configuration::DslFileSourceConfig;
use super::platform;
use crate::ProcessNodeConstruction;

/// Platform-neutral construction contract for a DSL capture source.
pub trait DslFileSourceFactory: Send + Sync {
    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
    ) -> Result<ProcessNodeConstruction, String>;
}

/// Returns the DSL capture-source factory selected for the current platform.
pub fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    platform::source_factory()
}
