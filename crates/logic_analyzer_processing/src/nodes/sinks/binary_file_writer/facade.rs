use std::sync::Arc;

use super::configuration::BinaryFileWriterConfig;
use super::platform;
use crate::ProcessNodeConstruction;

/// Platform-neutral construction contract for a binary file writer.
pub trait BinaryFileWriterFactory: Send + Sync {
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String>;
}

/// Returns the binary-writer factory selected for the current platform.
pub fn writer_factory() -> Arc<dyn BinaryFileWriterFactory> {
    platform::writer_factory()
}
