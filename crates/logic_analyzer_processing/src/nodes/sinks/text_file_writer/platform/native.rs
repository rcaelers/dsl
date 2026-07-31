use std::sync::Arc;

use super::super::facade::TextFileWriterFactory;
use super::super::implementation::TextFileWriter;
use crate::ProcessNodeConstruction;

struct NativeTextFileWriterFactory;

impl TextFileWriterFactory for NativeTextFileWriterFactory {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(TextFileWriter::new().with_name(name)),
            (),
        ))
    }
}

pub(crate) fn writer_factory() -> Arc<dyn TextFileWriterFactory> {
    Arc::new(NativeTextFileWriterFactory)
}
