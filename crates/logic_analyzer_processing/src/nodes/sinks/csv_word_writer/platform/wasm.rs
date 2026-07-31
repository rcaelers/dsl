use signal_processing::ProcessNode;

use super::super::configuration::CsvWordWriterConfig;
use crate::nodes::sinks::discard_writer::DiscardWordWriter;

pub(crate) fn create_writer(
    name: String,
    _config: CsvWordWriterConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    Ok(Box::new(DiscardWordWriter::new(name)))
}
