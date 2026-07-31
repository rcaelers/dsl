use std::sync::Arc;

use super::super::configuration::CsvWordWriterConfig;
use super::super::facade::CsvWordWriterFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::discard_writer::DiscardWordWriter;

struct WasmCsvWordWriterFactory;

impl CsvWordWriterFactory for WasmCsvWordWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(DiscardWordWriter::new(name)),
            (),
        ))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn CsvWordWriterFactory> {
    Arc::new(WasmCsvWordWriterFactory)
}
