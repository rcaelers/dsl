use signal_processing::ProcessNode;

use super::super::implementation::TextFileWriter;

pub(crate) fn create_writer(name: String) -> Result<Box<dyn ProcessNode>, String> {
    Ok(Box::new(TextFileWriter::new().with_name(name)))
}
