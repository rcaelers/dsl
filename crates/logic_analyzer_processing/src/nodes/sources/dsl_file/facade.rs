use signal_processing::ProcessNode;

use super::configuration::DslFileSourceConfig;
use super::platform;

/// Creates the processing source selected for the current platform.
pub fn create_source(
    name: impl Into<String>,
    config: DslFileSourceConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    platform::create_source(name.into(), config)
}
