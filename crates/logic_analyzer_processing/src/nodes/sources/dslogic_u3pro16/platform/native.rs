use signal_processing::ProcessNode;
use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::super::source::DsLogicU3Pro16Source;

pub(crate) fn create_source(
    name: String,
    config: LogicCaptureConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    DsLogicU3Pro16Source::open_first(config)
        .map(|source| Box::new(source.with_name(name)) as Box<dyn ProcessNode>)
        .map_err(|error| error.to_string())
}
