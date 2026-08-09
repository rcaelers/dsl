use std::sync::Arc;

use signal_runtime::ProcessNodeConstruction;

use super::super::output_storage::UnavailableOutputStorage;
use super::configuration::BinaryFileWriterConfig;
use super::implementation::BinaryFileWriter;
use crate::{OutputOrigin, OutputStorage, WriterConstructionError};

/// Platform-neutral construction contract for a binary file writer.
pub trait BinaryFileWriterFactory: Send + Sync {
    /// Returns the destination-storage capability used by the writer.
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    /// - `config`: Input consumed by this operation.
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError>;
}

struct StorageBinaryFileWriterFactory {
    storage: Arc<dyn OutputStorage>,
}

impl BinaryFileWriterFactory for StorageBinaryFileWriterFactory {
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError> {
        let mut writer = BinaryFileWriter::with_output_storage(Arc::clone(&self.storage))
            .with_width(config.width())
            .with_index_csv(config.index_csv())
            .with_name(name)
            .with_output_origin(output_origin);
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
