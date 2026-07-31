use std::sync::Arc;

use super::super::configuration::BinaryFileWriterConfig;
use super::super::facade::BinaryFileWriterFactory;
use super::super::implementation::BinaryFileWriter;
use crate::ProcessNodeConstruction;

struct NativeBinaryFileWriterFactory;

impl BinaryFileWriterFactory for NativeBinaryFileWriterFactory {
    fn create(
        &self,
        name: &str,
        config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        let mut writer = BinaryFileWriter::new()
            .with_width(config.width())
            .with_index_csv(config.index_csv())
            .with_name(name);
        if let Some(filename) = config.static_filename() {
            writer = writer.with_filename(filename);
        }
        Ok(ProcessNodeConstruction::new(Box::new(writer), ()))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn BinaryFileWriterFactory> {
    Arc::new(NativeBinaryFileWriterFactory)
}
