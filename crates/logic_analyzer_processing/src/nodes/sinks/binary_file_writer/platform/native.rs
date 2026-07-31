use signal_processing::ProcessNode;

use super::super::configuration::BinaryFileWriterConfig;
use super::super::implementation::BinaryFileWriter;

pub(crate) fn create_writer(
    name: String,
    config: BinaryFileWriterConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    let mut writer = BinaryFileWriter::new()
        .with_width(config.width())
        .with_index_csv(config.index_csv())
        .with_name(name);
    if let Some(filename) = config.static_filename() {
        writer = writer.with_filename(filename);
    }
    Ok(Box::new(writer))
}
