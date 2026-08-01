use std::sync::Arc;

use super::super::output_storage::UnavailableOutputStorage;
use super::configuration::BinaryFileWriterConfig;
use super::implementation::BinaryFileWriter;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::OutputStorage;

/// Platform-neutral construction contract for a binary file writer.
pub trait BinaryFileWriterFactory: Send + Sync {
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String>;
}

struct StorageBinaryFileWriterFactory {
    storage: Arc<dyn OutputStorage>,
}

impl BinaryFileWriterFactory for StorageBinaryFileWriterFactory {
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        let mut writer = BinaryFileWriter::with_output_storage(Arc::clone(&self.storage))
            .with_width(config.width())
            .with_index_csv(config.index_csv())
            .with_name(name);
        if let Some(filename) = config.static_filename() {
            writer = writer.with_filename(filename);
        }
        Ok(ProcessNodeConstruction::new(Box::new(writer), ()))
    }
}

/// Builds binary-writer nodes using an injected destination capability.
pub fn writer_factory(storage: Arc<dyn OutputStorage>) -> Arc<dyn BinaryFileWriterFactory> {
    Arc::new(StorageBinaryFileWriterFactory { storage })
}

/// Returns a factory whose writer reports the absent destination when used.
pub fn unavailable_writer_factory() -> Arc<dyn BinaryFileWriterFactory> {
    writer_factory(Arc::new(UnavailableOutputStorage))
}
