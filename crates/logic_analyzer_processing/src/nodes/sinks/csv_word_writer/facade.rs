use std::sync::Arc;

use super::configuration::CsvWordWriterConfig;
use super::platform;
use crate::ProcessNodeConstruction;

/// Platform-neutral construction contract for a CSV word writer.
pub trait CsvWordWriterFactory: Send + Sync {
    fn create(
        &self,
        name: &str,
        config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String>;
}

/// Returns the CSV-writer factory selected for the current platform.
pub fn writer_factory() -> Arc<dyn CsvWordWriterFactory> {
    platform::writer_factory()
}
