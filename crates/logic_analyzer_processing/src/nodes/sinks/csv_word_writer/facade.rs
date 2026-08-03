use std::sync::Arc;

use super::super::output_storage::UnavailableOutputStorage;
use super::configuration::CsvWordWriterConfig;
use super::implementation::CsvWordWriter;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::OutputStorage;

/// Platform-neutral construction contract for a CSV word writer.
pub trait CsvWordWriterFactory: Send + Sync {
    /// Returns the destination-storage capability used by the writer.
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    /// - `config`: Input consumed by this operation.
    fn create(
        &self,
        name: &str,
        config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String>;
}

struct StorageCsvWordWriterFactory {
    storage: Arc<dyn OutputStorage>,
}

impl CsvWordWriterFactory for StorageCsvWordWriterFactory {
    fn create(
        &self,
        name: &str,
        config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        let mut writer = CsvWordWriter::with_output_storage(Arc::clone(&self.storage))
            .with_value_format(config.value_format())
            .with_header(config.header().map(str::to_owned))
            .with_name(name);
        if let Some(filename) = config.static_filename() {
            writer = writer.with_filename(filename);
        }
        Ok(ProcessNodeConstruction::new(Box::new(writer), ()))
    }
}

/// Builds CSV-writer nodes using an injected destination capability.
pub fn writer_factory(storage: Arc<dyn OutputStorage>) -> Arc<dyn CsvWordWriterFactory> {
    Arc::new(StorageCsvWordWriterFactory { storage })
}

/// Returns a factory whose writer reports the absent destination when used.
pub fn unavailable_writer_factory() -> Arc<dyn CsvWordWriterFactory> {
    writer_factory(Arc::new(UnavailableOutputStorage))
}
