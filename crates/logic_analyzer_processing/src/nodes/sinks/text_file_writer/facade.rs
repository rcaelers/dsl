use std::sync::Arc;

use super::platform;
use crate::ProcessNodeConstruction;

/// Platform-neutral construction contract for a text file writer.
pub trait TextFileWriterFactory: Send + Sync {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String>;
}

/// Returns the text-writer factory selected for the current platform.
pub fn writer_factory() -> Arc<dyn TextFileWriterFactory> {
    platform::writer_factory()
}
