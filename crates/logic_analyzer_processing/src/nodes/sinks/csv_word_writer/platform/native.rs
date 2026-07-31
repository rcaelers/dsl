use signal_processing::ProcessNode;

use super::super::configuration::CsvWordWriterConfig;
use super::super::implementation::CsvWordWriter;

pub(crate) fn create_writer(
    name: String,
    config: CsvWordWriterConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    let mut writer = CsvWordWriter::new()
        .with_value_format(config.value_format())
        .with_header(config.header().map(str::to_owned))
        .with_name(name);
    if let Some(filename) = config.static_filename() {
        writer = writer.with_filename(filename);
    }
    Ok(Box::new(writer))
}
