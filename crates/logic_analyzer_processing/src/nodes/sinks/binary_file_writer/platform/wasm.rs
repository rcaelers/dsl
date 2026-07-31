use std::sync::Arc;

use super::super::configuration::BinaryFileWriterConfig;
use super::super::facade::BinaryFileWriterFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::discard_writer::DiscardWordWriter;

struct WasmBinaryFileWriterFactory;

impl BinaryFileWriterFactory for WasmBinaryFileWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(DiscardWordWriter::new(name)),
            (),
        ))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn BinaryFileWriterFactory> {
    Arc::new(WasmBinaryFileWriterFactory)
}
