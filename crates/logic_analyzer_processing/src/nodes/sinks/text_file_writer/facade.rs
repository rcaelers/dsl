use std::sync::Arc;

use super::super::output_storage::UnavailableOutputStorage;
use super::implementation::TextFileWriter;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::OutputStorage;

/// Platform-neutral construction contract for a text file writer.
pub trait TextFileWriterFactory: Send + Sync {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String>;
}

struct StorageTextFileWriterFactory {
    storage: Arc<dyn OutputStorage>,
}

impl TextFileWriterFactory for StorageTextFileWriterFactory {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(
                TextFileWriter::with_output_storage(Arc::clone(&self.storage)).with_name(name),
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
