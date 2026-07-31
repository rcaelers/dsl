use signal_processing::ProcessNode;

use crate::nodes::sinks::discard_writer::DiscardTextWriter;

pub(crate) fn create_writer(name: String) -> Result<Box<dyn ProcessNode>, String> {
    Ok(Box::new(DiscardTextWriter::new(name)))
}
