use std::sync::Arc;

use super::super::facade::TextFileWriterFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sinks::discard_writer::DiscardTextWriter;

struct WasmTextFileWriterFactory;

impl TextFileWriterFactory for WasmTextFileWriterFactory {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(DiscardTextWriter::new(name)),
            (),
        ))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn TextFileWriterFactory> {
    Arc::new(WasmTextFileWriterFactory)
}
