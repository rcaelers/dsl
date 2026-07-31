use signal_processing::ProcessNode;

use super::super::configuration::DslFileSourceConfig;
use super::super::implementation::DslFileSource;

pub(crate) fn create_source(
    name: String,
    config: DslFileSourceConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    DslFileSource::new(config.path())
        .map(|source| Box::new(source.with_name(name)) as Box<dyn ProcessNode>)
        .map_err(|error| error.to_string())
}
