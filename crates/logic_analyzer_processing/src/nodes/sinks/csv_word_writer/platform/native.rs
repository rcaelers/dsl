use std::sync::Arc;

use super::super::configuration::CsvWordWriterConfig;
use super::super::facade::CsvWordWriterFactory;
use super::super::implementation::CsvWordWriter;
use crate::ProcessNodeConstruction;

struct NativeCsvWordWriterFactory;

impl CsvWordWriterFactory for NativeCsvWordWriterFactory {
    fn create(
        &self,
        name: &str,
        config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        let mut writer = CsvWordWriter::new()
            .with_value_format(config.value_format())
            .with_header(config.header().map(str::to_owned))
            .with_name(name);
        if let Some(filename) = config.static_filename() {
            writer = writer.with_filename(filename);
        }
        Ok(ProcessNodeConstruction::new(Box::new(writer), ()))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn CsvWordWriterFactory> {
    Arc::new(NativeCsvWordWriterFactory)
}
