use std::sync::Arc;

use signal_runtime::ProcessNodeConstruction;

use super::super::output_storage::UnavailableOutputStorage;
use super::writer::TextFileWriter;
use crate::{OutputOrigin, OutputStorage, WriterConstructionError};

/// Platform-neutral construction contract for a text file writer.
pub trait TextFileWriterFactory: Send + Sync {
    /// Returns the destination-storage capability used by the writer.
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    fn create(
        &self,
        name: &str,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError>;
}

struct StorageTextFileWriterFactory {
    storage: Arc<dyn OutputStorage>,
}

impl TextFileWriterFactory for StorageTextFileWriterFactory {
    fn create(
        &self,
        name: &str,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError> {
        Ok(ProcessNodeConstruction::new(
            Box::new(
                TextFileWriter::with_output_storage(Arc::clone(&self.storage))
                    .with_name(name)
                    .with_output_origin(output_origin),
            ),
            (),
        ))
    }
}

/// Builds text-writer nodes using an injected destination capability.
pub fn writer_factory(storage: Arc<dyn OutputStorage>) -> Arc<dyn TextFileWriterFactory> {
    Arc::new(StorageTextFileWriterFactory { storage })
}

/// Returns a factory whose writer reports the absent destination when used.
pub fn unavailable_writer_factory() -> Arc<dyn TextFileWriterFactory> {
    writer_factory(Arc::new(UnavailableOutputStorage))
}
